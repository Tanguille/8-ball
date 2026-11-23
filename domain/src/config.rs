use std::{env, num::NonZeroUsize, path::PathBuf};

use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub default: String,
    #[serde(default = "default_retention")]
    pub retention_days: u32,
    pub endpoints: IndexMap<String, EndpointConfig>,
    pub embedder: Option<EmbedderConfig>,
    #[serde(default)]
    pub specs: IndexMap<String, SpecConfig>,
    #[serde(default)]
    pub learning: Learning,
    pub escalation: Option<Escalation>,
}

/// Re-asks `unsure` answers on a larger endpoint before returning.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Escalation {
    /// A configured endpoint other than `default`, usually a chat model with `kind = "logprob"`.
    pub endpoint: String,
    /// Most re-asks per minute that reach the model, across all requests. Answers over the budget stay `unsure`.
    #[serde(default = "default_per_minute")]
    pub per_minute: NonZeroUsize,
}

const fn default_per_minute() -> NonZeroUsize {
    NonZeroUsize::new(30).unwrap()
}

/// How 8-ball learns from answers that agents report. Each part can be turned off.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Learning {
    /// Mark decisions whose answer the agent should report with `label`.
    pub report: bool,
    /// Answer with a reported label when the same question and state come back.
    pub remember: bool,
    /// Share of skipped decisions (noul `no`, triage `rejected`) returned as `unsure`, so the agent checks them.
    pub explore: f64,
    /// Refit an endpoint's bands each time its labelled states pass a multiple of this; 0 turns it off.
    pub recalibrate_after: usize,
    /// Also learn ad-hoc questions, by their exact text.
    pub ad_hoc: bool,
}

impl Default for Learning {
    fn default() -> Self {
        Self {
            report: true,
            remember: true,
            explore: 0.05,
            recalibrate_after: 20,
            ad_hoc: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointConfig {
    pub url: String,
    pub kind: Option<BackendKind>,
    pub concurrency: Option<NonZeroUsize>,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub max_state_chars: Option<usize>,
    pub extra_body: Option<serde_json::Map<String, serde_json::Value>>,
    /// For `kind = "logprob"` with a reasoning model: tokens the model may think before it answers. Unset or 0 asks for one token.
    pub reasoning_tokens: Option<u32>,
    /// Spaces requests evenly to stay under a hosted provider's rate limit.
    pub requests_per_minute: Option<std::num::NonZeroU32>,
    /// Seconds one HTTP attempt may take once it has a slot; default 20. Waiting for a slot is not counted, and a retry gets its own budget.
    pub timeout_secs: Option<std::num::NonZeroU64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Systemone,
    /// System One questions on a hosted `/v1/decisions` API, with a `model` and a `result` envelope.
    Decisions,
    Logprob,
    Rerank,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbedderConfig {
    pub url: String,
    pub model: String,
    pub api_key_env: Option<String>,
}

// No deny_unknown_fields here: it doesn't work together with #[serde(flatten)].
#[derive(Debug, Clone, Deserialize)]
pub struct SpecConfig {
    pub instructions: String,
    #[serde(flatten)]
    pub kind: crate::question::Kind,
    pub endpoint: Option<String>,
    /// False keeps this spec's `unsure` answers; default true when `[escalation]` exists.
    pub escalate: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    // thiserror makes `source` the error's source, which `{e:#}` prints after this message.
    #[error("cannot read {path}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid config {path}: {message}")]
    Invalid { path: String, message: String },
}

const fn default_retention() -> u32 {
    90
}

impl Config {
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        let path_string = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path_string.clone(),
            source,
        })?;

        Self::parse(&text, &path_string)
    }

    /// Parses TOML text. `source` names the text in errors, such as a path or an env var.
    pub fn parse(text: &str, source: &str) -> Result<Self, ConfigError> {
        let invalid = |message| ConfigError::Invalid {
            path: source.into(),
            message,
        };
        let mut config: Self =
            toml::from_str(text).map_err(|e: toml::de::Error| invalid(e.to_string()))?;
        for endpoint in config.endpoints.values_mut() {
            endpoint.url = normalize_url(&endpoint.url);
        }
        if let Some(embedder) = &mut config.embedder {
            embedder.url = normalize_url(&embedder.url);
        }
        config.validate().map_err(invalid)?;

        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if !self.endpoints.contains_key(&self.default) {
            return Err(format!("unknown default endpoint {}", self.default));
        }
        // A NaN fails `contains` too.
        if !(0.0..=1.0).contains(&self.learning.explore) {
            return Err("learning.explore must be between 0 and 1".into());
        }
        for (name, endpoint) in &self.endpoints {
            if endpoint.kind == Some(BackendKind::Logprob) && endpoint.model.is_none() {
                return Err(format!("endpoint {name}: logprob needs model"));
            }
        }
        if let Some(escalation) = &self.escalation {
            let endpoint = &escalation.endpoint;
            if !self.endpoints.contains_key(endpoint) {
                return Err(format!("escalation: unknown endpoint {endpoint}"));
            }
            if *endpoint == self.default {
                return Err(format!(
                    "escalation: endpoint {endpoint} is the default endpoint"
                ));
            }
        }
        for (name, spec) in &self.specs {
            let mut characters = name.chars();
            let alphanumeric =
                |character: char| character.is_ascii_lowercase() || character.is_ascii_digit();
            if name.len() > 64
                || !characters.next().is_some_and(alphanumeric)
                || !characters.all(|character| alphanumeric(character) || character == '-')
            {
                return Err(format!("invalid spec name {name}"));
            }
            if let Some(endpoint) = &spec.endpoint
                && !self.endpoints.contains_key(endpoint)
            {
                return Err(format!("spec {name}: unknown endpoint {endpoint}"));
            }
            crate::question::Question {
                id: name.clone(),
                instructions: spec.instructions.clone(),
                kind: spec.kind.clone(),
            }
            .validate()
            .map_err(|e| format!("spec {name}: {e}"))?;
        }

        Ok(())
    }
}

fn normalize_url(url: &str) -> String {
    let url = url.strip_suffix('/').unwrap_or(url);

    url.strip_suffix("/v1").unwrap_or(url).into()
}

#[must_use]
pub fn default_config_path() -> PathBuf {
    xdg_path("XDG_CONFIG_HOME", ".config", "8ball/config.toml")
}

/// `$variable/relative`, else `$HOME/fallback/relative` when the XDG variable is unset or empty.
#[must_use]
pub fn xdg_path(variable: &str, fallback: &str, relative: &str) -> PathBuf {
    env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map_or_else(
            || PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(fallback),
            PathBuf::from,
        )
        .join(relative)
}
