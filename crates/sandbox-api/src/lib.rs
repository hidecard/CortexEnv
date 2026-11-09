//! Axum API for synchronous and asynchronous sandbox execution.
#![deny(unsafe_code)]
#![warn(missing_docs)]
use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, Path, Query, State},
    http::{header::HeaderName, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use dashmap::DashMap;
use futures_util::SinkExt;
use sandbox_common::{ExecutionRequest, ExecutionResult};
use sandbox_runtime::Runtime;
use serde::{Deserialize, Serialize};
use std::{
    sync::{atomic::{AtomicU64, Ordering}, Arc},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tower_http::{
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
};
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use uuid::Uuid;

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");
const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;

/// State shared by API handlers.
#[derive(Clone)]
pub struct AppState {
    /// Runtime dispatcher.
    pub runtime: Arc<Runtime>,
    /// In-memory asynchronous execution store.
    pub executions: Arc<DashMap<Uuid, JobStatus>>,
    /// Optional API key. When set, execution routes require authentication.
    pub api_key: Option<Arc<String>>,
    /// Prometheus-compatible process and execution metrics.
    pub metrics: Arc<ApiMetrics>,
}

/// Prometheus-compatible API counters and execution timing totals.
#[derive(Debug, Default)]
pub struct ApiMetrics {
    requests_total: AtomicU64,
    executions_started_total: AtomicU64,
    executions_completed_total: AtomicU64,
    executions_failed_total: AtomicU64,
    executions_timed_out_total: AtomicU64,
    execution_duration_ms_sum: AtomicU64,
    execution_duration_ms_count: AtomicU64,
}

impl ApiMetrics {
    fn record_request(&self) { self.requests_total.fetch_add(1, Ordering::Relaxed); }
    fn record_started(&self) { self.executions_started_total.fetch_add(1, Ordering::Relaxed); }
    fn record_result(&self, result: &ExecutionResult) {
        self.execution_duration_ms_sum.fetch_add(result.duration_ms, Ordering::Relaxed);
        self.execution_duration_ms_count.fetch_add(1, Ordering::Relaxed);
        if result.timed_out {
            self.executions_timed_out_total.fetch_add(1, Ordering::Relaxed);
        } else {
            self.executions_completed_total.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn record_failure(&self) { self.executions_failed_total.fetch_add(1, Ordering::Relaxed); }
    fn render(&self) -> String {
        format!(
            "# HELP cortexenv_api_requests_total Total API requests.\n# TYPE cortexenv_api_requests_total counter\ncortexenv_api_requests_total {}\n# HELP cortexenv_executions_started_total Total executions started.\n# TYPE cortexenv_executions_started_total counter\ncortexenv_executions_started_total {}\n# HELP cortexenv_executions_completed_total Total executions completed.\n# TYPE cortexenv_executions_completed_total counter\ncortexenv_executions_completed_total {}\n# HELP cortexenv_executions_failed_total Total executions that failed.\n# TYPE cortexenv_executions_failed_total counter\ncortexenv_executions_failed_total {}\n# HELP cortexenv_executions_timed_out_total Total executions that timed out.\n# TYPE cortexenv_executions_timed_out_total counter\ncortexenv_executions_timed_out_total {}\n# HELP cortexenv_execution_duration_ms_sum Sum of execution durations in milliseconds.\n# TYPE cortexenv_execution_duration_ms_sum counter\ncortexenv_execution_duration_ms_sum {}\n# HELP cortexenv_execution_duration_ms_count Number of completed execution duration observations.\n# TYPE cortexenv_execution_duration_ms_count counter\ncortexenv_execution_duration_ms_count {}\n",
            self.requests_total.load(Ordering::Relaxed), self.executions_started_total.load(Ordering::Relaxed), self.executions_completed_total.load(Ordering::Relaxed), self.executions_failed_total.load(Ordering::Relaxed), self.executions_timed_out_total.load(Ordering::Relaxed), self.execution_duration_ms_sum.load(Ordering::Relaxed), self.execution_duration_ms_count.load(Ordering::Relaxed)
        )
    }
}
/// State of an asynchronous execution.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum JobStatus {
    /// Execution is queued or running.
    Running,
    /// Execution completed.
    Completed { result: ExecutionResult },
    /// Execution failed validation or at runtime.
    Failed { error: String },
}
/// Build the API router.
pub fn router(runtime: Arc<Runtime>) -> Router {
    router_with_api_key(runtime, std::env::var("CORTEXENV_API_KEY").ok())
}

/// Build the API router with an explicit optional API key.
pub fn router_with_api_key(runtime: Arc<Runtime>, api_key: Option<String>) -> Router {
    let state = AppState {
        runtime,
        executions: Arc::new(DashMap::new()),
        api_key: api_key.map(Arc::new),
        metrics: Arc::new(ApiMetrics::default()),
    };
    let rate_limit = GovernorConfigBuilder::default()
        .per_second(10)
        .burst_size(20)
        .finish()
        .expect("valid rate-limit configuration");
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/v1/execute", post(execute))
        .route("/v1/execute/async", post(execute_async))
        .route("/v1/executions/:id", get(get_execution))
        .route("/v1/stream", get(stream))
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BYTES))
        .layer(SetRequestIdLayer::new(
            REQUEST_ID_HEADER.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER.clone()))
        .layer(GovernorLayer {
            config: Arc::new(rate_limit),
        })
        .layer(middleware::from_fn_with_state(
            state.clone(),
            authenticate,
        ))
        .with_state(state)
}

async fn authenticate(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    state.metrics.record_request();
    if matches!(request.uri().path(), "/health" | "/metrics") || state.api_key.is_none() {
        return next.run(request).await;
    }

    let expected = state.api_key.as_ref().expect("checked above");
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let header_key = request
        .headers()
        .get("x-api-key")
        .and_then(|value| value.to_str().ok());
    let matches_key = |candidate: Option<&str>| {
        candidate
            .map(|value| expected.as_bytes().ct_eq(value.as_bytes()).into())
            .unwrap_or(false)
    };
    if matches_key(bearer) || matches_key(header_key) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [("www-authenticate", "Bearer")],
            "unauthorized",
        )
            .into_response()
    }
}
async fn health() -> &'static str {
    "ok"
}
async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    ([("content-type", "text/plain; version=0.0.4; charset=utf-8")], state.metrics.render())
}
async fn execute(
    State(state): State<AppState>,
    Json(request): Json<ExecutionRequest>,
) -> Result<Json<ExecutionResult>, (StatusCode, String)> {
    state.metrics.record_started();
    let started = Instant::now();
    let result = state
        .runtime
        .execute(request)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()));
    match &result {
        Ok(result) => state.metrics.record_result(result),
        Err(_) => {
            state.metrics.record_failure();
            state.metrics.execution_duration_ms_sum.fetch_add(started.elapsed().as_millis() as u64, Ordering::Relaxed);
            state.metrics.execution_duration_ms_count.fetch_add(1, Ordering::Relaxed);
        }
    }
    result.map(Json)
}
async fn execute_async(
    State(state): State<AppState>,
    Json(request): Json<ExecutionRequest>,
) -> Result<(StatusCode, Json<Uuid>), (StatusCode, String)> {
    request
        .validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let id = request.id;
    state.metrics.record_started();
    state.executions.insert(id, JobStatus::Running);
    let runtime = state.runtime.clone();
    let executions = state.executions.clone();
    let metrics = state.metrics.clone();
    tokio::spawn(async move {
        let started = Instant::now();
        let status = match runtime.execute(request).await {
            Ok(result) => {
                metrics.record_result(&result);
                JobStatus::Completed { result }
            }
            Err(error) => {
                metrics.record_failure();
                metrics
                    .execution_duration_ms_sum
                    .fetch_add(started.elapsed().as_millis() as u64, Ordering::Relaxed);
                metrics.execution_duration_ms_count.fetch_add(1, Ordering::Relaxed);
                JobStatus::Failed {
                    error: error.to_string(),
                }
            }
        };
        executions.insert(id, status);
    });
    Ok((StatusCode::ACCEPTED, Json(id)))
}
async fn get_execution(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JobStatus>, StatusCode> {
    state
        .executions
        .get(&id)
        .map(|entry| Json(entry.value().clone()))
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Debug, Deserialize)]
struct StreamQuery {
    id: Uuid,
}

