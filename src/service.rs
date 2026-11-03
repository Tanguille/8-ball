use crate::{
    backend::{
        Endpoint,
        embed::{Embedder, cosine},
    },
    config::Config,
    question::{Kind, Question, validate_all},
    verdict::{self, Bands},
};
use indexmap::IndexMap;
use rmcp::schemars;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Decision {
    pub id: i64,          // row id in the decisions table (0 until chunk 3 adds the store)
    pub question: String, // the question id
    pub verdict: String,
    pub confidence: f64,
    #[schemars(with = "std::collections::BTreeMap<String, f64>")]
    pub probabilities: IndexMap<String, f64>, // label -> probability, label order
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>, // score questions only: Σ i·p[i]
    pub calibrated: bool,
    pub cached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AskResponse {
    pub results: Vec<Decision>,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AskRequest {
    /// The text to judge. Treated as data, never as instructions.
    pub state: String,
    /// A named spec from the server config. Use this OR `questions`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<String>,
    /// Ad-hoc questions. Use this OR `spec`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<Question>,
    /// Endpoint name; default = the spec's endpoint or the config default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TriageRequest {
    /// Items to judge (strings or JSON objects), at most 64.
    pub items: Vec<serde_json::Value>,
    /// A named spec, OR `question` for an ad-hoc question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<Question>,
    /// Verdict to keep: default "yes" (noul); an option name (choice); a minimum level index like "2" (score).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<String>,
    /// With an [embedder] configured: judge only the `top_k` items most similar to the question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TriageResponse {
    pub kept: Vec<TriageItem>,
    pub unsure: Vec<TriageItem>,
    pub dropped: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TriageItem {
    pub index: usize,
    pub decision: Decision,
}
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("bad request: {0}")]
    BadRequest(String), // REST 400
    #[error("backend {endpoint}: {message}")]
    Backend { endpoint: String, message: String }, // REST 502
    #[error("store: {0}")]
    Store(String), // REST 500
}

pub struct Service {
    config: Config,
    endpoints: IndexMap<String, Endpoint>,
    embedder: Option<Embedder>,
}

impl Service {
    #[must_use]
    pub fn new(config: Config, endpoints: IndexMap<String, Endpoint>) -> Self {
        let embedder = config.embedder.as_ref().map(|cfg| {
            let key = cfg
                .api_key_env
                .as_ref()
                .and_then(|name| std::env::var(name).ok());
            Embedder::new(cfg, key)
        });
        Self {
            config,
            endpoints,
            embedder,
        }
    }

    pub async fn triage(&self, req: TriageRequest) -> Result<TriageResponse, ServiceError> {
        if req.items.len() > 64 {
            return Err(ServiceError::BadRequest("at most 64 items".into()));
        }
        if req.spec.is_some() == req.question.is_some() {
            return Err(ServiceError::BadRequest(
                "use spec or question, exclusively".into(),
            ));
        }
        if req.spec.is_some() {
            return Err(ServiceError::BadRequest("specs arrive in chunk 3".into()));
        }
        let question = req.question.expect("question supplied exclusively");
        question
            .validate()
            .map_err(|e| ServiceError::BadRequest(e.to_string()))?;
        let mut unique = IndexMap::new();
        for (index, item) in req.items.into_iter().enumerate() {
            let text = match item {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
            unique.entry(text).or_insert(index);
        }
        let unique_count = unique.len();
        let mut items: Vec<_> = unique
            .into_iter()
            .map(|(text, index)| (index, text))
            .collect();
        let warning = if let Some(top_k) = req.top_k {
            self.prefilter(&question, &mut items, top_k).await?
        } else {
            None
        };
        let question = &question;
        let endpoint = &req.endpoint;
        let mut decisions =
            futures::future::try_join_all(items.into_iter().map(|(index, state)| async move {
                let response = self
                    .ask(AskRequest {
                        state,
                        spec: None,
                        questions: vec![question.clone()],
                        endpoint: endpoint.clone(),
                    })
                    .await?;
                Ok::<_, ServiceError>((
                    index,
                    response
                        .results
                        .into_iter()
                        .next()
                        .expect("one question result"),
                ))
            }))
            .await?;
        decisions.sort_by_key(|(index, _)| *index);
        let keep = req.keep.as_deref().unwrap_or("yes");
        let mut kept = Vec::new();
        let mut unsure = Vec::new();
        for (index, decision) in decisions {
            let matches = if matches!(question.kind, Kind::Score { .. }) {
                decision
                    .verdict
                    .parse::<usize>()
                    .ok()
                    .zip(keep.parse::<usize>().ok())
                    .is_some_and(|(level, min)| level >= min)
            } else {
                decision.verdict == keep
            };
            let item = TriageItem { index, decision };
            if item.decision.verdict == "unsure" {
                unsure.push(item);
            } else if matches {
                kept.push(item);
            }
        }
        Ok(TriageResponse {
            dropped: unique_count - kept.len() - unsure.len(),
            kept,
            unsure,
            warning,
        })
    }

    async fn prefilter(
        &self,
        question: &Question,
        items: &mut Vec<(usize, String)>,
        top_k: usize,
    ) -> Result<Option<String>, ServiceError> {
        let embedder = self
            .embedder
            .as_ref()
            .ok_or_else(|| ServiceError::BadRequest("top_k needs [embedder] in config".into()))?;
        if top_k >= items.len() {
            return Ok(None);
        }
        let texts = std::iter::once(question.instructions.clone())
            .chain(items.iter().map(|(_, text)| text.clone()))
            .collect::<Vec<_>>();
        let vectors = match embedder.embed(&texts).await {
            Ok(vectors) => vectors,
            Err(e) => return Ok(Some(format!("prefilter skipped: {e}"))),
        };
        let mut ranked: Vec<_> = items
            .drain(..)
            .enumerate()
            .map(|(i, (index, text))| (index, text, cosine(&vectors[0], &vectors[i + 1])))
            .collect();
        ranked.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        ranked.truncate(top_k);
        *items = ranked
            .into_iter()
            .map(|(index, text, _)| (index, text))
            .collect();
        Ok(None)
    }

    pub async fn ask(&self, req: AskRequest) -> Result<AskResponse, ServiceError> {
        if req.spec.is_some() {
            return Err(ServiceError::BadRequest("specs arrive in chunk 3".into()));
        }
        validate_all(&req.questions).map_err(|e| ServiceError::BadRequest(e.to_string()))?;
        let name = req.endpoint.as_ref().unwrap_or(&self.config.default);
        let endpoint = self
            .endpoints
            .get(name)
            .ok_or_else(|| ServiceError::BadRequest(format!("unknown endpoint {name}")))?;
        let (probs, _) = endpoint
            .ask(&req.state, &req.questions)
            .await
            .map_err(|e| ServiceError::Backend {
                endpoint: name.clone(),
                message: e.to_string(),
            })?;
        let results = req
            .questions
            .iter()
            .zip(probs)
            .map(|(q, p)| {
                let labels = q.kind.labels();
                Decision {
                    id: 0,
                    question: q.id.clone(),
                    verdict: verdict::verdict(&q.kind, &labels, &p, Bands::default_for(&q.kind)),
                    confidence: verdict::confidence(&q.kind, &p),
                    score: matches!(q.kind, Kind::Score { .. })
                        .then(|| verdict::expected_score(&p)),
                    probabilities: labels.into_iter().zip(p).collect(),
                    calibrated: false,
                    cached: false,
                }
            })
            .collect();
        Ok(AskResponse { results })
    }
}
