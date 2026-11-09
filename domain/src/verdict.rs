use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::Digest as _;

use crate::question::Kind;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
pub fn confidence_choice(probabilities: &[f64]) -> f64 {
    if probabilities.len() < 2 {
        return 1.0;
    }
    let count = f64::from(u32::try_from(probabilities.len()).expect("option count fits u32"));

    ((probabilities[argmax(probabilities)] - 1.0 / count) / (1.0 - 1.0 / count)).max(0.0)
}

#[must_use]
pub fn confidence_score(probabilities: &[f64]) -> f64 {
    if probabilities.len() < 2 {
        return 1.0;
    }
    let count = f64::from(u32::try_from(probabilities.len()).expect("level count fits u32"));
    let mode = f64::from(u32::try_from(argmax(probabilities)).expect("level index fits u32"));
    let distance: f64 = (0_u32..)
        .zip(probabilities)
        .map(|(index, probability)| probability * (f64::from(index) - mode).abs())
        .sum();
    let uniform: f64 = (0_u32..)
        .zip(probabilities)
        .map(|(index, _)| (f64::from(index) - (count - 1.0) / 2.0).abs() / count)
        .sum();

    (1.0 - distance / uniform).max(0.0)
}

#[must_use]
pub fn confidence(kind: &Kind, probabilities: &[f64]) -> f64 {
    match kind {
        Kind::Score { .. } => confidence_score(probabilities),
        _ => confidence_choice(probabilities),
    }
}

#[must_use]
pub fn expected_score(probabilities: &[f64]) -> f64 {
    (0_u32..)
        .zip(probabilities)
        .map(|(index, probability)| f64::from(index) * probability)
        .sum()
}

fn argmax(probabilities: &[f64]) -> usize {
    let mut best = 0;
    for i in 1..probabilities.len() {
        if probabilities[i] > probabilities[best] {
            best = i;
        }
    }

    best
}

/// probabilities[0] is P(yes) for noul.
#[must_use]
pub fn verdict(kind: &Kind, probabilities: &[f64], bands: Bands) -> String {
    match (kind, bands) {
        (Kind::Noul { .. }, Bands::Noul { yes_at, no_at }) => {
            let yes = probabilities[0];

            if yes >= yes_at && yes > no_at {
                "yes".into()
            } else if yes <= no_at && yes < yes_at {
                "no".into()
            } else {
                "unsure".into()
            }
        }
        (_, Bands::Level { min_confidence }) => {
            if confidence(kind, probabilities) >= min_confidence {
                kind.labels()[argmax(probabilities)].clone()
            } else {
                "unsure".into()
            }
        }
        // Noul bands on a choice or score question never decide.
        _ => "unsure".into(),
    }
}

/// First 16 hex chars of sha256(text).
#[must_use]
pub fn short_hash(text: &str) -> String {
    let mut hash = String::with_capacity(16);
    for byte in sha2::Sha256::digest(text.as_bytes()).iter().take(8) {
        write!(&mut hash, "{byte:02x}").expect("writing to String");
    }

    hash
}

/// Identity of the question's meaning: type + instructions + criteria (not the id).
#[must_use]
pub fn question_hash(question: &crate::question::Question) -> String {
    short_hash(
        &serde_json::to_string(&(&question.instructions, &question.kind))
            .expect("question serializes"),
    )
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
    pub probs: crate::Probs,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
            .map(|example| (example.probs[0], example.label == "yes"))
            .collect();
        let no: Vec<_> = examples
            .iter()
            .map(|example| (example.probs[0], example.label == "no"))
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
            .map(|example| {
                (
                    confidence(kind, &example.probs),
                    labels[argmax(&example.probs)] == example.label,
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
    for example in examples {
        let result = verdict(kind, &example.probs, bands);
        if result != "unsure" {
            decided += 1;
            if result == example.label {
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

    candidates.into_iter().find(|threshold| {
        let selected: Vec<_> = data
            .iter()
            .filter(|(value, _)| {
                if above {
                    value >= threshold
                } else {
                    value <= threshold
                }
            })
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
