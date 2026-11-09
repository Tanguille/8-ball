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

use crate::{Service, ServiceError};

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
        .route("/v1/questions", routing::post(questions))
        .route("/v1/decisions", routing::post(decisions))
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
    Json(request): Json<crate::AskRequest>,
) -> Result<Json<crate::AskResponse>, ServiceError> {
    service.ask(request).await.map(Json)
}

async fn questions(
    State(service): State<Arc<Service>>,
    Json(request): Json<crate::QuestionsRequest>,
) -> Result<Json<crate::QuestionsResponse>, ServiceError> {
    service.questions(request).await.map(Json)
}

async fn decisions(
    State(service): State<Arc<Service>>,
    Json(request): Json<crate::DecisionsRequest>,
) -> Result<Json<crate::DecisionsResponse>, ServiceError> {
    service.decisions(request).await.map(Json)
}

async fn triage(
    State(service): State<Arc<Service>>,
    Json(request): Json<crate::TriageRequest>,
) -> Result<Json<crate::TriageResponse>, ServiceError> {
    service.triage(request).await.map(Json)
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
    Json(request): Json<crate::LabelRequest>,
) -> Result<Json<crate::LabelResponse>, ServiceError> {
    service.label(request.id, request.value).await.map(Json)
}
async fn calibrate(
    State(service): State<Arc<Service>>,
    Json(request): Json<CalibrateRequest>,
) -> Result<Json<crate::CalibrateResponse>, ServiceError> {
    service
        .calibrate(&request.spec, request.precision, request.dry_run)
        .await
        .map(Json)
}

async fn doctor(State(service): State<Arc<Service>>) -> Json<crate::DoctorReport> {
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
        // Without a key only local clients may call, as for MCP. Checking Host stops a web page from reaching this
        // server through DNS rebinding and reading stored decisions.
        return if local_host(&request) {
            next.run(request).await
        } else {
            error(
                StatusCode::FORBIDDEN,
                "without EIGHTBALL_API_KEY only localhost may call this; set a key to serve other hosts",
            )
        };
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
/// The Host header (or the HTTP/2 authority) names `localhost`, `127.0.0.1` or `::1`, the hosts rmcp allows by default.
fn local_host(request: &axum::extract::Request) -> bool {
    request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|host| axum::http::uri::Authority::try_from(host).ok())
        .or_else(|| request.uri().authority().cloned())
        .is_some_and(|authority| {
            matches!(
                authority
                    .host()
                    .trim_matches(['[', ']'])
                    .to_ascii_lowercase()
                    .as_str(),
                "localhost" | "127.0.0.1" | "::1"
            )
        })
}
async fn no_oauth() -> Response {
    error(
        StatusCode::NOT_FOUND,
        "8ball does not use OAuth; send Authorization: Bearer <EIGHTBALL_API_KEY>",
    )
}
