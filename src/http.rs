use crate::service::{
    AskRequest, AskResponse, DoctorReport, Service, ServiceError, TriageRequest, TriageResponse,
};
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
        .route("/v1/triage", post(triage))
        .route("/v1/doctor", get(doctor))
        .route("/v1/label", post(label))
        .route("/v1/calibrate", post(calibrate))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(service)
}

async fn ask(
    State(service): State<Arc<Service>>,
    Json(req): Json<AskRequest>,
) -> Result<Json<AskResponse>, ServiceError> {
    service.ask(req).await.map(Json)
}

async fn triage(
    State(service): State<Arc<Service>>,
    Json(req): Json<TriageRequest>,
) -> Result<Json<TriageResponse>, ServiceError> {
    service.triage(req).await.map(Json)
}

#[derive(serde::Deserialize)]
struct LabelRequest {
    id: i64,
    value: String,
}
#[derive(serde::Deserialize)]
struct CalibrateRequest {
    spec: String,
    precision: Option<f64>,
    #[serde(default)]
    dry_run: bool,
}

async fn label(
    State(service): State<Arc<Service>>,
    Json(req): Json<LabelRequest>,
) -> Result<Json<crate::service::LabelResponse>, ServiceError> {
    service.label(req.id, req.value).await.map(Json)
}
async fn calibrate(
    State(service): State<Arc<Service>>,
    Json(req): Json<CalibrateRequest>,
) -> Result<Json<crate::service::CalibrateResponse>, ServiceError> {
    service
        .calibrate(&req.spec, req.precision, req.dry_run)
        .await
        .map(Json)
}

async fn doctor(State(service): State<Arc<Service>>) -> Json<DoctorReport> {
    Json(service.doctor().await)
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
