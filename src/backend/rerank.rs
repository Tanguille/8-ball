use serde_json::json;

use crate::backend::BackendError;

pub async fn ask(
    endpoint: &crate::backend::Endpoint,
    state: &str,
    questions: &[crate::question::Question],
) -> Result<Vec<crate::backend::Probs>, BackendError> {
    if questions
        .iter()
        .any(|question| !matches!(question.kind, crate::question::Kind::Noul { .. }))
    {
        return Err(BackendError("rerank answers noul questions only".into()));
    }

    futures::future::try_join_all(questions.iter().map(|question| async move {
        let body = json!({"query":question.instructions,"documents":[state]});
        let response = endpoint.post_json("/v1/rerank", body).await?;
        let score = response["results"][0]["relevance_score"]
            .as_f64()
            .ok_or_else(|| BackendError("missing relevance_score".into()))?;
        // The pinned reranker's live sanity response contains logits, including negative values.
        let probability = 1.0 / (1.0 + (-score).exp());

        Ok(vec![probability, 1.0 - probability])
    }))
    .await
}
