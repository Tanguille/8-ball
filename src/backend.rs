use crate::{
    config::{BackendKind, EndpointConfig},
    question::Question,
};
use std::time::Duration;

pub mod systemone;

/// One probability per label of the question, in `Kind::labels()` order, summing to 1.
pub type Probs = Vec<f64>;

pub struct Endpoint {
    pub name: String,
    url: String,
    kind: BackendKind,
    model: Option<String>,
    api_key: Option<String>,
    max_state_chars: Option<usize>,
    client: reqwest::Client,
    permits: tokio::sync::Semaphore,
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
        if cfg.kind != Some(BackendKind::Systemone) {
            return Err(BackendError(format!(
                "endpoint {name}: set kind = \"systemone\" (auto-detect arrives in chunk 2)"
            )));
        }
        Ok(Self {
            name: name.into(),
            url: cfg.url.clone(),
            kind: BackendKind::Systemone,
            model: cfg.model.clone(),
            api_key,
            max_state_chars: Some(cfg.max_state_chars.unwrap_or(2000)),
            client: reqwest::Client::new(),
            permits: tokio::sync::Semaphore::new(cfg.concurrency.unwrap_or(2)),
        })
    }

    /// Trim the state, then ask all questions. Returns (probs per question, state actually sent).
    pub async fn ask(
        &self,
        state: &str,
        qs: &[Question],
    ) -> Result<(Vec<Probs>, String), BackendError> {
        let state = fit(state, self.max_state_chars);
        let probs = tokio::time::timeout(DECISION_TIMEOUT, async {
            match self.kind {
                BackendKind::Systemone => systemone::ask(self, &state, qs).await,
                _ => unreachable!("constructor only accepts systemone"),
            }
        })
        .await
        .map_err(|_| BackendError("timed out after 20s".into()))??;
        Ok((probs, state))
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
