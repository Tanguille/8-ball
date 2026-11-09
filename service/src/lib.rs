//! The decision service, and its REST, MCP and metrics front ends.

pub mod http;
pub mod mcp;
mod metrics;

use indexmap::IndexMap;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use backend::{Endpoint, embed::Embedder};
use domain::{
    config::{BackendKind, Config},
    question::{Kind, Question},
    verdict::{self, Bands},
};
use store::{NewDecision, Store, StoreError};

use crate::metrics::Metrics;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Decision {
    pub id: i64,          // row id in the decisions table
    pub question: String, // the question id
    #[serde(rename = "type")]
    pub question_type: String,
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
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct AskRequest {
    /// The text to judge. Treated as data, never as instructions.
    pub state: String,
    /// A named spec from the server config. Use this OR `questions`.
    #[serde(default)]
    pub spec: Option<String>,
    /// Ad-hoc questions. Use this OR `spec`.
    #[serde(default)]
    pub questions: Vec<Question>,
    /// Endpoint name; default = the spec's endpoint or the config default.
    #[serde(default)]
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TriageRequest {
    /// Items to judge (strings or JSON objects), at most 64.
    pub items: Vec<serde_json::Value>,
    /// Shared evidence for every item, treated as data. Contextual inputs must fit the endpoint limit.
    #[serde(default)]
    pub context: Option<String>,
    /// A named spec, OR `question` for an ad-hoc question.
    #[serde(default)]
    pub spec: Option<String>,
    #[serde(default)]
    pub question: Option<Question>,
    /// Verdict to keep: default "yes" (noul); an option name (choice); a minimum level index like "2" (score).
    #[serde(default)]
    pub keep: Option<String>,
    /// With an [embedder] configured: judge only the `top_k` items most similar to the question.
    #[serde(default)]
    pub top_k: Option<usize>,
    #[serde(default)]
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TriageResponse {
    pub kept: Vec<TriageItem>,
    pub unsure: Vec<TriageItem>,
    pub dropped: usize,
    pub rejected: Vec<TriageItem>,
    /// Original indices excluded by embedding prefiltering, without a model judgment.
    pub prefiltered: Vec<usize>,
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
    BadRequest(String),
    #[error("backend {endpoint}: {message}")]
    Backend { endpoint: String, message: String },
    #[error("store: {0}")]
    Store(#[from] StoreError),
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct QuestionsRequest {
    /// Default 50, at most 500.
    pub limit: Option<usize>,
}
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct QuestionsResponse {
    pub questions: Vec<QuestionSummary>,
}
/// Stored decisions of one spec version or one ad-hoc question.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct QuestionSummary {
    pub spec: Option<String>,
    pub question_hash: String,
    /// Instructions and kind. Null for an ad-hoc question that 8-ball answered before it stored question text.
    pub question: Option<serde_json::Value>,
    pub decisions: i64,
    pub unsure: i64,
    pub labelled: i64,
    pub last_at: String,
    /// The current version of a configured spec: `label` accepts its decisions.
    pub labelable: bool,
}
/// `spec` alone selects its current version; `question` alone selects an ad-hoc question; both select an older spec version.
#[derive(Debug, Default, Deserialize)]
pub struct DecisionsRequest {
    pub spec: Option<String>,
    pub question: Option<String>,
    #[serde(default)]
    pub unlabelled: bool,
    pub limit: Option<usize>,
}
#[derive(Debug, Serialize)]
pub struct DecisionsResponse {
    pub spec: Option<String>,
    pub question_hash: String,
    pub question: Option<serde_json::Value>,
    pub labelable: bool,
    pub decisions: Vec<StoredDecision>,
}
#[derive(Debug, Serialize)]
pub struct StoredDecision {
    pub id: i64,
    pub at: String,
    pub verdict: String,
    /// label -> probability when the question is known, else the raw probabilities in label order.
    pub probabilities: serde_json::Value,
    pub state: String,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReviewRequest {
    /// A configured spec. Only its current version can be labelled.
    pub spec: String,
    /// Default 50, at most 500.
    pub limit: Option<usize>,
}
/// Unlabelled decisions without the model's answer, so a label stays independent of it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ReviewResponse {
    pub spec: String,
    pub question: serde_json::Value,
    /// The values `label` accepts.
    pub labels: Vec<String>,
    pub decisions: Vec<ReviewItem>,
}
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ReviewItem {
    pub id: i64,
    pub state: String,
}
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LabelRequest {
    /// A decision id from `review` or `decisions`.
    pub id: i64,
    /// yes or no for a noul spec, else an option or level name.
    pub value: String,
}
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct LabelResponse {
    pub id: i64,
    pub label: String,
}

#[derive(Debug, Serialize)]
pub struct CalibrateResponse {
    pub spec: String,
    pub calibration_id: String,
    pub bands: Bands,
    pub report: verdict::Report,
    pub stored: bool,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub default: String,
    pub endpoints: Vec<DoctorEndpoint>,
    pub embedder: Option<serde_json::Value>,
    pub specs: Vec<serde_json::Value>,
}

#[derive(Debug, Default, Serialize)]
pub struct DoctorEndpoint {
    pub name: String,
    pub url: String,
    pub kind: Option<BackendKind>,
    pub fingerprint: Option<String>,
    pub concurrency: Option<usize>,
    pub probe: Option<serde_json::Value>,
    pub error: Option<String>,
}

struct Judgment {
    decision: Decision,
    fresh: Option<NewDecision>,
}

pub struct Service {
    config: Config,
    endpoints: IndexMap<String, Endpoint>,
    embedder: Option<Embedder>,
    store: Store,
    metrics: Metrics,
}

impl Service {
    #[must_use]
    pub fn new(config: Config, store: Store) -> Self {
        let key =
            |variable: &Option<String>| variable.as_ref().and_then(|name| std::env::var(name).ok());
        let endpoints = config
            .endpoints
            .iter()
            .map(|(name, configuration)| {
                let endpoint = Endpoint::new(name, configuration, key(&configuration.api_key_env));

                (name.clone(), endpoint)
            })
            .collect();
        let embedder = config
            .embedder
            .as_ref()
            .map(|configuration| Embedder::new(configuration, key(&configuration.api_key_env)));

        Self {
            config,
            endpoints,
            embedder,
            store,
            metrics: Metrics::default(),
        }
    }

    pub(crate) const fn specs(&self) -> &IndexMap<String, domain::config::SpecConfig> {
        &self.config.specs
    }

    #[must_use]
    pub fn metrics(&self) -> String {
        self.metrics.render()
    }

    fn backend_error(&self, name: &str, error: backend::BackendError) -> ServiceError {
        self.metrics.record_backend_error(name);

        ServiceError::Backend {
            endpoint: name.into(),
            message: error.0,
        }
    }

    pub async fn label(&self, id: i64, value: String) -> Result<LabelResponse, ServiceError> {
        let (spec, question_hash) = self
            .store
            .metadata(id)
            .await?
            .ok_or_else(|| ServiceError::BadRequest(format!("no decision {id}")))?;
        let spec = spec.ok_or_else(|| {
            ServiceError::BadRequest(
                "ad-hoc question: save it as a spec in the config first".into(),
            )
        })?;
        let question = self.spec_question(&spec)?;
        if verdict::question_hash(&question) != question_hash {
            return Err(ServiceError::BadRequest(format!(
                "decision {id} was made for an older version of spec {spec}"
            )));
        }
        let value = if matches!(question.kind, Kind::Noul { .. }) {
            match value.as_str() {
                "yes" | "true" => "yes".into(),
                "no" | "false" => "no".into(),
                _ => return Err(ServiceError::BadRequest("label must be yes or no".into())),
            }
        } else {
            if !question.kind.labels().contains(&value) {
                return Err(ServiceError::BadRequest(format!("invalid label {value}")));
            }
            value
        };
        if !self.store.set_label(id, &value).await? {
            return Err(ServiceError::BadRequest(format!("no decision {id}")));
        }

        Ok(LabelResponse { id, label: value })
    }

    pub async fn questions(
        &self,
        request: QuestionsRequest,
    ) -> Result<QuestionsResponse, ServiceError> {
        let rows = self.store.questions(list_limit(request.limit)).await?;
        let questions = rows
            .into_iter()
            .map(|row| {
                let current = row
                    .spec
                    .as_deref()
                    .and_then(|spec| self.spec_question(spec).ok());
                let (labelable, question) =
                    shown_question(current, &row.question_hash, row.question);
                QuestionSummary {
                    spec: row.spec,
                    question_hash: row.question_hash,
                    question,
                    decisions: row.decisions,
                    unsure: row.unsure,
                    labelled: row.labelled,
                    last_at: row.last_at,
                    labelable,
                }
            })
            .collect();

        Ok(QuestionsResponse { questions })
    }

    pub async fn decisions(
        &self,
        request: DecisionsRequest,
    ) -> Result<DecisionsResponse, ServiceError> {
        let current = request
            .spec
            .as_deref()
            .map(|spec| self.spec_question(spec))
            .transpose()?;
        let question_hash = match (&request.question, &current) {
            (Some(hash), _) => hash.clone(),
            (None, Some(question)) => verdict::question_hash(question),
            (None, None) => {
                return Err(ServiceError::BadRequest("give spec or question".into()));
            }
        };
        let (labelable, question) = shown_question(
            current,
            &question_hash,
            self.store.question(&question_hash).await?,
        );
        let labels = question
            .clone()
            .and_then(|question| serde_json::from_value::<Kind>(question).ok())
            .map(|kind| kind.labels());
        let decisions = self
            .store
            .decisions(
                request.spec.as_deref(),
                &question_hash,
                request.unlabelled,
                list_limit(request.limit),
            )
            .await?
            .into_iter()
            .map(|row| StoredDecision {
                id: row.id,
                at: row.at,
                verdict: row.verdict,
                probabilities: match &labels {
                    Some(labels) if labels.len() == row.answer.len() => labels
                        .iter()
                        .cloned()
                        .zip(row.answer.into_iter().map(serde_json::Value::from))
                        .collect::<serde_json::Map<_, _>>()
                        .into(),
                    _ => row.answer.into(),
                },
                state: row.state,
                label: row.label,
            })
            .collect();

        Ok(DecisionsResponse {
            spec: request.spec,
            question_hash,
            question,
            labelable,
            decisions,
        })
    }

    pub async fn review(&self, request: ReviewRequest) -> Result<ReviewResponse, ServiceError> {
        let question = self.spec_question(&request.spec)?;
        let decisions = self
            .store
            .decisions(
                Some(&request.spec),
                &verdict::question_hash(&question),
                true,
                list_limit(request.limit),
            )
            .await?
            .into_iter()
            .map(|row| ReviewItem {
                id: row.id,
                state: row.state,
            })
            .collect();

        Ok(ReviewResponse {
            spec: request.spec,
            labels: question.kind.labels(),
            question: stored_question(&question),
            decisions,
        })
    }

    fn spec_question(&self, spec: &str) -> Result<Question, ServiceError> {
        Ok(self.resolve_questions(Some(spec), &[], None)?.0.remove(0))
    }

    pub async fn calibrate(
        &self,
        spec: &str,
        precision: Option<f64>,
        dry_run: bool,
    ) -> Result<CalibrateResponse, ServiceError> {
        let (questions, name) = self.resolve_questions(Some(spec), &[], None)?;
        let question = &questions[0];
        let endpoint = &self.endpoints[&name];
        let info = endpoint
            .info()
            .await
            .map_err(|e| self.backend_error(&name, e))?;
        let question_hash = verdict::question_hash(question);
        let calibration_id = verdict::calibration_id(&question_hash, info.kind, &info.fingerprint);
        let mut groups: IndexMap<String, Vec<store::Labelled>> = IndexMap::new();
        for row in self.store.labelled(spec, &question_hash).await? {
            groups.entry(row.state_hash.clone()).or_default().push(row);
        }
        let mut warnings = Vec::new();
        let mut examples = Vec::new();
        for (state_hash, rows) in groups {
            let first = &rows[0];
            if rows.iter().any(|row| row.label != first.label) {
                let ids = rows
                    .iter()
                    .map(|row| row.id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                warnings.push(format!("conflicting labels for decision ids {ids}"));
                continue;
            }
            let probabilities = if let Some((_, probabilities)) = self
                .store
                .cached(&calibration_id, &state_hash, Some(spec))
                .await?
            {
                probabilities
            } else {
                self.ask(AskRequest {
                    state: first.state.clone(),
                    spec: Some(spec.into()),
                    questions: vec![],
                    endpoint: Some(name.clone()),
                })
                .await?
                .results
                .remove(0)
                .probabilities
                .into_values()
                .collect()
            };
            examples.push(verdict::Example {
                probs: probabilities,
                label: first.label.clone(),
            });
        }
        let (bands, mut report) =
            verdict::calibrate(&question.kind, &examples, precision.unwrap_or(0.95))
                .map_err(ServiceError::BadRequest)?;
        report.warnings.extend(warnings);
        if !dry_run {
            self.store
                .put_calibration(&calibration_id, spec, &bands, &report)
                .await?;
        }

        Ok(CalibrateResponse {
            spec: spec.into(),
            calibration_id,
            bands,
            report,
            stored: !dry_run,
        })
    }

    pub async fn doctor(&self) -> DoctorReport {
        let endpoints =
            futures::future::join_all(self.endpoints.iter().map(|(name, endpoint)| async move {
                let mut report = DoctorEndpoint {
                    name: name.clone(),
                    url: self.config.endpoints[name].url.clone(),
                    ..Default::default()
                };
                let info = match endpoint.info().await {
                    Ok(info) => info,
                    Err(e) => {
                        report.error = Some(self.backend_error(name, e).to_string());

                        return report;
                    }
                };
                report.kind = Some(info.kind);
                report.fingerprint = Some(info.fingerprint.clone());
                report.concurrency = Some(info.concurrency);
                if info.kind != BackendKind::Rerank {
                    let question = Question {
                        id: "probe".into(),
                        instructions: "Is the sky blue on a clear day?".into(),
                        kind: Kind::Noul { criteria: None },
                    };
                    let start = std::time::Instant::now();
                    let result = endpoint
                        .ask(
                            "The sky is clear and blue today.",
                            std::slice::from_ref(&question),
                        )
                        .await;
                    let milliseconds = start.elapsed().as_millis();
                    report.probe = Some(match result {
                        Ok(probabilities) => serde_json::json!({
                            "verdict": verdict::verdict(
                                &question.kind, &probabilities[0],
                                Bands::default_for(&question.kind)
                            ),
                            "p_yes": probabilities[0][0],
                            "ms": milliseconds
                        }),
                        Err(e) => serde_json::json!({
                            "error": self.backend_error(name, e).to_string(), "ms": milliseconds
                        }),
                    });
                }

                report
            }))
            .await;
        let mut specs = Vec::new();
        for (name, spec) in &self.config.specs {
            let (questions, endpoint) = self
                .resolve_questions(Some(name), &[], None)
                .expect("validated config");
            let mut status = serde_json::json!({
                "name": name,
                "type": spec.kind.type_name(),
                "instructions": spec.instructions,
                "calibration_id": null, "calibrated": false, "report": null
            });
            match self.endpoints[&endpoint].info().await {
                Ok(info) => {
                    let calibration_id = verdict::calibration_id(
                        &verdict::question_hash(&questions[0]),
                        info.kind,
                        &info.fingerprint,
                    );
                    status["calibration_id"] = serde_json::json!(calibration_id);
                    match self.store.calibration(&calibration_id).await {
                        Ok(Some((_, report))) => {
                            status["calibrated"] = serde_json::json!(true);
                            status["report"] = serde_json::json!(report);
                        }

                        Ok(None) => {}
                        Err(e) => status["error"] = serde_json::json!(e.to_string()),
                    }
                }
                Err(e) => {
                    status["error"] =
                        serde_json::json!(self.backend_error(&endpoint, e).to_string());
                }
            }
            specs.push(status);
        }

        DoctorReport {
            default: self.config.default.clone(),
            endpoints,
            embedder: self.config.embedder.as_ref().map(|configuration| {
                serde_json::json!({
                    "url": configuration.url,
                    "model": configuration.model
                })
            }),
            specs,
        }
    }

    pub async fn triage(&self, request: TriageRequest) -> Result<TriageResponse, ServiceError> {
        if request.items.len() > 64 {
            return Err(ServiceError::BadRequest("at most 64 items".into()));
        }
        if request.spec.is_some() == request.question.is_some() {
            return Err(ServiceError::BadRequest(
                "use spec or question, exclusively".into(),
            ));
        }
        let ad_hoc: Vec<_> = request.question.into_iter().collect();
        let (questions, endpoint) = self.resolve_questions(
            request.spec.as_deref(),
            &ad_hoc,
            request.endpoint.as_deref(),
        )?;
        let question = questions.into_iter().next().expect("one triage question");
        let keep = request.keep.as_deref().unwrap_or("yes");
        let valid_keep = match &question.kind {
            Kind::Score { criteria } => keep
                .parse::<usize>()
                .is_ok_and(|minimum| minimum < criteria.len()),
            _ => question.kind.labels().iter().any(|label| label == keep),
        };
        if !valid_keep {
            return Err(ServiceError::BadRequest(format!(
                "invalid keep value: {keep}"
            )));
        }
        let mut items = self
            .triage_items(&endpoint, request.items, request.context.as_deref())
            .await?;
        let unique_count = items.len();
        let (prefiltered, warning) = if let Some(top_k) = request.top_k {
            self.prefilter(&question, &mut items, top_k).await?
        } else {
            (Vec::new(), None)
        };
        let questions = std::slice::from_ref(&question);
        let (endpoint, spec) = (&endpoint, request.spec.as_deref());
        let mut decisions =
            futures::future::try_join_all(items.into_iter().map(|(index, state)| async move {
                let mut judgments = self.judge(&state, spec, questions, endpoint).await?;
                Ok::<_, ServiceError>((index, judgments.remove(0)))
            }))
            .await?;
        decisions.sort_by_key(|(index, _)| *index);
        let mut kept = Vec::new();
        let mut unsure = Vec::new();
        let mut rejected = Vec::new();
        let (indices, judgments): (Vec<_>, Vec<_>) = decisions.into_iter().unzip();
        let decisions = self.persist(judgments, endpoint, spec).await?;
        for (index, decision) in indices.into_iter().zip(decisions) {
            let matches = if matches!(question.kind, Kind::Score { .. }) {
                decision
                    .verdict
                    .parse::<usize>()
                    .ok()
                    .zip(keep.parse::<usize>().ok())
                    .is_some_and(|(level, minimum)| level >= minimum)
            } else {
                decision.verdict == keep
            };
            let item = TriageItem { index, decision };
            if item.decision.verdict == "unsure" {
                unsure.push(item);
            } else if matches {
                kept.push(item);
            } else {
                rejected.push(item);
            }
        }

        Ok(TriageResponse {
            dropped: unique_count - kept.len() - unsure.len(),
            kept,
            unsure,
            warning,
            rejected,
            prefiltered,
        })
    }

    async fn triage_items(
        &self,
        endpoint: &str,
        items: Vec<serde_json::Value>,
        context: Option<&str>,
    ) -> Result<Vec<(usize, String)>, ServiceError> {
        let mut unique = IndexMap::new();
        for (index, item) in items.into_iter().enumerate() {
            let text = match (&context, item) {
                (Some(context), item) => {
                    serde_json::json!({"context":context,"item":item}).to_string()
                }
                (None, serde_json::Value::String(text)) => text,
                (None, other) => other.to_string(),
            };
            unique.entry(text).or_insert(index);
        }
        let items: Vec<_> = unique
            .into_iter()
            .map(|(text, index)| (index, text))
            .collect();
        if context.is_some() && !items.is_empty() {
            let backend = &self.endpoints[endpoint];
            let info = backend
                .info()
                .await
                .map_err(|e| self.backend_error(endpoint, e))?;
            if items
                .iter()
                .any(|(_, text)| backend.fitted(&info, text) != *text)
            {
                return Err(ServiceError::BadRequest(
                    "context and item exceed the endpoint state limit; shorten the evidence".into(),
                ));
            }
        }

        Ok(items)
    }

    async fn prefilter(
        &self,
        question: &Question,
        items: &mut Vec<(usize, String)>,
        top_k: usize,
    ) -> Result<(Vec<usize>, Option<String>), ServiceError> {
        let embedder = self
            .embedder
            .as_ref()
            .ok_or_else(|| ServiceError::BadRequest("top_k needs [embedder] in config".into()))?;
        if top_k >= items.len() {
            return Ok((Vec::new(), None));
        }
        let texts = std::iter::once(question.instructions.clone())
            .chain(items.iter().map(|(_, text)| text.clone()))
            .collect::<Vec<_>>();
        let vectors = match embedder.embed(&texts).await {
            Ok(vectors) => vectors,
            Err(e) => return Ok((Vec::new(), Some(format!("prefilter skipped: {e}")))),
        };
        let mut ranked: Vec<_> = items
            .drain(..)
            .enumerate()
            .map(|(i, (index, text))| {
                (
                    index,
                    text,
                    backend::embed::cosine(&vectors[0], &vectors[i + 1]),
                )
            })
            .collect();
        ranked.sort_by(|left, right| {
            right
                .2
                .total_cmp(&left.2)
                .then_with(|| left.0.cmp(&right.0))
        });
        let mut prefiltered: Vec<_> = ranked
            .split_off(top_k)
            .into_iter()
            .map(|(index, _, _)| index)
            .collect();
        prefiltered.sort_unstable();
        *items = ranked
            .into_iter()
            .map(|(index, text, _)| (index, text))
            .collect();

        Ok((prefiltered, None))
    }

    pub async fn ask(&self, request: AskRequest) -> Result<AskResponse, ServiceError> {
        let spec = request.spec.as_deref();
        let (questions, endpoint) =
            self.resolve_questions(spec, &request.questions, request.endpoint.as_deref())?;
        let judgments = self
            .judge(&request.state, spec, &questions, &endpoint)
            .await?;

        Ok(AskResponse {
            results: self.persist(judgments, &endpoint, spec).await?,
        })
    }

    fn resolve_questions(
        &self,
        spec: Option<&str>,
        questions: &[Question],
        endpoint: Option<&str>,
    ) -> Result<(Vec<Question>, String), ServiceError> {
        let (questions, spec_endpoint) = if let Some(name) = spec {
            if !questions.is_empty() {
                return Err(ServiceError::BadRequest(
                    "use spec or questions, not both".into(),
                ));
            }
            let spec = self
                .config
                .specs
                .get(name)
                .ok_or_else(|| ServiceError::BadRequest(format!("unknown spec {name}")))?;
            (
                vec![Question {
                    id: name.into(),
                    instructions: spec.instructions.clone(),
                    kind: spec.kind.clone(),
                }],
                spec.endpoint.as_deref(),
            )
        } else {
            (questions.to_vec(), None)
        };
        domain::question::validate_all(&questions)
            .map_err(|e| ServiceError::BadRequest(e.to_string()))?;
        let name = endpoint.or(spec_endpoint).unwrap_or(&self.config.default);
        if !self.endpoints.contains_key(name) {
            return Err(ServiceError::BadRequest(format!("unknown endpoint {name}")));
        }

        Ok((questions, name.into()))
    }

    /// Judges resolved questions; `resolve_questions` must have validated them and `name`.
    async fn judge(
        &self,
        state: &str,
        spec: Option<&str>,
        questions: &[Question],
        name: &str,
    ) -> Result<Vec<Judgment>, ServiceError> {
        let endpoint = &self.endpoints[name];
        let backend_error = |e| self.backend_error(name, e);
        let info = endpoint.info().await.map_err(backend_error)?;
        let fitted = endpoint.fitted(&info, state);
        if fitted != state {
            self.metrics.record_trimmed_state(name);
        }
        let state = fitted;
        let state_hash = verdict::short_hash(&state);
        let mut cached = Vec::new();
        let mut uncached = Vec::new();
        let mut identities = Vec::new();
        for question in questions {
            let question_hash = verdict::question_hash(question);
            let calibration_id =
                verdict::calibration_id(&question_hash, info.kind, &info.fingerprint);
            let answer = self
                .store
                .cached(&calibration_id, &state_hash, spec)
                .await?;
            if answer.is_none() {
                uncached.push(question.clone());
            }
            cached.push(answer);
            identities.push((question_hash, calibration_id));
        }
        let mut answers = if uncached.is_empty() {
            Vec::new()
        } else {
            let started = std::time::Instant::now();
            let answers = endpoint.ask(&state, &uncached).await;
            self.metrics.record_backend_call(name, started.elapsed());
            answers.map_err(backend_error)?
        }
        .into_iter();
        let mut judgments = Vec::with_capacity(questions.len());
        for ((question, cached_answer), (question_hash, calibration_id)) in
            questions.iter().zip(cached).zip(identities)
        {
            let (id, probabilities, cached) = cached_answer.map_or_else(
                || {
                    (
                        0,
                        answers.next().expect("one probability vector per question"),
                        false,
                    )
                },
                |(id, probabilities)| (id, probabilities, true),
            );
            let calibration = if spec.is_some() {
                self.store.calibration(&calibration_id).await?
            } else {
                None
            };
            let calibrated = calibration.is_some();
            let bands =
                calibration.map_or_else(|| Bands::default_for(&question.kind), |(bands, _)| bands);
            let labels = question.kind.labels();
            let outcome = verdict::verdict(&question.kind, &probabilities, bands);
            let fresh = (!cached).then(|| NewDecision {
                spec: spec.map(Into::into),
                question: stored_question(question).to_string(),
                question_hash,
                calibration_id,
                state_hash: state_hash.clone(),
                state: state.clone(),
                answer: probabilities.clone(),
                verdict: outcome.clone(),
                endpoint: name.into(),
            });
            let decision = Decision {
                id,
                question: question.id.clone(),
                question_type: question.kind.type_name().into(),
                verdict: outcome,
                confidence: verdict::confidence(&question.kind, &probabilities),
                score: matches!(question.kind, Kind::Score { .. })
                    .then(|| verdict::expected_score(&probabilities)),
                probabilities: labels.into_iter().zip(probabilities).collect(),
                calibrated,
                cached,
            };
            judgments.push(Judgment { decision, fresh });
        }

        Ok(judgments)
    }

    async fn persist(
        &self,
        mut judgments: Vec<Judgment>,
        endpoint: &str,
        spec: Option<&str>,
    ) -> Result<Vec<Decision>, ServiceError> {
        let fresh = judgments
            .iter_mut()
            .filter_map(|judgment| judgment.fresh.take())
            .collect::<Vec<_>>();
        let ids = if fresh.is_empty() {
            Vec::new()
        } else {
            self.store.insert_many(fresh).await?
        };
        let mut ids = ids.into_iter();

        Ok(judgments
            .into_iter()
            .map(|mut judgment| {
                if !judgment.decision.cached {
                    judgment.decision.id = ids.next().expect("id for each fresh decision");
                }
                self.metrics
                    .record_decision(endpoint, spec, &judgment.decision);
                judgment.decision
            })
            .collect())
    }
}

/// Default 50, at most 500: enough to review in one go without unbounded responses.
fn list_limit(limit: Option<usize>) -> i64 {
    i64::try_from(limit.unwrap_or(50).min(500)).expect("at most 500")
}

/// The stored form of a question: its JSON without the caller's id, which is not part of the hash.
fn stored_question(question: &Question) -> serde_json::Value {
    let mut value = serde_json::to_value(question).expect("question serializes");
    value
        .as_object_mut()
        .expect("question is an object")
        .remove("id");

    value
}

/// Whether `label` accepts decisions of `question_hash` (it is the current spec version), and the question to show.
fn shown_question(
    current: Option<Question>,
    question_hash: &str,
    stored: Option<String>,
) -> (bool, Option<serde_json::Value>) {
    let labelable = current
        .as_ref()
        .is_some_and(|question| verdict::question_hash(question) == question_hash);
    // Decisions from before the questions table still show the current spec text.
    let question = stored
        .and_then(|text| serde_json::from_str(&text).ok())
        .or_else(|| {
            current
                .filter(|_| labelable)
                .map(|question| stored_question(&question))
        });

    (labelable, question)
}
