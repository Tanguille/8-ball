use serde::{Deserialize, Serialize};

use crate::service::{Decision, TriageResponse};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub item: serde_json::Value,
    pub context: Option<String>,
    pub expected_keep: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid evaluation dataset: {0}")]
pub struct DatasetError(String);

pub fn parse(input: &str) -> Result<Vec<Case>, DatasetError> {
    let mut cases = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for (index, line) in input.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let number = index + 1;
        let case: Case =
            serde_json::from_str(line).map_err(|e| DatasetError(format!("line {number}: {e}")))?;
        if case.id.trim().is_empty() || !ids.insert(case.id.clone()) {
            return Err(DatasetError(format!(
                "line {number}: empty or duplicate id"
            )));
        }
        cases.push(case);
    }
    if cases.is_empty() {
        return Err(DatasetError(
            "at least one labelled case is required".into(),
        ));
    }

    Ok(cases)
}

#[derive(Debug, Serialize)]
pub struct Observation {
    pub id: String,
    pub expected_keep: bool,
    pub would_keep: bool,
    pub elapsed_ms: f64,
    pub decision: Option<Decision>,
    pub error: Option<String>,
}

impl Observation {
    #[must_use]
    pub fn new(
        case: Case,
        elapsed: std::time::Duration,
        response: Result<TriageResponse, String>,
    ) -> Self {
        let result = response.and_then(single_decision);
        let (would_keep, decision, error) = match result {
            Ok((keep, decision)) => (keep, Some(decision), None),
            Err(error) => (true, None, Some(error)),
        };

        Self {
            id: case.id,
            expected_keep: case.expected_keep,
            would_keep,
            elapsed_ms: elapsed.as_secs_f64() * 1000.0,
            decision,
            error,
        }
    }
}

fn single_decision(response: TriageResponse) -> Result<(bool, Decision), String> {
    let rejected = !response.rejected.is_empty();
    let unsure = !response.unsure.is_empty();
    if response.kept.len() + response.unsure.len() + response.rejected.len() != 1
        || !response.prefiltered.is_empty()
        || response.dropped != usize::from(rejected)
        || response.warning.is_some()
    {
        return Err("expected exactly one judged item without prefiltering".into());
    }
    let item = response
        .kept
        .into_iter()
        .chain(response.unsure)
        .chain(response.rejected)
        .next()
        .expect("one decision checked above");
    if item.index != 0 || (item.decision.verdict == "unsure") != unsure {
        return Err("inconsistent triage decision".into());
    }

    Ok((!rejected, item.decision))
}

#[derive(Debug, Serialize)]
pub struct Latency {
    pub samples: usize,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
}

impl Latency {
    fn new(mut values: Vec<f64>) -> Self {
        values.sort_by(f64::total_cmp);
        let samples = values.len();

        Self {
            samples,
            p50_ms: samples.checked_sub(1).map(|index| values[index / 2]),
            p95_ms: samples
                .checked_sub(1)
                .map(|index| values[index - samples / 20]),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub total: u32,
    pub decided: u32,
    pub unsure: u32,
    pub errors: u32,
    pub cached: u32,
    pub would_skip: u32,
    pub missed_useful: u32,
    pub unnecessary_keeps: u32,
    pub coverage: Option<f64>,
    pub decision_accuracy: Option<f64>,
    pub skip_precision: Option<f64>,
    pub useful_recall: Option<f64>,
    pub would_skip_fraction: Option<f64>,
    pub uncached_latency: Latency,
    pub cached_latency: Latency,
}

impl Report {
    #[must_use]
    pub fn from_observations(observations: &[Observation]) -> Self {
        let count = |predicate: fn(&Observation) -> bool| {
            u32::try_from(observations.iter().filter(|item| predicate(item)).count())
                .expect("evaluation count fits u32")
        };
        let total = count(|_| true);
        let errors = count(|item| item.error.is_some());
        let unsure = count(|item| {
            item.decision
                .as_ref()
                .is_some_and(|decision| decision.verdict == "unsure")
        });
        let decided = total - errors - unsure;
        let would_skip = count(|item| !item.would_keep);
        let missed_useful = count(|item| item.expected_keep && !item.would_keep);
        let unnecessary_keeps = count(|item| {
            !item.expected_keep
                && item
                    .decision
                    .as_ref()
                    .is_some_and(|decision| decision.verdict != "unsure")
                && item.would_keep
        });
        let useful = count(|item| item.expected_keep);
        let cached = count(|item| {
            item.decision
                .as_ref()
                .is_some_and(|decision| decision.cached)
        });
        let latency = |cached| {
            Latency::new(
                observations
                    .iter()
                    .filter(|item| {
                        item.decision
                            .as_ref()
                            .is_some_and(|decision| decision.cached == cached)
                    })
                    .map(|item| item.elapsed_ms)
                    .collect(),
            )
        };

        Self {
            total,
            decided,
            unsure,
            errors,
            cached,
            would_skip,
            missed_useful,
            unnecessary_keeps,
            coverage: ratio(decided, total),
            decision_accuracy: ratio(decided - missed_useful - unnecessary_keeps, decided),
            skip_precision: ratio(would_skip - missed_useful, would_skip),
            useful_recall: ratio(useful - missed_useful, useful),
            would_skip_fraction: ratio(would_skip, total),
            uncached_latency: latency(false),
            cached_latency: latency(true),
        }
    }
}

fn ratio(numerator: u32, denominator: u32) -> Option<f64> {
    (denominator > 0).then(|| f64::from(numerator) / f64::from(denominator))
}