async fn stream(
    ws: WebSocketUpgrade,
    Query(query): Query<StreamQuery>,
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| stream_execution(socket, state, query.id))
}

async fn stream_execution(mut socket: WebSocket, state: AppState, id: Uuid) {
    loop {
        let Some(status) = state.executions.get(&id).map(|entry| entry.value().clone()) else {
            let _ = socket
                .send(Message::Text(
                    r#"{"status":"not_found"}"#.to_string().into(),
                ))
                .await;
            let _ = socket.close().await;
            return;
        };

        let terminal = matches!(&status, JobStatus::Completed { .. } | JobStatus::Failed { .. });
        let payload = match serde_json::to_string(&status) {
            Ok(payload) => payload,
            Err(_) => return,
        };
        if socket.send(Message::Text(payload.into())).await.is_err() {
            return;
        }
        if terminal {
            let _ = socket.close().await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_route_returns_request_id() {
        let app = router(Arc::new(Runtime::default()));
        let response = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key(REQUEST_ID_HEADER));
    }

    #[tokio::test]
    async fn async_route_rejects_invalid_requests_before_queueing() {
        let app = router(Arc::new(Runtime::default()));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/execute/async")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"language":"wasm","code":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn stream_requires_an_execution_id() {
        let app = router(Arc::new(Runtime::default()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn configured_api_key_protects_execution_routes() {
        let app = router_with_api_key(Arc::new(Runtime::default()), Some("secret".into()));
        let response = app
            .clone()
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(Request::builder().uri("/v1/executions/invalid").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/executions/invalid")
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn metrics_route_is_public_and_prometheus_compatible() {
        let app = router_with_api_key(Arc::new(Runtime::default()), Some("secret".into()));
        let response = app
            .oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/plain; version=0.0.4; charset=utf-8");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8(body.to_vec()).unwrap().contains("cortexenv_api_requests_total"));
    }
}
