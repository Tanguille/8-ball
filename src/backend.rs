use std::time::Duration;

use crate::config::BackendKind;

pub mod embed;
pub mod logprob;
pub mod rerank;
pub mod systemone;

/// One probability per label of the question, in `Kind::labels()` order, summing to 1.
pub type Probs = Vec<f64>;

pub struct Endpoint {
    pub name: String,
    url: String,
    kind: Option<BackendKind>,
    model: Option<String>,
    api_key: Option<String>,
    extra_body: Option<toml::Table>,
    max_state_chars: Option<usize>,
    client: reqwest::Client,
    concurrency: Option<usize>,
    info: tokio::sync::OnceCell<(Info, tokio::sync::Semaphore)>,
}

// Decision models answer in under 1 s; 20 s only absorbs a cold model load.
pub const DECISION_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct BackendError(pub String);

impl From<reqwest::Error> for BackendError {
    fn from(error: reqwest::Error) -> Self {
        Self(error.to_string())
    }
}

impl Endpoint {
    pub fn new(
        name: &str,
        config: &crate::config::EndpointConfig,
        api_key: Option<String>,
    ) -> Result<Self, BackendError> {
        if config.concurrency == Some(0) {
            return Err(BackendError(format!(
                "endpoint {name}: concurrency must be positive"
            )));
        }

        Ok(Self {
            name: name.into(),
            url: config.url.clone(),
            kind: config.kind,
            model: config.model.clone(),
            api_key,
            extra_body: config.extra_body.clone(),
            max_state_chars: config.max_state_chars,
            client: reqwest::Client::new(),
            concurrency: config.concurrency,
            info: tokio::sync::OnceCell::new(),
        })
    }

    pub async fn info(&self) -> Result<&Info, BackendError> {
        Ok(&self.detected().await?.0)
    }

    async fn detected(&self) -> Result<&(Info, tokio::sync::Semaphore), BackendError> {
        self.info
            .get_or_try_init(|| async {
                let info = self.detect().await?;
                let permits = tokio::sync::Semaphore::new(info.concurrency);

                Ok((info, permits))
            })
            .await
    }

    async fn get_info_json(&self, path: &str) -> Result<serde_json::Value, BackendError> {
        let request = self
            .client
            .get(format!("{}{path}", self.url))
            .timeout(Duration::from_secs(5));

        Ok(authed(request, self.api_key.as_deref())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    async fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, BackendError> {
        let permit = self
            .detected()
            .await?
            .1
            .acquire()
            .await
            .map_err(|e| BackendError(e.to_string()))?;
        let request = self.client.post(format!("{}{path}", self.url)).json(body);
        let response = authed(request, self.api_key.as_deref()).send().await?;
        let status = response.status();
        let text = response.text().await?;
        drop(permit);
        if !status.is_success() {
            let text: String = text.chars().take(500).collect();
            let hint = if status == reqwest::StatusCode::NOT_IMPLEMENTED {
                "; not a decision model"
            } else {
                ""
            };

            return Err(BackendError(format!("HTTP {status}: {text}{hint}")));
        }

        serde_json::from_str(&text).map_err(|e| BackendError(e.to_string()))
    }

    async fn detect(&self) -> Result<Info, BackendError> {
        let properties = self
            .get_info_json("/props")
            .await
            .unwrap_or(serde_json::Value::Null);
        let mut models = serde_json::Value::Null;
        let kind = if let Some(kind) = self.kind {
            kind
        } else {
            models = self.get_info_json("/v1/models").await.map_err(|e| {
                BackendError(format!(
                    "cannot detect kind of {}: {e}; set kind in config",
                    self.name
                ))
            })?;
            let data = models["data"].as_array();
            if data.is_some_and(|data| {
                data.iter().any(|model| {
                    model["architecture"]["output_modalities"]
                        .as_array()
                        .is_some_and(|modes| modes.iter().any(|mode| mode == "decisions"))
                })
            }) {
                BackendKind::Systemone
            } else if self.model.is_some()
                && data.is_some_and(|data| {
                    data.iter().any(|model| model.get("architecture").is_some())
                })
            {
                BackendKind::Logprob
            } else {
                return Err(BackendError(format!(
                    "cannot detect kind of {}: no decisions model and no `model` set; set kind in config",
                    self.name
                )));
            }
        };
        let fingerprint = properties["model_path"].as_str().map_or_else(
            || {
                self.model
                    .as_deref()
                    .or_else(|| models["data"][0]["id"].as_str())
                    .unwrap_or(&self.url)
                    .into()
            },
            |path| {
                format!(
                    "{path}@{}",
                    properties["build_info"].as_str().unwrap_or_default()
                )
            },
        );
        let concurrency = self
            .concurrency
            .or_else(|| {
                properties["total_slots"]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
            })
            .unwrap_or(2);

        Ok(Info {
            kind,
            fingerprint,
            concurrency,
        })
    }

    /// Fit only after `info()` when the endpoint kind is auto-detected.
    #[must_use]
    pub fn fitted(&self, state: &str) -> String {
        let kind = self
            .info
            .get()
            .map_or(self.kind, |(info, _)| Some(info.kind));
        let maximum = self.max_state_chars.or(match kind {
            Some(BackendKind::Systemone | BackendKind::Rerank) => Some(2000),
            _ => None,
        });

        fit(state, maximum)
    }

    /// Ask all questions about an already-fitted state.
    pub async fn ask(
        &self,
        state: &str,
        questions: &[crate::question::Question],
    ) -> Result<Vec<Probs>, BackendError> {
        tokio::time::timeout(DECISION_TIMEOUT, async {
            let info = self.info().await?;

            match info.kind {
                BackendKind::Systemone => systemone::ask(self, state, questions).await,
                BackendKind::Logprob => logprob::ask(self, state, questions).await,
                BackendKind::Rerank => rerank::ask(self, state, questions).await,
            }
        })
        .await
        .map_err(|_| BackendError("timed out after 20s".into()))?
    }
}

/// Adds the bearer header only when a key is configured.
pub fn authed(request: reqwest::RequestBuilder, key: Option<&str>) -> reqwest::RequestBuilder {
    match key {
        Some(key) => request.bearer_auth(key),
        None => request,
    }
}

/// Keep the first and last maximum/2 characters (chars, not bytes), joined by "\n…\n".
#[must_use]
pub fn fit(state: &str, maximum: Option<usize>) -> String {
    let Some(maximum) = maximum else {
        return state.into();
    };
    let count = state.chars().count();
    if count <= maximum {
        return state.into();
    }
    let head: String = state.chars().take(maximum / 2).collect();
    let tail: String = state.chars().skip(count - maximum / 2).collect();

    format!("{head}\n…\n{tail}")
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Info {
    pub kind: BackendKind,
    pub fingerprint: String,
    pub concurrency: usize,
}
