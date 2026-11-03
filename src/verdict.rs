use crate::question::Kind;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum Bands {
    Noul { yes_at: f64, no_at: f64 },
    Level { min_confidence: f64 },
}

impl Bands {
    /// Not tuned for any model: they only keep uncalibrated answers from deciding much.
    #[must_use]
    pub const fn default_for(kind: &Kind) -> Self {
        match kind {
            Kind::Noul { .. } => Self::Noul {
                yes_at: 0.85,
                no_at: 0.15,
            },
            _ => Self::Level {
                min_confidence: 0.6,
            },
        }
    }
}

#[must_use]
pub fn confidence_choice(p: &[f64]) -> f64 {
    if p.len() < 2 {
        return 1.0;
    }
    let n = f64::from(u32::try_from(p.len()).expect("option count fits u32"));
    ((p[argmax(p)] - 1.0 / n) / (1.0 - 1.0 / n)).max(0.0)
}

#[must_use]
pub fn confidence_score(p: &[f64]) -> f64 {
    if p.len() < 2 {
        return 1.0;
    }
    let n = f64::from(u32::try_from(p.len()).expect("level count fits u32"));
    let mode = f64::from(u32::try_from(argmax(p)).expect("level index fits u32"));
    let dist: f64 = (0_u32..)
        .zip(p)
        .map(|(i, p)| p * (f64::from(i) - mode).abs())
        .sum();
    let uniform: f64 = (0_u32..)
        .zip(p)
        .map(|(i, _)| (f64::from(i) - (n - 1.0) / 2.0).abs() / n)
        .sum();
    (1.0 - dist / uniform).max(0.0)
}

#[must_use]
pub fn confidence(kind: &Kind, p: &[f64]) -> f64 {
    match kind {
        Kind::Score { .. } => confidence_score(p),
        _ => confidence_choice(p),
    }
}

#[must_use]
pub fn expected_score(p: &[f64]) -> f64 {
    (0_u32..).zip(p).map(|(i, p)| f64::from(i) * p).sum()
}

fn argmax(p: &[f64]) -> usize {
    let mut best = 0;
    for i in 1..p.len() {
        if p[i] > p[best] {
            best = i;
        }
    }
    best
}

/// p[0] is P(yes) for noul.
#[must_use]
pub fn verdict(kind: &Kind, labels: &[String], p: &[f64], bands: Bands) -> String {
    match (kind, bands) {
        (Kind::Noul { .. }, Bands::Noul { yes_at, no_at }) => {
            let yes = p[0];
            if yes >= yes_at && yes > no_at {
                "yes".into()
            } else if yes <= no_at && yes < yes_at {
                "no".into()
            } else {
                "unsure".into()
            }
        }
        (_, Bands::Level { min_confidence }) => {
            if confidence(kind, p) >= min_confidence {
                labels[argmax(p)].clone()
            } else {
                "unsure".into()
            }
        }
        // A band of the wrong shape (stored for another kind) never decides.
        _ => "unsure".into(),
    }
}

/// First 16 hex chars of sha256(text).
#[must_use]
pub fn short_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let mut hash = String::with_capacity(16);
    for byte in Sha256::digest(text.as_bytes()).iter().take(8) {
        write!(&mut hash, "{byte:02x}").expect("writing to String");
    }
    hash
}

/// Identity of the question's meaning: type + instructions + criteria (not the id).
#[must_use]
pub fn question_hash(q: &crate::question::Question) -> String {
    short_hash(&serde_json::to_string(&(&q.instructions, &q.kind)).expect("question serializes"))
}

/// Identity of question + exact model. The url is excluded so a hostname change keeps calibrations.
#[must_use]
pub fn calibration_id(
    question_hash: &str,
    kind: crate::config::BackendKind,
    fingerprint: &str,
) -> String {
    short_hash(&format!("{question_hash}\n{kind:?}\n{fingerprint}"))
}

pub struct Example {
    pub probs: crate::backend::Probs,
    pub label: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Report {
    pub examples: usize,
    pub decided: usize,
    pub correct: usize,
    pub precision: f64,
    pub coverage: f64,
    pub target: f64,
    pub in_sample: bool,
    pub warnings: Vec<String>,
}

/// target must be in [0.5, 0.99]; otherwise Err.
pub fn calibrate(
    kind: &Kind,
    examples: &[Example],
    target: f64,
) -> Result<(Bands, Report), String> {
    if !(0.5..=0.99).contains(&target) {
        return Err("precision must be between 0.5 and 0.99".into());
    }
    let mut warnings = Vec::new();
    let labels = kind.labels();
    let bands = if matches!(kind, Kind::Noul { .. }) {
        let yes: Vec<_> = examples
            .iter()
            .map(|e| (e.probs[0], e.label == "yes"))
            .collect();
        let no: Vec<_> = examples
            .iter()
            .map(|e| (e.probs[0], e.label == "no"))
            .collect();
        let yes_at = threshold(&yes, true, true, target).unwrap_or_else(|| {
            warnings.push("yes side: not enough evidence for the target".into());
            1.01
        });
        let no_at = threshold(&no, false, true, target).unwrap_or_else(|| {
            warnings.push("no side: not enough evidence for the target".into());
            -0.01
        });
        Bands::Noul { yes_at, no_at }
    } else {
        let data: Vec<_> = examples
            .iter()
            .map(|e| {
                (
                    confidence(kind, &e.probs),
                    labels[argmax(&e.probs)] == e.label,
                )
            })
            .collect();
        let min_confidence = threshold(&data, true, false, target).unwrap_or_else(|| {
            warnings.push("not enough evidence for the target".into());
            1.01
        });
        Bands::Level { min_confidence }
    };
    let mut decided = 0;
    let mut correct = 0;
    for e in examples {
        let result = verdict(kind, &labels, &e.probs, bands);
        if result != "unsure" {
            decided += 1;
            if result == e.label {
                correct += 1;
            }
        }
    }
    if examples.len() < 20 {
        warnings.push("fewer than 20 examples".into());
    }
    let report = Report {
        examples: examples.len(),
        decided,
        correct,
        precision: ratio(correct, decided),
        coverage: ratio(decided, examples.len()),
        target,
        in_sample: true,
        warnings,
    };
    Ok((bands, report))
}

fn threshold(
    data: &[(f64, bool)],
    above: bool,
    correct_candidates_only: bool,
    target: f64,
) -> Option<f64> {
    let mut candidates: Vec<_> = data
        .iter()
        .filter(|(_, correct)| !correct_candidates_only || *correct)
        .map(|(value, _)| *value)
        .collect();
    candidates.sort_by(f64::total_cmp);
    if !above {
        candidates.reverse();
    }
    let min_side = (1.0 / (1.0 - target)).ceil();
    candidates.into_iter().find(|t| {
        let selected: Vec<_> = data
            .iter()
            .filter(|(value, _)| if above { value >= t } else { value <= t })
            .collect();
        let count = f64::from(u32::try_from(selected.len()).expect("example count fits u32"));
        count >= min_side
            && ratio(
                selected.iter().filter(|(_, correct)| *correct).count(),
                selected.len(),
            ) >= target
    })
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    f64::from(u32::try_from(numerator).expect("example count fits u32"))
        / f64::from(u32::try_from(denominator).expect("example count fits u32"))
}
