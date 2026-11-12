use serde_json::json;

use domain::question::Kind;

use crate::BackendError;

/// A System One server and a hosted decisions API differ in the route and in where the answers are.
pub async fn ask(
    endpoint: &crate::Endpoint,
    info: &crate::Info,
    state: &str,
    requested_questions: &[domain::question::Question],
) -> Result<Vec<domain::Probs>, BackendError> {
    let (path, answers) = if info.kind == crate::BackendKind::Decisions {
        ("/v1/decisions", "/result/answers")
    } else {
        ("/v1/systemone", "/answers")
    };
    let mut questions = serde_json::Map::new();
    for question in requested_questions {
        let mut wire = json!(question.kind);
        wire["instructions"] = json!(question.instructions);
        if let Kind::Choice { criteria } = &question.kind {
            for (name, description) in criteria {
                if description.is_empty() {
                    wire["criteria"][name] = serde_json::Value::Null;
                }
            }
        }
        questions.insert(question.id.clone(), wire);
    }
    let body = json!({"state":state,"questions":questions});
    let response = endpoint.post_json(info, path, body).await?;
    let response = response
        .pointer(answers)
        .ok_or_else(|| BackendError(format!("no {answers} in the response")))?;

    requested_questions
        .iter()
        .map(|question| {
            let answer = response
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
