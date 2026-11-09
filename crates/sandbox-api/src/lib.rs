//! Axum API for synchronous and asynchronous sandbox execution.
#![deny(unsafe_code)]
#![warn(missing_docs)]
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header::HeaderName, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use dashmap::{mapref::entry::Entry, DashMap};
use sandbox_common::{ExecutionRequest, ExecutionResult, SandboxError};
use sandbox_runtime::Runtime;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::GlobalKeyExtractor, GovernorLayer,
};
use tower_http::{
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
};
use uuid::Uuid;

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");
const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_CONCURRENT_EXECUTIONS: usize = 4;
const MAX_CONCURRENT_EXECUTIONS: usize = 1024;
const DEFAULT_MAX_STORED_EXECUTIONS: usize = 1_000;
const MAX_STORED_EXECUTIONS: usize = 100_000;

/// State shared by API handlers.
#[derive(Clone)]
pub struct AppState {
    /// Runtime dispatcher.
    pub runtime: Arc<Runtime>,
    /// In-memory asynchronous execution store.
    pub executions: Arc<ExecutionStore>,
    /// Optional API key. When set, execution routes require authentication.
    pub api_key: Option<Arc<String>>,
    /// Prometheus-compatible process and execution metrics.
    pub metrics: Arc<ApiMetrics>,
    /// Bounds the number of executions admitted at once.
    pub execution_slots: Arc<Semaphore>,
}

/// In-memory async job store that bounds retained completed results.
pub struct ExecutionStore {
    jobs: DashMap<Uuid, JobStatus>,
    completed_order: Mutex<VecDeque<Uuid>>,
    max_completed: usize,
}

impl ExecutionStore {
    /// Create a job store retaining between one and 100,000 completed jobs.
    pub fn new(max_completed: usize) -> Self {
        Self {
            jobs: DashMap::new(),
            completed_order: Mutex::new(VecDeque::new()),
            max_completed: max_completed.clamp(1, MAX_STORED_EXECUTIONS),
        }
    }

    fn insert_running(&self, id: Uuid) -> bool {
        match self.jobs.entry(id) {
            Entry::Occupied(_) => false,
            Entry::Vacant(entry) => {
                entry.insert(JobStatus::Running);
                true
            }
        }
    }

    fn finish(&self, id: Uuid, status: JobStatus) {
        self.jobs.insert(id, status);
        let mut completed_order = self
            .completed_order
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        completed_order.push_back(id);
        while completed_order.len() > self.max_completed {
            if let Some(expired_id) = completed_order.pop_front() {
                let is_completed = self.jobs.get(&expired_id).is_some_and(|entry| {
                    matches!(
                        entry.value(),
                        JobStatus::Completed { .. } | JobStatus::Failed { .. }
                    )
                });
                if is_completed {
                    self.jobs.remove(&expired_id);
                }
            }
        }
    }

