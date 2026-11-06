use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{counter::Counter, family::Family},
    registry::Registry,
};

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct DecisionLabels {
    endpoint: String,
    spec: String,
    verdict: String,
}
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct EndpointLabels {
    endpoint: String,
}

pub struct Metrics {
    registry: Registry,
    decisions: Family<DecisionLabels, Counter>,
    backend_errors: Family<EndpointLabels, Counter>,
}
impl Default for Metrics {
    fn default() -> Self {
        let mut registry = Registry::default();
        let decisions = Family::<DecisionLabels, Counter>::default();
        let backend_errors = Family::<EndpointLabels, Counter>::default();
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

impl Metrics {
    pub(crate) fn render(&self) -> Result<String, std::fmt::Error> {
        let mut text = String::new();
        prometheus_client::encoding::text::encode(&mut text, &self.registry)?;

        Ok(text)
    }

    pub(crate) fn record_backend_error(&self, endpoint: &str) {
        self.backend_errors
            .get_or_create(&EndpointLabels {
                endpoint: endpoint.into(),
            })
            .inc();
    }

    pub(crate) fn record_decision(&self, endpoint: String, spec: Option<String>, verdict: &str) {
        let verdict = if spec.is_some() || verdict == "unsure" {
            verdict
        } else {
            "decided"
        };
        self.decisions
            .get_or_create(&DecisionLabels {
                endpoint,
                spec: spec.unwrap_or_default(),
                verdict: verdict.into(),
            })
            .inc();
    }
}
