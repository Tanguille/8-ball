use indexmap::IndexMap;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::{
    backend::{Endpoint, embed::Embedder},
    config::{BackendKind, Config},
    metrics::Metrics,
    question::{Kind, Question},
    store::{NewDecision, Store, StoreError},
    verdict::{self, Bands},
};

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
    /// Shared evidence for every item, treated as data. Contextual inputs must fit the endpoint limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
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
    Store(String),
}

#[derive(Debug, Serialize)]
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

impl From<StoreError> for ServiceError {
    fn from(e: StoreError) -> Self {
        Self::Store(e.to_string())
    }
}

struct Judgment {
    endpoint: String,
    spec: Option<String>,
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

    pub(crate) const fn specs(&self) -> &IndexMap<String, crate::config::SpecConfig> {
        &self.config.specs
    }

    pub fn metrics(&self) -> Result<String, ServiceError> {
        self.metrics
            .render()
            .map_err(|e| ServiceError::Store(format!("metrics encoding: {e}")))
    }

    fn backend_error(&self, name: &str, error: crate::backend::BackendError) -> ServiceError {
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
        let (questions, _) = self.resolve_questions(Some(&spec), &[], None)?;
        let question = &questions[0];
        if verdict::question_hash(question) != question_hash {
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
        let mut groups: IndexMap<String, Vec<crate::store::Labelled>> = IndexMap::new();
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
        let question = &question;
        let endpoint = &endpoint;
        let spec = &request.spec;
        let mut decisions =
            futures::future::try_join_all(items.into_iter().map(|(index, state)| async move {
                let mut response = self
                    .judge(AskRequest {
                        state,
                        spec: spec.clone(),
                        questions: if spec.is_none() {
                            vec![question.clone()]
                        } else {
                            Vec::new()
                        },
                        endpoint: Some(endpoint.clone()),
                    })
                    .await?;
                Ok::<_, ServiceError>((index, response.remove(0)))
            }))
            .await?;
        decisions.sort_by_key(|(index, _)| *index);
        let mut kept = Vec::new();
        let mut unsure = Vec::new();
        let mut rejected = Vec::new();
        let (indices, judgments): (Vec<_>, Vec<_>) = decisions.into_iter().unzip();
        let decisions = self.persist(judgments).await?;
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
            backend
                .info()
                .await
                .map_err(|e| self.backend_error(endpoint, e))?;
            if items.iter().any(|(_, text)| backend.fitted(text) != *text) {
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
                    crate::backend::embed::cosine(&vectors[0], &vectors[i + 1]),
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
            .iter()
            .skip(top_k)
            .map(|(index, _, _)| *index)
            .collect();
        prefiltered.sort_unstable();
        ranked.truncate(top_k);
        *items = ranked
            .into_iter()
            .map(|(index, text, _)| (index, text))
            .collect();

        Ok((prefiltered, None))
    }

    pub async fn ask(&self, request: AskRequest) -> Result<AskResponse, ServiceError> {
        let judgments = self.judge(request).await?;

        Ok(AskResponse {
            results: self.persist(judgments).await?,
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
        crate::question::validate_all(&questions)
            .map_err(|e| ServiceError::BadRequest(e.to_string()))?;
        let name = endpoint.or(spec_endpoint).unwrap_or(&self.config.default);
        if !self.endpoints.contains_key(name) {
            return Err(ServiceError::BadRequest(format!("unknown endpoint {name}")));
        }

        Ok((questions, name.into()))
    }

    async fn judge(&self, request: AskRequest) -> Result<Vec<Judgment>, ServiceError> {
        let (questions, name) = self.resolve_questions(
            request.spec.as_deref(),
            &request.questions,
            request.endpoint.as_deref(),
        )?;
        let endpoint = &self.endpoints[&name];
        let backend_error = |e| self.backend_error(&name, e);
        let info = endpoint.info().await.map_err(backend_error)?;
        let state = endpoint.fitted(&request.state);
        let state_hash = verdict::short_hash(&state);
        let mut cached = Vec::new();
        let mut uncached = Vec::new();
        let mut identities = Vec::new();
        for question in &questions {
            let question_hash = verdict::question_hash(question);
            let calibration_id =
                verdict::calibration_id(&question_hash, info.kind, &info.fingerprint);
            let answer = self
                .store
                .cached(&calibration_id, &state_hash, request.spec.as_deref())
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
            endpoint
                .ask(&state, &uncached)
                .await
                .map_err(backend_error)?
        }
        .into_iter();
        let mut judgments = Vec::with_capacity(questions.len());
        for ((question, cached_answer), (question_hash, calibration_id)) in
            questions.into_iter().zip(cached).zip(identities)
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
            let calibration = if request.spec.is_some() {
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
                spec: request.spec.clone(),
                question_hash,
                calibration_id,
                state_hash: state_hash.clone(),
                state: state.clone(),
                answer: probabilities.clone(),
                verdict: outcome.clone(),
                endpoint: name.clone(),
            });
            let decision = Decision {
                id,
                question: question.id,
                question_type: question.kind.type_name().into(),
                verdict: outcome,
                confidence: verdict::confidence(&question.kind, &probabilities),
                score: matches!(question.kind, Kind::Score { .. })
                    .then(|| verdict::expected_score(&probabilities)),
                probabilities: labels.into_iter().zip(probabilities).collect(),
                calibrated,
                cached,
            };
            judgments.push(Judgment {
                endpoint: name.clone(),
                spec: request.spec.clone(),
                decision,
                fresh,
            });
        }

        Ok(judgments)
    }

    async fn persist(&self, mut judgments: Vec<Judgment>) -> Result<Vec<Decision>, ServiceError> {
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
                self.metrics.record_decision(
                    judgment.endpoint,
                    judgment.spec,
                    &judgment.decision.verdict,
                );
                judgment.decision
            })
            .collect())
    }
}