    /// Return a snapshot of a stored job status, if it has not been evicted.
    pub fn get(&self, id: &Uuid) -> Option<JobStatus> {
        self.jobs.get(id).map(|entry| entry.value().clone())
    }
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
    fn record_request(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }
    fn record_started(&self) {
        self.executions_started_total
            .fetch_add(1, Ordering::Relaxed);
    }
    fn record_result(&self, result: &ExecutionResult) {
        self.execution_duration_ms_sum.fetch_add(
            result.duration_ms.min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
        self.execution_duration_ms_count
            .fetch_add(1, Ordering::Relaxed);
        if result.timed_out {
            self.executions_timed_out_total
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.executions_completed_total
                .fetch_add(1, Ordering::Relaxed);
        }
    }
    fn record_failure(&self) {
        self.executions_failed_total.fetch_add(1, Ordering::Relaxed);
    }
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
    let max_concurrent_executions = std::env::var("CORTEXENV_MAX_CONCURRENT_EXECUTIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=MAX_CONCURRENT_EXECUTIONS).contains(value))
        .unwrap_or(DEFAULT_MAX_CONCURRENT_EXECUTIONS);
    router_with_config(runtime, api_key, max_concurrent_executions)
}

/// Build the API router with explicit authentication and execution capacity.
///
/// The capacity is clamped to `1..=1024`. Requests arriving while every slot
/// is occupied are rejected with `503 Service Unavailable` rather than being
/// spawned into an unbounded background queue.
pub fn router_with_config(
    runtime: Arc<Runtime>,
    api_key: Option<String>,
    max_concurrent_executions: usize,
) -> Router {
    let max_stored_executions = std::env::var("CORTEXENV_MAX_STORED_EXECUTIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=MAX_STORED_EXECUTIONS).contains(value))
        .unwrap_or(DEFAULT_MAX_STORED_EXECUTIONS);
    let state = AppState {
        runtime,
        executions: Arc::new(ExecutionStore::new(max_stored_executions)),
        api_key: api_key.map(Arc::new),
        metrics: Arc::new(ApiMetrics::default()),
        execution_slots: Arc::new(Semaphore::new(
            max_concurrent_executions.clamp(1, MAX_CONCURRENT_EXECUTIONS),
        )),
    };
    let rate_limit = GovernorConfigBuilder::default()
        .per_second(10)
        .burst_size(20)
        .key_extractor(GlobalKeyExtractor)
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
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER.clone()))
        .layer(SetRequestIdLayer::new(
            REQUEST_ID_HEADER.clone(),
            MakeRequestUuid,
        ))
        .layer(GovernorLayer {
            config: Arc::new(rate_limit),
        })
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state)
}

