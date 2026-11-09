use std::{sync::Arc, time::Duration};

use reqwest::{Client, Error, RequestBuilder};
use serde_json::Value;
use tokio::{
    sync::{Mutex, Semaphore},
    time::Instant,
};

use domain::config::{BackendKind, EndpointConfig};

pub mod embed;
mod logprob;
mod rerank;
mod systemone;

pub struct Endpoint {
    name: String,
    config: EndpointConfig,
    api_key: Option<String>,
    client: Client,
    /// The last detected info and when it was detected.
    info: Mutex<Option<(Instant, Arc<Info>)>>,
}

// Decision models answer in under 1 s; 20 s only absorbs a cold model load.
const DECISION_TIMEOUT: Duration = Duration::from_secs(20);
// A new model behind the same URL gets its own fingerprint, and so its own cache and calibration, within this time.
const INFO_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct BackendError(pub String);

impl From<Error> for BackendError {
    fn from(error: Error) -> Self {
        Self(error.to_string())
    }
}

impl Endpoint {
    #[must_use]
    pub fn new(name: &str, config: &EndpointConfig, api_key: Option<String>) -> Self {
        Self {
            name: name.into(),
            config: config.clone(),
            api_key,
            client: Client::new(),
            info: Mutex::new(None),
        }
    }

    /// Detects the endpoint, again once `INFO_TTL` has passed.
    // ponytail: callers wait on one lock while a refresh runs (at most 5 s per GET); fine at one refresh a minute.
    pub async fn info(&self) -> Result<Arc<Info>, BackendError> {
        let mut slot = self.info.lock().await;
        let previous = match slot.as_ref() {
            Some((detected_at, info)) if detected_at.elapsed() < INFO_TTL => {
                return Ok(info.clone());
            }
            other => other.map(|(_, info)| info.clone()),
        };
        let info = match (self.detect(previous.is_some()).await, previous) {
            // Reuse the old value while nothing changed, so requests in flight and new ones share one semaphore.
            (Ok(detected), Some(previous)) if previous.same_identity(&detected) => previous,
            (Ok(detected), _) => Arc::new(detected),
            // A failed refresh keeps the last identity. A dead backend still fails the request that follows.
            (Err(_), Some(previous)) => previous,
            (Err(e), None) => return Err(e),
        };
        *slot = Some((Instant::now(), info.clone()));
        drop(slot);

        Ok(info)
    }

    async fn get_info_json(&self, path: &str) -> Result<Value, BackendError> {
        let request = self
            .client
            .get(format!("{}{path}", self.config.url))
            .timeout(Duration::from_secs(5));

        Ok(authed(request, self.api_key.as_deref())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }

    /// Takes the permit from the `info` that `ask` fetched, so each call does not lock `Endpoint::info` again.
    async fn post_json(
        &self,
        info: &Info,
        path: &str,
        mut body: Value,
    ) -> Result<Value, BackendError> {
        if let Some(model) = &self.config.model {
            body["model"] = model.as_str().into();
        }
        let permit = info
            .permits
            .acquire()
            .await
            .map_err(|e| BackendError(e.to_string()))?;
        let request = self
            .client
            .post(format!("{}{path}", self.config.url))
            .json(&body);
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

    async fn detect(&self, refresh: bool) -> Result<Info, BackendError> {
        let properties = match self.get_info_json("/props").await {
            Ok(properties) => properties,
            // Without /props the fingerprint falls back to the model id, which would switch caches; keep the old one.
            Err(e) if refresh => return Err(e),
            Err(_) => Value::Null,
        };
        let mut models = Value::Null;
        let kind = if let Some(kind) = self.config.kind {
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
            } else if self.config.model.is_some()
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
        // The model file alone: a llama.cpp build bump, often daily, must not drop the cache and calibration.
        let fingerprint = properties["model_path"]
            .as_str()
            .or(self.config.model.as_deref())
            .or_else(|| models["data"][0]["id"].as_str())
            .unwrap_or(&self.config.url)
            .into();
        let concurrency = self
            .config
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
            permits: Semaphore::new(concurrency),
        })
    }

    #[must_use]
    pub fn fitted(&self, info: &Info, state: &str) -> String {
        let maximum = self.config.max_state_chars.or(match info.kind {
            BackendKind::Systemone | BackendKind::Rerank => Some(2000),
            BackendKind::Logprob => None,
        });

        fit(state, maximum)
    }

    /// Ask all questions about an already-fitted state.
    pub async fn ask(
        &self,
        state: &str,
        questions: &[domain::question::Question],
    ) -> Result<Vec<domain::Probs>, BackendError> {
        tokio::time::timeout(DECISION_TIMEOUT, async {
            let info = self.info().await?;

            match info.kind {
                BackendKind::Systemone => systemone::ask(self, &info, state, questions).await,
                BackendKind::Logprob => logprob::ask(self, &info, state, questions).await,
                BackendKind::Rerank => rerank::ask(self, &info, state, questions).await,
            }
        })
        .await
        .map_err(|_| BackendError(format!("timed out after {DECISION_TIMEOUT:?}")))?
    }
}

pub fn authed(request: RequestBuilder, key: Option<&str>) -> RequestBuilder {
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

#[derive(Debug)]
pub struct Info {
    pub kind: BackendKind,
    pub fingerprint: String,
    pub concurrency: usize,
    permits: Semaphore,
}

impl Info {
    fn same_identity(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.fingerprint == other.fingerprint
            && self.concurrency == other.concurrency
    }
}
