//! The decision service, and its REST, MCP and metrics front ends.

use std::{
    slice,
    time::{Duration, Instant},
};

use futures::future;
use indexmap::IndexMap;
// The JsonSchema derive expands to `schemars::` paths, so the module must be in scope too.
use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use backend::{Endpoint, embed::Embedder};
use domain::{
    config::{BackendKind, Config, Escalation},
    question::{Kind, Question},
    verdict::{self, Bands},
};
use store::{NewDecision, Store};

use crate::metrics::Metrics;

pub mod http;
pub mod mcp;
mod metrics;

// The flags are separate wire fields that callers read one by one.
#[expect(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
    /// Do the work, then report what it showed with `label`.
    #[serde(default)]
    pub report: bool,
    /// A skipped answer (noul `no`, triage `rejected`) returned as `unsure` so the caller checks it.
    #[serde(default)]
    pub explored: bool,
    /// The verdict is a label reported earlier for the same question and state.
    #[serde(default)]
    pub remembered: bool,
    /// The verdict came from the escalation endpoint, after the default model was unsure.
    #[serde(default)]
    pub escalated: bool,
    /// Why this triage item has no answer. Its verdict is `unsure`: do the work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Decision {
    /// An unstored `unsure` answer with no probabilities, which `judge` and a failed triage item fill in.
    fn unsure(question: &Question) -> Self {
        Self {
            id: 0,
            question: question.id.clone(),
            question_type: question.kind.type_name().into(),
            verdict: "unsure".into(),
            confidence: 0.0,
            probabilities: IndexMap::new(),
            score: None,
            calibrated: false,
            cached: false,
            report: false,
            explored: false,
            remembered: false,
            escalated: false,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskResponse {
    pub results: Vec<Decision>,
}
#[derive(Debug, Clone, Deserialize, JsonSchema)]
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
    /// False turns off remembered labels, exploration and reports for this request.
    #[serde(default = "learn_default")]
    pub learn: bool,
    /// True re-asks `unsure` answers on the server's escalation endpoint, false keeps them. Default: the spec's
    /// `escalate`, else on when the server has [escalation].
    #[serde(default)]
    pub escalate: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TriageRequest {
    /// Items to judge (strings or JSON objects), at most 64.
    pub items: Vec<Value>,
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
    /// False turns off remembered labels, exploration and reports for this request.
    #[serde(default = "learn_default")]
    pub learn: bool,
    /// As in `decision_ask`.
    #[serde(default)]
    pub escalate: Option<bool>,
}

const fn learn_default() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
    Store(#[from] store::StoreError),
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct QuestionsRequest {
    /// 1 to 500, default 50.
    pub limit: Option<usize>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct QuestionsResponse {
    pub questions: Vec<QuestionSummary>,
}
/// Stored decisions of one spec version or one ad-hoc question.
#[derive(Debug, Serialize, JsonSchema)]
pub struct QuestionSummary {
    pub spec: Option<String>,
    pub question_hash: String,
    /// Instructions and kind. Null for an ad-hoc question that 8-ball answered before it stored question text.
    pub question: Option<Value>,
    pub decisions: u64,
    pub unsure: u64,
    pub labelled: u64,
    pub last_at: String,
    /// `label` accepts its decisions: the current version of a configured spec, or an ad-hoc question with stored text when `learning.ad_hoc` is on.
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
    pub question: Option<Value>,
    pub labelable: bool,
    pub decisions: Vec<StoredDecision>,
}
#[derive(Debug, Serialize)]
pub struct StoredDecision {
    pub id: i64,
    pub at: String,
    pub verdict: String,
    /// label -> probability when the question is known, else the raw probabilities in label order.
    pub probabilities: Value,
    pub state: String,
    pub label: Option<String>,
    /// The endpoint that answered.
    pub endpoint: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReviewRequest {
    /// A configured spec. Only its current version can be labelled.
    pub spec: String,
    /// 1 to 500, default 50.
    pub limit: Option<usize>,
}
/// Unlabelled decisions without the model's answer, so a label stays independent of it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReviewResponse {
    pub spec: String,
    pub question: Value,
    /// The values `label` accepts.
    pub labels: Vec<String>,
    pub decisions: Vec<ReviewItem>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReviewItem {
    pub id: i64,
    pub state: String,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LabelRequest {
    /// A decision id from `review` or `decisions`.
    pub id: i64,
    /// yes or no for a noul question, else an option or level name.
    pub value: String,
}
#[derive(Debug, Serialize, JsonSchema)]
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
    pub embedder: Option<Value>,
    pub specs: Vec<Value>,
}

#[derive(Debug, Default, Serialize)]
pub struct DoctorEndpoint {
    pub name: String,
    pub url: String,
    pub kind: Option<BackendKind>,
    pub fingerprint: Option<String>,
    pub concurrency: Option<usize>,
    pub probe: Option<Value>,
    pub error: Option<String>,
}

// ponytail: one deadline for the whole triage, prefilter included; items still unjudged when it passes fail open
// to `unsure`. At 64 items, 2 slots and the 2.7 s p90 call this leaves room; a request field if callers need another
// value.
const TRIAGE_DEADLINE: Duration = Duration::from_secs(90);

// ponytail: one deadline for a single ask, escalation included, below the 25 s that the CLI and the plugin skill wait.
// The primary answer fails the request when it misses it; an escalation that misses it keeps the primary `unsure`.
const ASK_DEADLINE: Duration = Duration::from_secs(20);

/// The result of `future`, or the error of `passed` once `deadline` is reached.
async fn within<T, E>(
    deadline: tokio::time::Instant,
    future: impl Future<Output = Result<T, E>>,
    passed: impl FnOnce() -> E,
) -> Result<T, E> {
    tokio::time::timeout_at(deadline, future)
        .await
        .unwrap_or_else(|_| Err(passed()))
}

/// One request's judging switches, shared by the primary call and its escalation.
struct Pass<'a> {
    spec: Option<&'a str>,
    endpoint: &'a str,
    remember: bool,
    /// Where `unsure` answers go; `None` keeps them.
    escalation: Option<&'a Escalation>,
    deadline: tokio::time::Instant,
    /// The error message once `deadline` passes.
    passed: String,
}

struct Judgment {
    decision: Decision,
    /// The endpoint that answered.
    endpoint: String,
    /// Rows to store, oldest first. An escalated judgment holds the primary's row, then its own; the last row is
    /// the returned decision unless that decision is `cached`.
    fresh: Vec<NewDecision>,
    /// `verdict::explore_draw` for this question and state.
    explore_draw: f64,
    /// A spec, or an ad-hoc question whose text `label` can read.
    learnable: bool,
}

impl Judgment {
    /// A triage item whose judgment failed: `unsure`, not stored, never reported.
    fn failed(question: &Question, endpoint: &str, error: &ServiceError) -> Self {
        Self {
            decision: Decision {
                error: Some(error.to_string()),
                ..Decision::unsure(question)
            },
            endpoint: endpoint.into(),
            fresh: Vec::new(),
            explore_draw: 1.0,
            learnable: false,
        }
    }
}

/// What the caller does with a verdict, which decides whether `learn` reports or explores it.
enum Action {
    /// The caller would not work on it.
    Skip,
    /// The caller works on it.
    Work,
    Other,
}

/// A stored answer that saves a model call: `(id, probabilities, remembered label)`.
type Stored = Option<(i64, domain::Probs, Option<String>)>;

pub struct Service {
    config: Config,
    endpoints: IndexMap<String, Endpoint>,
    embedder: Option<Embedder>,
    store: Store,
    metrics: Metrics,
    /// Held during an automatic refit, so concurrent labels skip theirs.
    refit: Mutex<()>,
    /// The escalation budget window: its start and the escalations granted in it. A std mutex, held for a few
    /// instructions and never across an await.
    escalations: std::sync::Mutex<(Instant, usize)>,
    triage_deadline: Duration,
    ask_deadline: Duration,
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
            refit: Mutex::new(()),
            escalations: std::sync::Mutex::new((Instant::now(), 0)),
            triage_deadline: TRIAGE_DEADLINE,
            ask_deadline: ASK_DEADLINE,
        }
    }

    /// Replaces the triage and ask deadlines, so tests need not wait 90 s and 20 s.
    #[doc(hidden)]
    #[must_use]
    pub const fn with_deadlines(mut self, triage: Duration, ask: Duration) -> Self {
        self.triage_deadline = triage;
        self.ask_deadline = ask;
        self
    }

    /// `label` accepts decisions of a spec, or of an ad-hoc question with stored text while `learning.ad_hoc` is on.
    const fn learnable(&self, spec: Option<&str>, has_text: bool) -> bool {
        spec.is_some() || (self.config.learning.ad_hoc && has_text)
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
        let (spec, question_hash, state_hash) = self
            .store
            .metadata(id)
            .await?
            .ok_or_else(|| ServiceError::BadRequest(format!("no decision {id}")))?;
        let question = if let Some(spec) = &spec {
            let question = self.spec_question(spec)?;
            if verdict::question_hash(&question) != question_hash {
                return Err(ServiceError::BadRequest(format!(
                    "decision {id} was made for an older version of spec {spec}"
                )));
            }

            question
        } else {
            self.ad_hoc_question(&question_hash).await?
        };
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
        // A label serves every endpoint that answered this state, an escalated one's primary included.
        let endpoints = self
            .store
            .endpoints(spec.as_deref(), &question_hash, &state_hash)
            .await?;
        for name in &endpoints {
            if let Err(e) = self
                .maybe_recalibrate(spec.as_deref(), &question, name)
                .await
            {
                tracing::warn!(error = %e, endpoint = %name, "automatic recalibration failed");
            }
        }

        Ok(LabelResponse { id, label: value })
    }

    /// The stored text of an ad-hoc question, which `label` checks values against.
    async fn ad_hoc_question(&self, question_hash: &str) -> Result<Question, ServiceError> {
        if !self.config.learning.ad_hoc {
            return Err(ServiceError::BadRequest(
                "ad-hoc question: save it as a spec in the config first".into(),
            ));
        }
        let text = self.store.question(question_hash).await?.ok_or_else(|| {
            ServiceError::BadRequest("question text unknown: ask it again first".into())
        })?;
        // The stored text leaves out the caller's id, which `Question` needs.
        let mut question: Map<String, Value> =
            serde_json::from_str(&text).map_err(|e| ServiceError::BadRequest(e.to_string()))?;
        question.insert("id".into(), "q".into());

        serde_json::from_value(question.into()).map_err(|e| ServiceError::BadRequest(e.to_string()))
    }

    /// Refits a question's bands on `endpoint` each time its labelled states pass a multiple of
    /// `learning.recalibrate_after`.
    // ponytail: inline cached-only refit; move to a background task if it ever needs model calls
    async fn maybe_recalibrate(
        &self,
        spec: Option<&str>,
        question: &Question,
        endpoint: &str,
    ) -> Result<(), ServiceError> {
        let every = self.config.learning.recalibrate_after;
        if every == 0 {
            return Ok(());
        }
        let question_hash = verdict::question_hash(question);
        let (calibration_id, backend) = self.calibration_id(endpoint, &question_hash).await?;
        // Only the states this endpoint answered, which its fit counts as examples, so an escalation endpoint that
        // sees a few states refits on its own cadence.
        let count = self
            .store
            .labelled_states(spec, &question_hash, &calibration_id)
            .await?;
        if count < every {
            return Ok(());
        }
        let Ok(_refit) = self.refit.try_lock() else {
            return Ok(());
        };
        let stored = self.store.calibration(&calibration_id, spec).await?;
        // Refit once the count has crossed a multiple since the last fit, so a refit that a failed probe or a busy
        // lock skipped runs at the next label instead of the next multiple.
        let fitted = stored.as_ref().map_or(0, |(_, report)| report.examples);
        if count / every <= fitted / every {
            return Ok(());
        }
        let calibrated = stored.is_some();
        let previous = stored.map_or_else(
            || Bands::default_for(&question.kind, backend),
            |(bands, _)| bands,
        );
        let response = self
            .calibrate_question(spec, question, endpoint, None, false, Some(previous))
            .await?;
        // A refit that keeps the bands still records its example count, else every later label refits again.
        if calibrated && !response.stored {
            self.store
                .put_calibration(&calibration_id, spec, &response.bands, &response.report)
                .await?;
        }

        Ok(())
    }

    pub async fn questions(
        &self,
        request: QuestionsRequest,
    ) -> Result<QuestionsResponse, ServiceError> {
        let rows = self.store.questions(list_limit(request.limit)?).await?;
        let questions = rows
            .into_iter()
            .map(|row| {
                let current = row
                    .spec
                    .as_deref()
                    .and_then(|spec| self.spec_question(spec).ok());
                let (labelable, question) = self.shown_question(
                    row.spec.as_deref(),
                    current,
                    &row.question_hash,
                    row.question,
                );

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
        let (labelable, question) = self.shown_question(
            request.spec.as_deref(),
            current,
            &question_hash,
            self.store.question(&question_hash).await?,
        );
        let labels = question
            .as_ref()
            .and_then(|question| Kind::deserialize(question).ok())
            .map(|kind| kind.labels());
        let decisions = self
            .store
            .decisions(
                request.spec.as_deref(),
                &question_hash,
                request.unlabelled,
                list_limit(request.limit)?,
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
                        .zip(row.answer.into_iter().map(Value::from))
                        .collect::<Map<_, _>>()
                        .into(),
                    _ => row.answer.into(),
                },
                state: row.state,
                label: row.label,
                endpoint: row.endpoint,
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

    /// Whether `label` accepts decisions of `question_hash`, and the question to show.
    /// `label` accepts the current version of a spec, and an ad-hoc question by `learnable`.
    fn shown_question(
        &self,
        spec: Option<&str>,
        current: Option<Question>,
        question_hash: &str,
        stored: Option<String>,
    ) -> (bool, Option<Value>) {
        let labelable = if spec.is_some() {
            current
                .as_ref()
                .is_some_and(|question| verdict::question_hash(question) == question_hash)
        } else {
            self.learnable(spec, stored.is_some())
        };
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

    pub async fn review(&self, request: ReviewRequest) -> Result<ReviewResponse, ServiceError> {
        let question = self.spec_question(&request.spec)?;
        let decisions = self
            .store
            .decisions(
                Some(&request.spec),
                &verdict::question_hash(&question),
                true,
                list_limit(request.limit)?,
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

        self.calibrate_question(Some(spec), &questions[0], &name, precision, dry_run, None)
            .await
    }

    /// The calibration id of a question on an endpoint, with the endpoint's backend kind.
    async fn calibration_id(
        &self,
        name: &str,
        question_hash: &str,
    ) -> Result<(String, BackendKind), ServiceError> {
        let endpoint = self
            .endpoints
            .get(name)
            .ok_or_else(|| ServiceError::BadRequest(format!("unknown endpoint {name}")))?;
        let info = endpoint
            .info()
            .await
            .map_err(|e| self.backend_error(name, e))?;

        Ok((
            verdict::calibration_id(question_hash, info.kind, &info.fingerprint),
            info.kind,
        ))
    }

    /// Fits a question's bands to its labels. With `previous` bands (an automatic refit), it uses only
    /// answers cached under the current model, keeps a side's previous threshold when the labels cannot
    /// set it, and stores nothing when the bands do not change.
    async fn calibrate_question(
        &self,
        spec: Option<&str>,
        question: &Question,
        name: &str,
        precision: Option<f64>,
        dry_run: bool,
        previous: Option<Bands>,
    ) -> Result<CalibrateResponse, ServiceError> {
        let question_hash = verdict::question_hash(question);
        let (calibration_id, backend) = self.calibration_id(name, &question_hash).await?;
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
                .cached(&calibration_id, &state_hash, spec)
                .await?
            {
                probabilities
            } else if previous.is_some() {
                continue;
            } else {
                // Never remember here: a fit must use the model's answers, not the labels themselves.
                let (judgments, _) = self
                    .judge(&first.state, spec, slice::from_ref(question), name, false)
                    .await?;

                self.persist(judgments, spec)
                    .await?
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
        let (bands, mut report) = verdict::calibrate(
            &question.kind,
            backend,
            &examples,
            precision.unwrap_or(0.95),
            previous,
        )
        .map_err(ServiceError::BadRequest)?;
        report.warnings.extend(warnings);
        // An automatic refit that changes nothing stores nothing, so `calibrated` never flips on an empty fit.
        let stored = !dry_run && previous != Some(bands);
        if stored {
            self.store
                .put_calibration(&calibration_id, spec, &bands, &report)
                .await?;
        }

        Ok(CalibrateResponse {
            spec: spec.unwrap_or("").into(),
            calibration_id,
            bands,
            report,
            stored,
        })
    }

    pub async fn doctor(&self) -> DoctorReport {
        let endpoints =
            future::join_all(self.endpoints.iter().map(|(name, endpoint)| async move {
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
                    let start = Instant::now();
                    let result = endpoint
                        .ask(
                            "The sky is clear and blue today.",
                            slice::from_ref(&question),
                        )
                        .await;
                    let milliseconds = start.elapsed().as_millis();
                    report.probe = Some(match result {
                        Ok(probabilities) => json!({
                            "verdict": verdict::verdict(
                                &question.kind, &probabilities[0],
                                Bands::default_for(&question.kind, info.kind)
                            ),
                            "p_yes": probabilities[0][0],
                            "ms": milliseconds
                        }),
                        Err(e) => json!({
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
            let mut status = json!({
                "name": name,
                "type": spec.kind.type_name(),
                "instructions": spec.instructions,
                "calibration_id": null, "calibrated": false, "report": null
            });
            match self
                .calibration_id(&endpoint, &verdict::question_hash(&questions[0]))
                .await
            {
                Ok((calibration_id, _)) => {
                    status["calibration_id"] = json!(calibration_id);
                    match self.store.calibration(&calibration_id, Some(name)).await {
                        Ok(Some((_, report))) => {
                            status["calibrated"] = json!(true);
                            status["report"] = json!(report);
                        }

                        Ok(None) => {}
                        Err(e) => status["error"] = json!(e.to_string()),
                    }
                }
                Err(e) => status["error"] = json!(e.to_string()),
            }
            specs.push(status);
        }

        DoctorReport {
            default: self.config.default.clone(),
            endpoints,
            embedder: self.config.embedder.as_ref().map(|configuration| {
                json!({
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
        let (questions, endpoint) = self.resolve_questions(
            request.spec.as_deref(),
            request.question.as_slice(),
            request.endpoint.as_deref(),
        )?;
        let question = questions.into_iter().next().expect("one triage question");
        let keep = request.keep.as_deref().unwrap_or("yes");
        if !question.kind.labels().iter().any(|label| label == keep) {
            return Err(ServiceError::BadRequest(format!(
                "invalid keep value: {keep}"
            )));
        }
        let (mut items, firsts) = self
            .triage_items(&endpoint, request.items, request.context.as_deref())
            .await?;
        let spec = request.spec.as_deref();
        let pass = self.pass(
            spec,
            &endpoint,
            request.learn,
            request.escalate,
            "triage",
            self.triage_deadline,
        );
        let warning = if let Some(top_k) = request.top_k {
            self.prefilter(&question, &mut items, top_k, &pass).await?
        } else {
            None
        };
        let (mut decisions, failures) = self.judge_items(items, &question, &pass).await?;
        let warning = [warning, failures]
            .into_iter()
            .flatten()
            .reduce(|first, second| format!("{first}; {second}"));
        decisions.sort_by_key(|(index, _)| *index);
        let matches = |verdict: &str| {
            if matches!(question.kind, Kind::Score { .. }) {
                verdict
                    .parse::<usize>()
                    .ok()
                    .zip(keep.parse::<usize>().ok())
                    .is_some_and(|(level, minimum)| level >= minimum)
            } else {
                verdict == keep
            }
        };
        // Before the split, so an explored item lands in `unsure`.
        if request.learn {
            for (_, judgment) in &mut decisions {
                let verdict = &judgment.decision.verdict;
                let action = if matches(verdict) {
                    Action::Work
                } else if verdict == "unsure" {
                    Action::Other
                } else {
                    Action::Skip
                };
                self.learn(judgment, &action);
            }
        }
        let (indices, judgments): (Vec<_>, Vec<_>) = decisions.into_iter().unzip();
        let decisions: IndexMap<_, _> = indices
            .into_iter()
            .zip(self.persist(judgments, spec).await?)
            .collect();
        let mut kept = Vec::new();
        let mut unsure = Vec::new();
        let mut rejected = Vec::new();
        let mut prefiltered = Vec::new();
        // A duplicate shares the decision of its first copy; a first copy without one was prefiltered.
        for (index, first) in firsts.into_iter().enumerate() {
            let Some(decision) = decisions.get(&first).cloned() else {
                prefiltered.push(index);

                continue;
            };
            let item = TriageItem { index, decision };
            if item.decision.verdict == "unsure" {
                unsure.push(item);
            } else if matches(&item.decision.verdict) {
                kept.push(item);
            } else {
                rejected.push(item);
            }
        }

        Ok(TriageResponse {
            dropped: rejected.len() + prefiltered.len(),
            kept,
            unsure,
            warning,
            rejected,
            prefiltered,
        })
    }

    /// Judges each triage item before the deadline. A failed item fails open to `unsure`, with a warning that counts
    /// the failures; if every item failed, the request fails with the first error.
    async fn judge_items(
        &self,
        items: Vec<(usize, String)>,
        question: &Question,
        pass: &Pass<'_>,
    ) -> Result<(Vec<(usize, Judgment)>, Option<String>), ServiceError> {
        let questions = slice::from_ref(question);
        let results = future::join_all(items.into_iter().map(|(index, state)| async move {
            let judged = self.judge_escalating(&state, questions, pass).await;

            (index, judged.map(|mut judgments| judgments.remove(0)))
        }))
        .await;
        let total = results.len();
        let failed = results.iter().filter(|(_, judged)| judged.is_err()).count();
        if failed > 0 && failed == total {
            let first = results.into_iter().find_map(|(_, judged)| judged.err());

            return Err(first.expect("a failed item"));
        }
        let warning = results
            .iter()
            .find_map(|(_, judged)| judged.as_ref().err())
            .map(|error| format!("{failed} of {total} items failed and are unsure: {error}"));
        let decisions = results
            .into_iter()
            .map(|(index, judged)| {
                (
                    index,
                    judged
                        .unwrap_or_else(|error| Judgment::failed(question, pass.endpoint, &error)),
                )
            })
            .collect();

        Ok((decisions, warning))
    }

    /// The distinct item states with the index of their first copy, and for every item the index of its first copy.
    async fn triage_items(
        &self,
        endpoint: &str,
        items: Vec<Value>,
        context: Option<&str>,
    ) -> Result<(Vec<(usize, String)>, Vec<usize>), ServiceError> {
        let mut unique = IndexMap::new();
        let mut firsts = Vec::with_capacity(items.len());
        for (index, item) in items.into_iter().enumerate() {
            let text = match (&context, item) {
                (Some(context), item) => json!({"context":context,"item":item}).to_string(),
                (None, Value::String(text)) => text,
                (None, other) => other.to_string(),
            };
            firsts.push(*unique.entry(text).or_insert(index));
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

        Ok((items, firsts))
    }

    /// Keeps the `top_k` items most similar to the question, or all of them with a warning if embedding fails.
    async fn prefilter(
        &self,
        question: &Question,
        items: &mut Vec<(usize, String)>,
        top_k: usize,
        pass: &Pass<'_>,
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
        let passed = || backend::BackendError(pass.passed.clone());
        let vectors = match within(pass.deadline, embedder.embed(&texts), passed).await {
            Ok(vectors) => vectors,
            Err(e) => return Ok(Some(format!("prefilter skipped: {e}"))),
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
        *items = ranked
            .into_iter()
            .take(top_k)
            .map(|(index, text, _)| (index, text))
            .collect();

        Ok(None)
    }

    pub async fn ask(&self, request: AskRequest) -> Result<AskResponse, ServiceError> {
        let spec = request.spec.as_deref();
        let (questions, endpoint) =
            self.resolve_questions(spec, &request.questions, request.endpoint.as_deref())?;
        let pass = self.pass(
            spec,
            &endpoint,
            request.learn,
            request.escalate,
            "ask",
            self.ask_deadline,
        );
        let mut judgments = self
            .judge_escalating(&request.state, &questions, &pass)
            .await?;
        if request.learn {
            for (judgment, question) in judgments.iter_mut().zip(&questions) {
                // A caller works on a noul `yes` and skips a `no`; choices and scores are worked only when unsure.
                let noul = matches!(question.kind, Kind::Noul { .. });
                let action = match judgment.decision.verdict.as_str() {
                    "no" if noul => Action::Skip,
                    "yes" if noul => Action::Work,
                    _ => Action::Other,
                };
                self.learn(judgment, &action);
            }
        }

        Ok(AskResponse {
            results: self.persist(judgments, spec).await?,
        })
    }

    fn resolve_questions(
        &self,
        spec: Option<&str>,
        questions: &[Question],
        endpoint: Option<&str>,
    ) -> Result<(Vec<Question>, String), ServiceError> {
        let questions = if let Some(name) = spec {
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
            vec![Question {
                id: name.into(),
                instructions: spec.instructions.clone(),
                kind: spec.kind.clone(),
            }]
        } else {
            questions.to_vec()
        };
        domain::question::validate_all(&questions)
            .map_err(|e| ServiceError::BadRequest(e.to_string()))?;
        let name = endpoint
            .or_else(|| spec.and_then(|name| self.config.specs.get(name)?.endpoint.as_deref()))
            .unwrap_or(&self.config.default);
        if !self.endpoints.contains_key(name) {
            return Err(ServiceError::BadRequest(format!("unknown endpoint {name}")));
        }

        Ok((questions, name.into()))
    }

    /// Judges valid questions on `name`, which must be a configured endpoint.
    /// With `remember`, a label reported for the same question and state is the verdict.
    /// Also returns the state as fitted to the endpoint, which `escalate` re-asks about.
    async fn judge(
        &self,
        state: &str,
        spec: Option<&str>,
        questions: &[Question],
        name: &str,
        remember: bool,
    ) -> Result<(Vec<Judgment>, String), ServiceError> {
        let endpoint = &self.endpoints[name];
        let backend_error = |e| self.backend_error(name, e);
        let info = endpoint.info().await.map_err(backend_error)?;
        let fitted = endpoint.fitted(&info, state);
        if fitted != state {
            self.metrics.record_trimmed_state(name);
        }
        let state = fitted;
        let state_hash = verdict::short_hash(&state);
        let mut lookups = Vec::new();
        let mut uncached = Vec::new();
        for question in questions {
            let question_hash = verdict::question_hash(question);
            let calibration_id =
                verdict::calibration_id(&question_hash, info.kind, &info.fingerprint);
            let (stored, bands, learnable) = self
                .lookup(spec, &question_hash, &calibration_id, &state_hash, remember)
                .await?;
            if stored.is_none() {
                uncached.push(question.clone());
            }
            lookups.push((question_hash, calibration_id, stored, bands, learnable));
        }
        let mut answers = if uncached.is_empty() {
            Vec::new()
        } else {
            let started = Instant::now();
            let answers = endpoint.ask(&state, &uncached).await;
            self.metrics.record_backend_call(name, started.elapsed());
            answers.map_err(backend_error)?
        }
        .into_iter();
        let mut judgments = Vec::with_capacity(questions.len());
        for (question, (question_hash, calibration_id, stored, bands, learnable)) in
            questions.iter().zip(lookups)
        {
            let calibrated = bands.is_some();
            let bands = bands.unwrap_or_else(|| Bands::default_for(&question.kind, info.kind));
            let explore_draw = verdict::explore_draw(&question_hash, &state_hash);
            let (id, probabilities, cached, label) = match stored {
                Some((id, probabilities, label)) => (id, probabilities, true, label),
                None => (
                    0,
                    answers.next().expect("one probability vector per question"),
                    false,
                    None,
                ),
            };
            let remembered = label.is_some();
            let outcome =
                label.unwrap_or_else(|| verdict::verdict(&question.kind, &probabilities, bands));
            let fresh = if cached {
                Vec::new()
            } else {
                vec![NewDecision {
                    spec: spec.map(Into::into),
                    question: stored_question(question).to_string(),
                    question_hash,
                    calibration_id,
                    state_hash: state_hash.clone(),
                    state: state.clone(),
                    answer: probabilities.clone(),
                    verdict: outcome.clone(),
                    endpoint: name.into(),
                }]
            };
            let decision = Decision {
                id,
                verdict: outcome,
                confidence: verdict::confidence(&question.kind, &probabilities),
                score: matches!(question.kind, Kind::Score { .. })
                    .then(|| verdict::expected_score(&probabilities)),
                probabilities: question
                    .kind
                    .labels()
                    .into_iter()
                    .zip(probabilities)
                    .collect(),
                calibrated,
                cached,
                remembered,
                ..Decision::unsure(question)
            };
            judgments.push(Judgment {
                decision,
                endpoint: name.into(),
                fresh,
                explore_draw,
                learnable,
            });
        }

        Ok((judgments, state))
    }

    /// The switches of one request. Its `unsure` answers go to the escalation endpoint unless the request, else the
    /// spec, turns that off, or the request already runs on that endpoint.
    fn pass<'a>(
        &'a self,
        spec: Option<&'a str>,
        endpoint: &'a str,
        learn: bool,
        escalate: Option<bool>,
        request: &'static str,
        limit: Duration,
    ) -> Pass<'a> {
        let escalation = self.config.escalation.as_ref().filter(|escalation| {
            escalation.endpoint != endpoint
                && escalate
                    .or_else(|| spec.and_then(|name| self.config.specs.get(name)?.escalate))
                    .unwrap_or(true)
        });

        Pass {
            spec,
            endpoint,
            remember: learn && self.config.learning.remember,
            escalation,
            deadline: tokio::time::Instant::now() + limit,
            passed: format!("{request} deadline of {limit:?} passed"),
        }
    }

    /// Judges before the deadline, then re-asks the `unsure` answers under the same deadline. A primary answer that
    /// misses it fails; an escalation that misses it keeps the primary `unsure`.
    async fn judge_escalating(
        &self,
        state: &str,
        questions: &[Question],
        pass: &Pass<'_>,
    ) -> Result<Vec<Judgment>, ServiceError> {
        let (mut judgments, state) = within(
            pass.deadline,
            self.judge(state, pass.spec, questions, pass.endpoint, pass.remember),
            || self.backend_error(pass.endpoint, backend::BackendError(pass.passed.clone())),
        )
        .await?;
        if let Some(escalation) = pass.escalation {
            let escalated = self.escalate(&state, questions, escalation, pass, &mut judgments);
            if tokio::time::timeout_at(pass.deadline, escalated)
                .await
                .is_err()
            {
                tracing::warn!("escalation: {}; keeping the unsure answer", pass.passed);
            }
        }

        Ok(judgments)
    }

    /// Re-asks `unsure` answers on the escalation endpoint, about `state` as the primary endpoint saw it, so both
    /// stored rows share a state hash and one label serves both. That needs a state limit on the escalation endpoint
    /// at least as large as the primary's, because a smaller one trims `state` again. Fails open: an error or a spent
    /// budget keeps the primary answer. It changes `judgments` only after its last await, so a deadline can cancel it
    /// safely.
    async fn escalate(
        &self,
        state: &str,
        questions: &[Question],
        escalation: &Escalation,
        pass: &Pass<'_>,
        judgments: &mut [Judgment],
    ) {
        let name = escalation.endpoint.as_str();
        let mut unsure: Vec<usize> = judgments
            .iter()
            .enumerate()
            .filter(|(_, judgment)| {
                judgment.decision.verdict == "unsure" && !judgment.decision.remembered
            })
            .map(|(index, _)| index)
            .collect();
        let (granted, granted_in) = {
            let mut window = self.escalations.lock().expect("escalation budget");
            let granted = grant(
                &mut window,
                Instant::now(),
                escalation.per_minute,
                unsure.len(),
            );
            (granted, window.0)
        };
        if granted < unsure.len() {
            self.metrics
                .record_escalation(name, "budget", unsure.len() - granted);
        }
        unsure.truncate(granted);
        if unsure.is_empty() {
            return;
        }
        let asked: Vec<Question> = unsure
            .iter()
            .map(|&index| questions[index].clone())
            .collect();
        let escalated = match self
            .judge(state, pass.spec, &asked, name, pass.remember)
            .await
        {
            Ok((escalated, _)) => escalated,
            Err(error) => {
                tracing::warn!(%error, "escalation failed; keeping the unsure answer");
                self.metrics.record_escalation(name, "error", unsure.len());
                return;
            }
        };
        // A cached answer made no model call, so it gives its grant back.
        let cached = escalated
            .iter()
            .filter(|judgment| judgment.decision.cached)
            .count();
        refund(
            &mut self.escalations.lock().expect("escalation budget"),
            granted_in,
            cached,
        );
        for (&index, mut judgment) in unsure.iter().zip(escalated) {
            let outcome = if judgment.decision.verdict == "unsure" {
                "unsure"
            } else {
                "decided"
            };
            self.metrics.record_escalation(name, outcome, 1);
            judgment.decision.escalated = true;
            judgment
                .fresh
                .splice(0..0, std::mem::take(&mut judgments[index].fresh));
            judgments[index] = judgment;
        }
    }

    /// Reads what the store knows about one question and state, before the model call:
    /// a stored answer, calibrated bands if any, and whether the question is learnable.
    async fn lookup(
        &self,
        spec: Option<&str>,
        question_hash: &str,
        calibration_id: &str,
        state_hash: &str,
        remember: bool,
    ) -> Result<(Stored, Option<Bands>, bool), ServiceError> {
        // Only an ad-hoc question needs its stored text to be learnable; a spec always is.
        let has_text = spec.is_none()
            && self.config.learning.ad_hoc
            && self.store.question(question_hash).await?.is_some();
        let learnable = self.learnable(spec, has_text);
        let bands = if learnable {
            self.store
                .calibration(calibration_id, spec)
                .await?
                .map(|(bands, _)| bands)
        } else {
            None
        };
        let remembered = if remember && learnable {
            self.store
                .remembered(spec, question_hash, state_hash)
                .await?
        } else {
            None
        };
        let stored = if let Some((id, probabilities, label)) = remembered {
            Some((id, probabilities, Some(label)))
        } else {
            self.store
                .cached(calibration_id, state_hash, spec)
                .await?
                .map(|(id, probabilities)| (id, probabilities, None))
        };
        // `persist` stores the text of a fresh ad-hoc question, so `label` can read it.
        let learnable = self.learnable(spec, has_text || stored.is_none());

        Ok((stored, bands, learnable))
    }

    /// Marks a decision to report, or explores a skipped one, after `judge` and before `persist`.
    /// The stored row keeps the model's verdict; only the returned decision changes.
    fn learn(&self, judgment: &mut Judgment, action: &Action) {
        let learning = &self.config.learning;
        let decision = &mut judgment.decision;
        if !learning.report || !judgment.learnable || decision.remembered {
            return;
        }

        if matches!(action, Action::Skip) && judgment.explore_draw < learning.explore {
            decision.verdict = "unsure".into();
            decision.explored = true;
        }
        decision.report = matches!(action, Action::Work) || decision.verdict == "unsure";
    }

    async fn persist(
        &self,
        mut judgments: Vec<Judgment>,
        spec: Option<&str>,
    ) -> Result<Vec<Decision>, ServiceError> {
        let counts: Vec<usize> = judgments
            .iter()
            .map(|judgment| judgment.fresh.len())
            .collect();
        let rows: Vec<NewDecision> = judgments
            .iter_mut()
            .flat_map(|judgment| std::mem::take(&mut judgment.fresh))
            .collect();
        let ids = if rows.is_empty() {
            Vec::new()
        } else {
            self.store.insert_many(rows).await?
        };
        let mut ids = ids.into_iter();

        Ok(judgments
            .into_iter()
            .zip(counts)
            .map(|(mut judgment, count)| {
                if let Some(id) = ids.by_ref().take(count).last()
                    && !judgment.decision.cached
                {
                    judgment.decision.id = id;
                }
                if judgment.decision.error.is_none() {
                    self.metrics
                        .record_decision(&judgment.endpoint, spec, &judgment.decision);
                }

                judgment.decision
            })
            .collect())
    }
}

/// Default 50, at most 500: enough to review in one go without unbounded responses.
fn list_limit(limit: Option<usize>) -> Result<u32, ServiceError> {
    match limit.unwrap_or(50) {
        0 => Err(ServiceError::BadRequest("limit must be at least 1".into())),
        limit => Ok(u32::try_from(limit.min(500)).expect("at most 500")),
    }
}

/// Gives `count` back to the window that granted it; a window that has since rolled over keeps its count.
fn refund(window: &mut (Instant, usize), granted_in: Instant, count: usize) {
    if window.0 == granted_in {
        window.1 = window.1.saturating_sub(count);
    }
}

/// Grants up to `wanted` escalations from a one-minute window that starts at its first grant.
// ponytail: fixed window, so a burst across a window edge can spend 2 x per_minute within 60 s; a token bucket if
// that ever matters.
fn grant(window: &mut (Instant, usize), now: Instant, per_minute: usize, wanted: usize) -> usize {
    if now.duration_since(window.0) >= Duration::from_secs(60) {
        *window = (now, 0);
    }
    let granted = per_minute.saturating_sub(window.1).min(wanted);
    window.1 += granted;

    granted
}

/// The stored form of a question: its JSON without the caller's id, which is not part of the hash.
fn stored_question(question: &Question) -> Value {
    let mut value = serde_json::to_value(question).expect("question serializes");
    value
        .as_object_mut()
        .expect("question is an object")
        .remove("id");

    value
}

#[cfg(test)]
mod tests {
    #[test]
    fn refund_only_returns_to_the_window_that_granted() {
        let start = std::time::Instant::now();
        let mut window = (start, 0);
        assert_eq!(super::grant(&mut window, start, 3, 3), 3);
        super::refund(&mut window, start, 2);
        assert_eq!(window.1, 1);
        let later = start + std::time::Duration::from_secs(61);
        assert_eq!(super::grant(&mut window, later, 3, 3), 3);
        super::refund(&mut window, start, 3);
        assert_eq!(
            window.1, 3,
            "a refund from an old window must not free the new one"
        );
    }

    #[test]
    fn grant_spends_then_refills_after_a_minute() {
        let start = std::time::Instant::now();
        let mut window = (start, 0);
        assert_eq!(super::grant(&mut window, start, 3, 2), 2);
        assert_eq!(super::grant(&mut window, start, 3, 2), 1);
        assert_eq!(super::grant(&mut window, start, 3, 1), 0);
        let later = start + std::time::Duration::from_secs(60);
        assert_eq!(super::grant(&mut window, later, 3, 5), 3);
    }
}