async fn authenticate(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
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
    (
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        state.metrics.render(),
    )
}
async fn execute(
    State(state): State<AppState>,
    Json(request): Json<ExecutionRequest>,
) -> Result<Json<ExecutionResult>, (StatusCode, String)> {
    let _permit = acquire_execution_slot(&state)?;
    state.metrics.record_started();
    let started = Instant::now();
    let request_id = request.id;
    let result = state.runtime.execute(request).await.map_err(|error| {
        tracing::warn!(request_id = %request_id, error = %error, "sandbox execution failed");
        client_safe_error(&error)
    });
    match &result {
        Ok(result) => state.metrics.record_result(result),
        Err(_) => {
            state.metrics.record_failure();
            state
                .metrics
                .execution_duration_ms_sum
                .fetch_add(started.elapsed().as_millis() as u64, Ordering::Relaxed);
            state
                .metrics
                .execution_duration_ms_count
                .fetch_add(1, Ordering::Relaxed);
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
    let permit = acquire_execution_slot(&state)?;
    let id = request.id;
    if !state.executions.insert_running(id) {
        return Err((StatusCode::CONFLICT, "execution ID already exists".into()));
    }
    state.metrics.record_started();
    let runtime = state.runtime.clone();
    let executions = state.executions.clone();
    let metrics = state.metrics.clone();
    tokio::spawn(async move {
        let _permit = permit;
        let started = Instant::now();
        let status = match runtime.execute(request).await {
            Ok(result) => {
                metrics.record_result(&result);
                JobStatus::Completed { result }
            }
            Err(error) => {
                metrics.record_failure();
                tracing::warn!(request_id = %id, error = %error, "async sandbox execution failed");
                metrics
                    .execution_duration_ms_sum
                    .fetch_add(started.elapsed().as_millis() as u64, Ordering::Relaxed);
                metrics
                    .execution_duration_ms_count
                    .fetch_add(1, Ordering::Relaxed);
                JobStatus::Failed {
                    error: client_safe_error(&error).1,
                }
            }
        };
        executions.finish(id, status);
    });
    Ok((StatusCode::ACCEPTED, Json(id)))
}

fn client_safe_error(error: &SandboxError) -> (StatusCode, String) {
    match error {
        SandboxError::InvalidRequest(message) => (
            StatusCode::BAD_REQUEST,
            format!("invalid request: {message}"),
        ),
        SandboxError::UnsupportedLanguage(language) => (
            StatusCode::BAD_REQUEST,
            format!("unsupported language: {language}"),
        ),
        SandboxError::ResourceLimit(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "resource limits could not be applied".into(),
        ),
        SandboxError::Timeout => (StatusCode::REQUEST_TIMEOUT, "execution timed out".into()),
        SandboxError::Runtime(_) => (
            StatusCode::BAD_REQUEST,
            "the program could not be compiled or executed".into(),
        ),
        SandboxError::Isolation | SandboxError::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "execution failed due to a server error".into(),
        ),
    }
}

fn acquire_execution_slot(state: &AppState) -> Result<OwnedSemaphorePermit, (StatusCode, String)> {
    state
        .execution_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "execution capacity is full".into(),
            )
        })
}
async fn get_execution(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JobStatus>, StatusCode> {
    state
        .executions
        .get(&id)
        .map(Json)
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
        let Some(status) = state.executions.get(&id) else {
            let _ = socket
                .send(Message::Text(
                    r#"{"status":"not_found"}"#.to_string().into(),
                ))
                .await;
            let _ = socket.close().await;
            return;
        };

        let terminal = matches!(
            &status,
            JobStatus::Completed { .. } | JobStatus::Failed { .. }
        );
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
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
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
    async fn async_route_rejects_duplicate_execution_ids() {
        let app = router(Arc::new(Runtime::default()));
        let id = Uuid::new_v4();
        let body = serde_json::json!({
            "id": id,
            "language": "wasm",
            "code": "0061736d01000000"
        })
        .to_string();
        let request = || {
            Request::builder()
                .method("POST")
                .uri("/v1/execute/async")
                .header("content-type", "application/json")
                .body(Body::from(body.clone()))
                .unwrap()
        };

        let first = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(first.status(), StatusCode::ACCEPTED);
        let second = app.oneshot(request()).await.unwrap();
        assert_eq!(second.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn exhausted_execution_slots_fail_closed() {
        let state = AppState {
            runtime: Arc::new(Runtime::default()),
            executions: Arc::new(ExecutionStore::new(1)),
            api_key: None,
            metrics: Arc::new(ApiMetrics::default()),
            execution_slots: Arc::new(Semaphore::new(1)),
        };
        let _permit = acquire_execution_slot(&state).unwrap();
        let error = acquire_execution_slot(&state).unwrap_err();
        assert_eq!(error.0, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.1, "execution capacity is full");
    }

    #[test]
    fn execution_store_evicts_oldest_completed_and_keeps_running_jobs() {
        let store = ExecutionStore::new(2);
        let active_id = Uuid::new_v4();
        assert!(store.insert_running(active_id));

        let completed_ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        for id in completed_ids {
            assert!(store.insert_running(id));
            store.finish(
                id,
                JobStatus::Failed {
                    error: "test failure".into(),
                },
            );
        }

        assert!(matches!(store.get(&active_id), Some(JobStatus::Running)));
        assert!(store.get(&completed_ids[0]).is_none());
        assert!(store.get(&completed_ids[1]).is_some());
        assert!(store.get(&completed_ids[2]).is_some());
        assert!(!store.insert_running(completed_ids[2]));
    }

    #[tokio::test]
    async fn runtime_errors_do_not_leak_internal_details() {
        let app = router(Arc::new(Runtime::default()));
        let request = ExecutionRequest {
            id: Uuid::new_v4(),
            language: sandbox_common::Language::Wasm,
            code: "00".into(),
            limits: Default::default(),
            network: Default::default(),
        };
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/execute")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let message = String::from_utf8(body.to_vec()).unwrap();
        assert!(message.contains("the program could not be compiled or executed"));
        assert!(!message.contains("failed to parse"));
    }

    #[test]
    fn internal_error_messages_are_generic_but_client_validation_stays_actionable() {
        let (status, message) = client_safe_error(&SandboxError::Runtime(
            "host path /srv/private/cortexenv".into(),
        ));
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(message, "the program could not be compiled or executed");
        assert!(!message.contains("/srv/private"));

        let (status, message) = client_safe_error(&SandboxError::InvalidRequest(
            "timeout_ms must be between 1 and 300000".into(),
        ));
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            message,
            "invalid request: timeout_ms must be between 1 and 300000"
        );
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
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/executions/invalid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/executions/{}", Uuid::new_v4()))
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
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["content-type"],
            "text/plain; version=0.0.4; charset=utf-8"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8(body.to_vec())
            .unwrap()
            .contains("cortexenv_api_requests_total"));
    }
}
