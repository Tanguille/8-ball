use crate::mcp::Mcp;
use crate::service::{
    AskRequest, AskResponse, DoctorReport, Service, ServiceError, TriageRequest, TriageResponse,
};
use axum::{
    Json, Router,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::json;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub fn router(service: Arc<Service>, api_key: Option<&str>, ct: &CancellationToken) -> Router {
    let mut cfg = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_sse_keep_alive(None)
        .with_cancellation_token(ct.child_token());
    if api_key.is_some() {
        cfg = cfg.disable_allowed_hosts();
    }
    let handler = Mcp::new(service.clone());
    let mcp = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        cfg,
    );
    let protected = Router::new()
        .nest_service("/mcp", mcp)
        .route("/v1/ask", post(ask))
        .route("/v1/triage", post(triage))
        .route("/v1/doctor", get(doctor))
        .route("/v1/label", post(label))
        .route("/v1/calibrate", post(calibrate))
        .layer(middleware::from_fn_with_state(
            api_key.map(Arc::<str>::from),
            require_bearer,
        ));
    Router::new()
        .merge(protected)
        .route("/healthz", get(|| async { "ok" }))
        .route("/metrics", get(metrics))
        .route("/register", post(no_oauth))
        .route("/.well-known/{*rest}", get(no_oauth))
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

async fn metrics(State(service): State<Arc<Service>>) -> Result<impl IntoResponse, ServiceError> {
    service.metrics().map(|body| {
        (
            [(
                header::CONTENT_TYPE,
                "application/openmetrics-text; version=1.0.0; charset=utf-8",
            )],
            body,
        )
    })
}

async fn require_bearer(State(key): State<Option<Arc<str>>>, req: Request, next: Next) -> Response {
    let Some(key) = key else {
        return next.run(req).await;
    };
    let given = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let ok = given.is_some_and(|token| {
        bool::from(subtle::ConstantTimeEq::ct_eq(
            token.as_bytes(),
            key.as_bytes(),
        ))
    });
    if ok {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"missing or wrong bearer token"})),
        )
            .into_response()
    }
}
async fn no_oauth() -> Response {
    (StatusCode::NOT_FOUND,Json(json!({"error":"8ball does not use OAuth; send Authorization: Bearer <EIGHTBALL_API_KEY>"}))).into_response()
}
