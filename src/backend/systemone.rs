use serde_json::{Value, json};

use crate::{backend::BackendError, question::Kind};

pub async fn ask(
    endpoint: &crate::backend::Endpoint,
    state: &str,
    requested_questions: &[crate::question::Question],
) -> Result<Vec<crate::backend::Probs>, BackendError> {
    let mut questions = serde_json::Map::new();
    for question in requested_questions {
        let mut wire =
            json!({"type":question.kind.type_name(),"instructions":question.instructions});
        match &question.kind {
            Kind::Noul {
                criteria: Some(criteria),
            } => wire["criteria"] = serde_json::to_value(criteria).expect("criteria serializes"),
            Kind::Noul { criteria: None } => {}
            Kind::Choice { criteria } => {
                wire["criteria"] = Value::Object(
                    criteria
                        .iter()
                        .map(|(name, description)| {
                            (
                                name.clone(),
                                if description.is_empty() {
                                    Value::Null
                                } else {
                                    json!(description)
                                },
                            )
                        })
                        .collect(),
                );
            }
            Kind::Score { criteria } => wire["criteria"] = json!(criteria),
        }
        questions.insert(question.id.clone(), wire);
    }
    let mut body = json!({"state":state,"questions":questions});
    if let Some(model) = &endpoint.model {
        body["model"] = json!(model);
    }
    let response = endpoint.post_json("/v1/systemone", &body).await?;

    requested_questions
        .iter()
        .map(|question| {
            let answer = response["answers"]
                .get(&question.id)
                .ok_or_else(|| BackendError(format!("no answer for question {}", question.id)))?;
            let mut probabilities = match &question.kind {
                Kind::Noul { .. } => {
                    let probability = answer["noul"].as_f64().ok_or_else(|| {
                        BackendError(format!("no noul probability for question {}", question.id))
                    })?;

                    vec![probability, 1.0 - probability]
                }
                _ => question
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
            if probabilities
                .iter()
                .any(|probability| !probability.is_finite() || !(0.0..=1.0).contains(probability))
            {
                return Err(BackendError(
                    "probability must be finite and within [0,1]".into(),
                ));
            }
            let sum: f64 = probabilities.iter().sum();
            if sum == 0.0 {
                return Err(BackendError("probabilities sum to zero".into()));
            }
            if (sum - 1.0).abs() > 1e-6 {
                for probability in &mut probabilities {
                    *probability /= sum;
                }
            }

            Ok(probabilities)
        })
        .collect()
}
