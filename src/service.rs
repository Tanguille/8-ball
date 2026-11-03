use crate::{
    backend::{
        Endpoint,
        embed::{Embedder, cosine},
    },
    config::{BackendKind, Config},
    question::{Kind, Question, validate_all},
    store::{NewDecision, Store, StoreError},
    verdict::{self, Bands},
};
use indexmap::IndexMap;
use rmcp::schemars;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Decision {
    pub id: i64,          // row id in the decisions table
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

#[derive(Debug, Serialize)]
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

#[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
struct DecisionLabels {
    endpoint: String,
    spec: String,
    verdict: String,
}
#[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
struct EndpointLabels {
    endpoint: String,
}

pub struct Metrics {
    registry: prometheus_client::registry::Registry,
    decisions: prometheus_client::metrics::family::Family<
        DecisionLabels,
        prometheus_client::metrics::counter::Counter,
    >,
    backend_errors: prometheus_client::metrics::family::Family<
        EndpointLabels,
        prometheus_client::metrics::counter::Counter,
    >,
}
impl Default for Metrics {
    fn default() -> Self {
        let mut registry = prometheus_client::registry::Registry::default();
        let decisions = prometheus_client::metrics::family::Family::<
            DecisionLabels,
            prometheus_client::metrics::counter::Counter,
        >::default();
        let backend_errors = prometheus_client::metrics::family::Family::<
            EndpointLabels,
            prometheus_client::metrics::counter::Counter,
        >::default();
        registry.register("eightball_decisions", "Decision results", decisions.clone());
        registry.register(
            "eightball_backend_errors",
            "Failed endpoint calls",
            backend_errors.clone(),
        );
        Self {
            registry,
            decisions,
            backend_errors,
        }
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
    pub fn new(config: Config, endpoints: IndexMap<String, Endpoint>, store: Store) -> Self {
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
            store,
            metrics: Metrics::default(),
        }
    }

    pub(crate) const fn specs(&self) -> &IndexMap<String, crate::config::SpecConfig> {
        &self.config.specs
    }

    pub fn metrics(&self) -> Result<String, ServiceError> {
        let mut text = String::new();
        prometheus_client::encoding::text::encode(&mut text, &self.metrics.registry)
            .map_err(|e| ServiceError::Store(format!("metrics encoding: {e}")))?;
        Ok(text)
    }

    fn backend_error(&self, name: &str, error: crate::backend::BackendError) -> ServiceError {
        self.metrics
            .backend_errors
            .get_or_create(&EndpointLabels {
                endpoint: name.into(),
            })
            .inc();
        ServiceError::Backend {
            endpoint: name.into(),
            message: error.0,
        }
    }

    pub async fn label(&self, id: i64, value: String) -> Result<LabelResponse, ServiceError> {
        let (spec, qh) = self
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
        if verdict::question_hash(question) != qh {
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
        if self.store.set_label(id, &value).await?.is_none() {
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
        let qh = verdict::question_hash(question);
        let cid = verdict::calibration_id(&qh, info.kind, &info.fingerprint);
        let mut groups: IndexMap<String, Vec<crate::store::Labelled>> = IndexMap::new();
        for row in self.store.labelled(spec, &qh).await? {
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
            let probs =
                if let Some((_, probs)) = self.store.cached(&cid, &state_hash, Some(spec)).await? {
                    probs
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
                probs,
                label: first.label.clone(),
            });
        }
        let (bands, mut report) =
            verdict::calibrate(&question.kind, &examples, precision.unwrap_or(0.95))
                .map_err(ServiceError::BadRequest)?;
        report.warnings.extend(warnings);
        if !dry_run {
            self.store
                .put_calibration(&cid, spec, &bands, &report)
                .await?;
        }
        Ok(CalibrateResponse {
            spec: spec.into(),
            calibration_id: cid,
            bands,
            report,
            stored: !dry_run,
        })
    }

    pub async fn doctor(&self) -> DoctorReport {
        let endpoints=futures::future::join_all(self.endpoints.iter().map(|(name,endpoint)|async move {
            let mut report=DoctorEndpoint {name:name.clone(),url:self.config.endpoints[name].url.clone(),kind:None,fingerprint:None,concurrency:None,probe:None,error:None};
            let info=match endpoint.info().await {
                Ok(info)=>info,
                Err(e)=> {report.error=Some(self.backend_error(name,e).to_string());return report;},
            };
            report.kind=Some(info.kind);report.fingerprint=Some(info.fingerprint.clone());report.concurrency=Some(info.concurrency);
            if info.kind!=BackendKind::Rerank {
                let question=Question {id:"probe".into(),instructions:"Is the sky blue on a clear day?".into(),kind:Kind::Noul {criteria:None}};
                let start=std::time::Instant::now();
                let result=endpoint.ask("The sky is clear and blue today.",std::slice::from_ref(&question)).await;
                let ms=start.elapsed().as_millis();
                report.probe=Some(match result {
                    Ok(probs)=>serde_json::json!({"verdict":verdict::verdict(&question.kind,&question.kind.labels(),&probs[0],Bands::default_for(&question.kind)),"p_yes":probs[0][0],"ms":ms}),
                    Err(e)=>serde_json::json!({"error":self.backend_error(name,e).to_string(),"ms":ms}),
                });
            }
            report
        })).await;
        let mut specs = Vec::new();
        for (name, spec) in &self.config.specs {
            let (questions, endpoint) = self
                .resolve_questions(Some(name), &[], None)
                .expect("validated config");
            let mut status = serde_json::json!({"name":name,"type":spec.kind.type_name(),"instructions":spec.instructions,"calibration_id":null,"calibrated":false,"report":null});
            match self.endpoints[&endpoint].info().await {
                Ok(info) => {
                    let cid = verdict::calibration_id(
                        &verdict::question_hash(&questions[0]),
                        info.kind,
                        &info.fingerprint,
                    );
                    status["calibration_id"] = serde_json::json!(cid);
                    match self.store.calibration(&cid).await {
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
            embedder: self
                .config
                .embedder
                .as_ref()
                .map(|cfg| serde_json::json!({"url":cfg.url,"model":cfg.model})),
            specs,
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
        let ad_hoc: Vec<_> = req.question.into_iter().collect();
        let (questions, endpoint) =
            self.resolve_questions(req.spec.as_deref(), &ad_hoc, req.endpoint.as_deref())?;
        let question = questions.into_iter().next().expect("one triage question");
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
        let endpoint = &endpoint;
        let spec = &req.spec;
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
        let keep = req.keep.as_deref().unwrap_or("yes");
        let mut kept = Vec::new();
        let mut unsure = Vec::new();
        let (indices, judgments): (Vec<_>, Vec<_>) = decisions.into_iter().unzip();
        let decisions = self.persist(judgments).await?;
        for (index, decision) in indices.into_iter().zip(decisions) {
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
        let judgments = self.judge(req).await?;
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
        validate_all(&questions).map_err(|e| ServiceError::BadRequest(e.to_string()))?;
        let name = endpoint.or(spec_endpoint).unwrap_or(&self.config.default);
        if !self.endpoints.contains_key(name) {
            return Err(ServiceError::BadRequest(format!("unknown endpoint {name}")));
        }
        Ok((questions, name.into()))
    }

    async fn judge(&self, req: AskRequest) -> Result<Vec<Judgment>, ServiceError> {
        let (questions, name) =
            self.resolve_questions(req.spec.as_deref(), &req.questions, req.endpoint.as_deref())?;
        let endpoint = &self.endpoints[&name];
        let backend_error = |e| self.backend_error(&name, e);
        let info = endpoint.info().await.map_err(backend_error)?;
        let state = endpoint.fitted(&req.state);
        let state_hash = verdict::short_hash(&state);
        let mut cached = Vec::new();
        let mut uncached = Vec::new();
        let mut identities = Vec::new();
        for q in &questions {
            let qh = verdict::question_hash(q);
            let cid = verdict::calibration_id(&qh, info.kind, &info.fingerprint);
            let answer = self
                .store
                .cached(&cid, &state_hash, req.spec.as_deref())
                .await?;
            if answer.is_none() {
                uncached.push(q.clone());
            }
            cached.push(answer);
            identities.push((qh, cid));
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
        for ((q, cached_answer), (qh, cid)) in questions.into_iter().zip(cached).zip(identities) {
            let (id, p, cached) = cached_answer.map_or_else(
                || {
                    (
                        0,
                        answers.next().expect("one probability vector per question"),
                        false,
                    )
                },
                |(id, p)| (id, p, true),
            );
            let calibration = if req.spec.is_some() {
                self.store.calibration(&cid).await?
            } else {
                None
            };
            let calibrated = calibration.is_some();
            let bands = calibration.map_or_else(|| Bands::default_for(&q.kind), |(bands, _)| bands);
            let labels = q.kind.labels();
            let outcome = verdict::verdict(&q.kind, &labels, &p, bands);
            let fresh = (!cached).then(|| NewDecision {
                spec: req.spec.clone(),
                question_hash: qh,
                calibration_id: cid,
                state_hash: state_hash.clone(),
                state: state.clone(),
                answer: p.clone(),
                verdict: outcome.clone(),
                endpoint: name.clone(),
            });
            let decision = Decision {
                id,
                question: q.id,
                verdict: outcome,
                confidence: verdict::confidence(&q.kind, &p),
                score: matches!(q.kind, Kind::Score { .. }).then(|| verdict::expected_score(&p)),
                probabilities: labels.into_iter().zip(p).collect(),
                calibrated,
                cached,
            };
            judgments.push(Judgment {
                endpoint: name.clone(),
                spec: req.spec.clone(),
                decision,
                fresh,
            });
        }
        Ok(judgments)
    }

    async fn persist(&self, mut judgments: Vec<Judgment>) -> Result<Vec<Decision>, ServiceError> {
        let fresh = judgments
            .iter_mut()
            .filter_map(|j| j.fresh.take())
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
                let verdict = if judgment.spec.is_none() {
                    if judgment.decision.verdict == "unsure" {
                        "unsure".into()
                    } else {
                        "decided".into()
                    }
                } else {
                    judgment.decision.verdict.clone()
                };
                self.metrics
                    .decisions
                    .get_or_create(&DecisionLabels {
                        endpoint: judgment.endpoint,
                        spec: judgment.spec.unwrap_or_default(),
                        verdict,
                    })
                    .inc();
                judgment.decision
            })
            .collect())
    }
}
