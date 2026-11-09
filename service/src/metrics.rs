use prometheus_client::{
    encoding::EncodeLabelSet,
    metrics::{
        counter::Counter,
        family::Family,
        histogram::{Histogram, exponential_buckets},
    },
    registry::Registry,
};

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct DecisionLabels {
    endpoint: String,
    spec: String,
    verdict: String,
    cached: bool,
}
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct EndpointLabels {
    endpoint: String,
}

type Latency = Family<EndpointLabels, Histogram, fn() -> Histogram>;

pub struct Metrics {
    registry: Registry,
    decisions: Family<DecisionLabels, Counter>,
    backend_errors: Family<EndpointLabels, Counter>,
    backend_seconds: Latency,
    trimmed_states: Family<EndpointLabels, Counter>,
}
impl Default for Metrics {
    fn default() -> Self {
        let mut registry = Registry::default();
        let decisions = Family::<DecisionLabels, Counter>::default();
        let backend_errors = Family::<EndpointLabels, Counter>::default();
        // 25 ms to 12.8 s: decision calls take about 1 s, and the 20 s timeout lands in +Inf.
        let backend_seconds: Latency =
            Family::new_with_constructor(|| Histogram::new(exponential_buckets(0.025, 2.0, 10)));
        let trimmed_states = Family::<EndpointLabels, Counter>::default();
        registry.register("eightball_decisions", "Decision results", decisions.clone());
        registry.register(
            "eightball_backend_errors",
            "Failed endpoint calls",
            backend_errors.clone(),
        );
        registry.register(
            "eightball_backend_request_duration_seconds",
            "Duration of decision calls to an endpoint, failed ones included",
            backend_seconds.clone(),
        );
        registry.register(
            "eightball_trimmed_states",
            "States whose middle was cut to fit max_state_chars",
            trimmed_states.clone(),
        );

        Self {
            registry,
            decisions,
            backend_errors,
            backend_seconds,
            trimmed_states,
        }
    }
}

impl Metrics {
    pub(crate) fn render(&self) -> String {
        let mut text = String::new();
        // Only a writer error or a bucketless native histogram fails encode; neither occurs here.
        prometheus_client::encoding::text::encode(&mut text, &self.registry)
            .expect("writing to String");

        text
    }

    pub(crate) fn record_backend_error(&self, endpoint: &str) {
        self.backend_errors
            .get_or_create(&EndpointLabels {
                endpoint: endpoint.into(),
            })
            .inc();
    }

    pub(crate) fn record_backend_call(&self, endpoint: &str, elapsed: std::time::Duration) {
        self.backend_seconds
            .get_or_create(&EndpointLabels {
                endpoint: endpoint.into(),
            })
            .observe(elapsed.as_secs_f64());
    }

    pub(crate) fn record_trimmed_state(&self, endpoint: &str) {
        self.trimmed_states
            .get_or_create(&EndpointLabels {
                endpoint: endpoint.into(),
            })
            .inc();
    }

    pub(crate) fn record_decision(
        &self,
        endpoint: &str,
        spec: Option<&str>,
        decision: &crate::Decision,
    ) {
        let verdict = if spec.is_some() || decision.verdict == "unsure" {
            decision.verdict.as_str()
        } else {
            "decided"
        };
        self.decisions
            .get_or_create(&DecisionLabels {
                endpoint: endpoint.into(),
                spec: spec.unwrap_or_default().into(),
                verdict: verdict.into(),
                cached: decision.cached,
            })
            .inc();
    }
}
