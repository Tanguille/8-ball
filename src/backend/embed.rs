use crate::backend::BackendError;

pub struct Embedder {
    url: String,
    model: String,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl Embedder {
    #[must_use]
    pub fn new(config: &crate::config::EmbedderConfig, api_key: Option<String>) -> Self {
        Self {
            url: config.url.clone(),
            model: config.model.clone(),
            api_key,
            client: reqwest::Client::new(),
        }
    }

    /// Batches of 8, one batch at a time (shared iGPU). 10 s timeout per batch.
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, BackendError> {
        let mut vectors = Vec::with_capacity(texts.len());
        let mut dimensions = None;
        for batch in texts.chunks(8) {
            let request = self
                .client
                .post(format!("{}/v1/embeddings", self.url))
                .timeout(std::time::Duration::from_secs(10))
                .json(&serde_json::json!({"model":self.model,"input":batch}));
            let response: serde_json::Value =
                crate::backend::authed(request, self.api_key.as_deref())
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
            let data = response["data"]
                .as_array()
                .ok_or_else(|| BackendError("embedder returned no data".into()))?;
            let mut ordered = vec![None; batch.len()];
            for entry in data {
                let index = entry["index"]
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok())
                    .filter(|index| *index < batch.len())
                    .ok_or_else(|| BackendError("embedder returned invalid index".into()))?;
                let vector: Vec<f32> = serde_json::from_value(entry["embedding"].clone())
                    .map_err(|e| BackendError(e.to_string()))?;
                if dimensions.is_some_and(|length| length != vector.len()) {
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
pub fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let dot: f32 = left
        .iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum();
    let norm = left.iter().map(|value| value * value).sum::<f32>().sqrt()
        * right.iter().map(|value| value * value).sum::<f32>().sqrt();

    if norm > 0.0 { dot / norm } else { 0.0 }
}
