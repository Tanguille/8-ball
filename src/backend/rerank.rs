use crate::{
    backend::{BackendError, Endpoint, Probs},
    question::{Kind, Question},
};
use serde_json::json;

pub async fn ask(ep: &Endpoint, state: &str, qs: &[Question]) -> Result<Vec<Probs>, BackendError> {
    if qs.iter().any(|q| !matches!(q.kind, Kind::Noul { .. })) {
        return Err(BackendError("rerank answers noul questions only".into()));
    }
    futures::future::try_join_all(qs.iter().map(|q| async move {
        let mut body = json!({"query":q.instructions,"documents":[state]});
        if let Some(model) = &ep.model {
            body["model"] = json!(model);
        }
        let response = ep.post_json("/v1/rerank", &body).await?;
        let score = response["results"][0]["relevance_score"]
            .as_f64()
            .ok_or_else(|| BackendError("missing relevance_score".into()))?;
        // The pinned reranker's live sanity response contains logits, including negative values.
        let p = 1.0 / (1.0 + (-score).exp());
        Ok(vec![p, 1.0 - p])
    }))
    .await
}
