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
