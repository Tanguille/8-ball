use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing,
};
use serde_json::json;

use crate::service::{Service, ServiceError};

pub fn router(
    service: Arc<Service>,
    api_key: Option<&str>,
    cancellation_token: &tokio_util::sync::CancellationToken,
) -> Router {
    let mut configuration =
        rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_sse_keep_alive(None)
            .with_cancellation_token(cancellation_token.child_token());
    if api_key.is_some() {
        configuration = configuration.disable_allowed_hosts();
    }
    let handler = crate::mcp::Mcp::new(service.clone());
    let mcp = rmcp::transport::streamable_http_server::StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(
            rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(),
        ),
        configuration,
    );
    let protected = Router::new()
        .nest_service("/mcp", mcp)
        .route("/v1/ask", routing::post(ask))
        .route("/v1/triage", routing::post(triage))
        .route("/v1/doctor", routing::get(doctor))
        .route("/v1/label", routing::post(label))
        .route("/v1/calibrate", routing::post(calibrate))
        .layer(middleware::from_fn_with_state(
            api_key.map(Arc::<str>::from),
            require_bearer,
        ));

    Router::new()
        .merge(protected)
        .route("/healthz", routing::get(|| async { "ok" }))
        .route("/metrics", routing::get(metrics))
        .route("/register", routing::post(no_oauth))
        .route("/.well-known/{*rest}", routing::get(no_oauth))
        .with_state(service)
}

async fn ask(
    State(service): State<Arc<Service>>,
    Json(request): Json<crate::service::AskRequest>,
) -> Result<Json<crate::service::AskResponse>, ServiceError> {
    service.ask(request).await.map(Json)
}

async fn triage(
    State(service): State<Arc<Service>>,
    Json(request): Json<crate::service::TriageRequest>,
) -> Result<Json<crate::service::TriageResponse>, ServiceError> {
    service.triage(request).await.map(Json)
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
    Json(request): Json<LabelRequest>,
) -> Result<Json<crate::service::LabelResponse>, ServiceError> {
    service.label(request.id, request.value).await.map(Json)
}
async fn calibrate(
    State(service): State<Arc<Service>>,
    Json(request): Json<CalibrateRequest>,
) -> Result<Json<crate::service::CalibrateResponse>, ServiceError> {
    service
        .calibrate(&request.spec, request.precision, request.dry_run)
        .await
        .map(Json)
}

async fn doctor(State(service): State<Arc<Service>>) -> Json<crate::service::DoctorReport> {
    Json(service.doctor().await)
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Backend { .. } => StatusCode::BAD_GATEWAY,
            Self::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        error(status, &self.to_string())
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

async fn metrics(State(service): State<Arc<Service>>) -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/openmetrics-text; version=1.0.0; charset=utf-8",
        )],
        service.metrics(),
    )
}

async fn require_bearer(
    State(key): State<Option<Arc<str>>>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> Response {
    let Some(key) = key else {
        return next.run(request).await;
    };
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let authorized = given.is_some_and(|token| {
        bool::from(subtle::ConstantTimeEq::ct_eq(
            token.as_bytes(),
            key.as_bytes(),
        ))
    });

    if authorized {
        next.run(request).await
    } else {
        error(StatusCode::UNAUTHORIZED, "missing or wrong bearer token")
    }
}
async fn no_oauth() -> Response {
    error(
        StatusCode::NOT_FOUND,
        "8ball does not use OAuth; send Authorization: Bearer <EIGHTBALL_API_KEY>",
    )
}
