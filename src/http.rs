use crate::service::{AskRequest, AskResponse, Service, ServiceError};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use std::sync::Arc;

pub fn router(service: Arc<Service>) -> Router {
    Router::new()
        .route("/v1/ask", post(ask))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(service)
}

async fn ask(
    State(service): State<Arc<Service>>,
    Json(req): Json<AskRequest>,
) -> Result<Json<AskResponse>, ServiceError> {
    service.ask(req).await.map(Json)
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Backend { .. } => StatusCode::BAD_GATEWAY,
            Self::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({"error":self.to_string()}))).into_response()
    }
}
