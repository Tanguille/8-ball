use crate::{
    backend::{BackendError, Endpoint, Probs},
    question::{Kind, Question},
};
use serde_json::{Value, json};

pub async fn ask(ep: &Endpoint, state: &str, qs: &[Question]) -> Result<Vec<Probs>, BackendError> {
    let mut questions = serde_json::Map::new();
    for q in qs {
        let mut wire = json!({"type":q.kind.type_name(),"instructions":q.instructions});
        match &q.kind {
            Kind::Noul {
                criteria: Some(criteria),
            } => wire["criteria"] = serde_json::to_value(criteria).expect("criteria serializes"),
            Kind::Noul { criteria: None } => {}
            Kind::Choice { criteria } => {
                wire["criteria"] = Value::Object(
                    criteria
                        .iter()
                        .map(|(name, desc)| {
                            (
                                name.clone(),
                                if desc.is_empty() {
                                    Value::Null
                                } else {
                                    json!(desc)
                                },
                            )
                        })
                        .collect(),
                );
            }
            Kind::Score { criteria } => wire["criteria"] = json!(criteria),
        }
        questions.insert(q.id.clone(), wire);
    }
    let mut body = json!({"state":state,"questions":questions});
    if let Some(model) = &ep.model {
        body["model"] = json!(model);
    }
    let response = ep.post_json("/v1/systemone", &body).await?;
    qs.iter()
        .map(|q| {
            let answer = response["answers"]
                .get(&q.id)
                .ok_or_else(|| BackendError(format!("no answer for question {}", q.id)))?;
            let mut probs = match &q.kind {
                Kind::Noul { .. } => {
                    let p = answer["noul"].as_f64().ok_or_else(|| {
                        BackendError(format!("no noul probability for question {}", q.id))
                    })?;
                    vec![p, 1.0 - p]
                }
                _ => q
                    .kind
                    .labels()
                    .iter()
                    .map(|label| {
                        answer["probabilities"][label].as_f64().ok_or_else(|| {
                            BackendError(format!("no probability for option {label}"))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            };
            let sum: f64 = probs.iter().sum();
            if sum == 0.0 {
                return Err(BackendError("probabilities sum to zero".into()));
            }
            if (sum - 1.0).abs() > 1e-6 {
                for p in &mut probs {
                    *p /= sum;
                }
            }
            Ok(probs)
        })
        .collect()
}
