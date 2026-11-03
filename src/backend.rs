use crate::{
    config::{BackendKind, EndpointConfig},
    question::Question,
};
use std::time::Duration;

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
        cfg: &EndpointConfig,
        api_key: Option<String>,
    ) -> Result<Self, BackendError> {
        if cfg.concurrency == Some(0) {
            return Err(BackendError(format!(
                "endpoint {name}: concurrency must be positive"
            )));
        }
        Ok(Self {
            name: name.into(),
            url: cfg.url.clone(),
            kind: cfg.kind,
            model: cfg.model.clone(),
            api_key,
            extra_body: cfg.extra_body.clone(),
            max_state_chars: cfg.max_state_chars,
            client: reqwest::Client::new(),
            concurrency: cfg.concurrency,
            info: tokio::sync::OnceCell::new(),
        })
    }

    pub async fn info(&self) -> Result<&Info, BackendError> {
        let (info, _) = self
            .info
            .get_or_try_init(|| async {
                let info = self.detect().await?;
                let permits = tokio::sync::Semaphore::new(info.concurrency);
                Ok::<_, BackendError>((info, permits))
            })
            .await?;
        Ok(info)
    }

    async fn get_info_json(&self, path: &str) -> Result<serde_json::Value, BackendError> {
        let mut request = self
            .client
            .get(format!("{}{path}", self.url))
            .timeout(Duration::from_secs(5));
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        Ok(request.send().await?.error_for_status()?.json().await?)
    }

    async fn post_json(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, BackendError> {
        self.info().await?;
        let permit = self
            .info
            .get()
            .expect("info initialized")
            .1
            .acquire()
            .await
            .map_err(|e| BackendError(e.to_string()))?;
        let mut request = self.client.post(format!("{}{path}", self.url)).json(body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await?;
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
        let props = self
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
                data.iter().any(|m| {
                    m["architecture"]["output_modalities"]
                        .as_array()
                        .is_some_and(|modes| modes.iter().any(|mode| mode == "decisions"))
                })
            }) {
                BackendKind::Systemone
            } else if self.model.is_some()
                && data.is_some_and(|data| data.iter().any(|m| m.get("architecture").is_some()))
            {
                BackendKind::Logprob
            } else {
                return Err(BackendError(format!(
                    "cannot detect kind of {}: no decisions model and no `model` set; set kind in config",
                    self.name
                )));
            }
        };
        let fingerprint = props["model_path"].as_str().map_or_else(
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
                    props["build_info"].as_str().unwrap_or_default()
                )
            },
        );
        let concurrency = self
            .concurrency
            .or_else(|| {
                props["total_slots"]
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
            })
            .unwrap_or(2);
        Ok(Info {
            kind,
            fingerprint,
            concurrency,
        })
    }

    /// Trim the state, then ask all questions. Returns (probs per question, state actually sent).
    pub async fn ask(
        &self,
        state: &str,
        qs: &[Question],
    ) -> Result<(Vec<Probs>, String), BackendError> {
        tokio::time::timeout(DECISION_TIMEOUT, async {
            let info = self.info().await?;
            let max = self.max_state_chars.or(match info.kind {
                BackendKind::Systemone | BackendKind::Rerank => Some(2000),
                BackendKind::Logprob => None,
            });
            let state = fit(state, max);
            let probs = match info.kind {
                BackendKind::Systemone => systemone::ask(self, &state, qs).await?,
                BackendKind::Logprob => logprob::ask(self, &state, qs).await?,
                BackendKind::Rerank => rerank::ask(self, &state, qs).await?,
            };
            Ok((probs, state))
        })
        .await
        .map_err(|_| BackendError("timed out after 20s".into()))?
    }
}

/// Keep the first and last max/2 characters (chars, not bytes), joined by "\n…\n".
#[must_use]
pub fn fit(state: &str, max: Option<usize>) -> String {
    let Some(max) = max else {
        return state.into();
    };
    let count = state.chars().count();
    if count <= max {
        return state.into();
    }
    let head: String = state.chars().take(max / 2).collect();
    let tail: String = state.chars().skip(count - max / 2).collect();
    format!("{head}\n…\n{tail}")
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Info {
    pub kind: BackendKind,
    pub fingerprint: String,
    pub concurrency: usize,
}
