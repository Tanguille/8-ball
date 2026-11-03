use crate::{backend::BackendError, config::EmbedderConfig};
use serde_json::{Value, json};
use std::time::Duration;

pub struct Embedder {
    url: String,
    model: String,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl Embedder {
    #[must_use]
    pub fn new(cfg: &EmbedderConfig, api_key: Option<String>) -> Self {
        Self {
            url: cfg.url.clone(),
            model: cfg.model.clone(),
            api_key,
            client: reqwest::Client::new(),
        }
    }

    /// Batches of 8, one batch at a time (shared iGPU). 10 s timeout per batch.
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, BackendError> {
        let mut vectors = Vec::with_capacity(texts.len());
        let mut dimensions = None;
        for batch in texts.chunks(8) {
            let mut request = self
                .client
                .post(format!("{}/v1/embeddings", self.url))
                .timeout(Duration::from_secs(10))
                .json(&json!({"model":self.model,"input":batch}));
            if let Some(key) = &self.api_key {
                request = request.bearer_auth(key);
            }
            let response: Value = request.send().await?.error_for_status()?.json().await?;
            let data = response["data"]
                .as_array()
                .ok_or_else(|| BackendError("embedder returned no data".into()))?;
            let mut ordered = vec![None; batch.len()];
            for entry in data {
                let index = entry["index"]
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .filter(|i| *i < batch.len())
                    .ok_or_else(|| BackendError("embedder returned invalid index".into()))?;
                let vector: Vec<f32> = serde_json::from_value(entry["embedding"].clone())
                    .map_err(|e| BackendError(e.to_string()))?;
                if dimensions.is_some_and(|len| len != vector.len()) {
                    return Err(BackendError("embedder returned mixed dimensions".into()));
                }
                dimensions = Some(vector.len());
                if ordered[index].replace(vector).is_some() {
                    return Err(BackendError("embedder returned duplicate index".into()));
                }
            }
            for vector in ordered {
                vectors.push(
                    vector.ok_or_else(|| BackendError("embedder returned missing index".into()))?,
                );
            }
        }
        Ok(vectors)
    }
}

#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(a, b)| a * b).sum();
    let norm =
        a.iter().map(|v| v * v).sum::<f32>().sqrt() * b.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 { dot / norm } else { 0.0 }
}
