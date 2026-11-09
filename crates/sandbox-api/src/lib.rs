//! Axum API for synchronous and asynchronous sandbox execution.
#![deny(unsafe_code)]
#![warn(missing_docs)]
use axum::{
    extract::{Path, State},
    http::{header::HeaderName, StatusCode},
    routing::{get, post},
    Json, Router,
};
use dashmap::DashMap;
use sandbox_common::{ExecutionRequest, ExecutionResult};
use sandbox_runtime::Runtime;
use serde::Serialize;
use std::sync::Arc;
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
    let state = AppState {
        runtime,
        executions: Arc::new(DashMap::new()),
    };
    let rate_limit = GovernorConfigBuilder::default()
        .per_second(10)
        .burst_size(20)
        .finish()
        .expect("valid rate-limit configuration");
    Router::new()
        .route("/health", get(health))
        .route("/v1/execute", post(execute))
        .route("/v1/execute/async", post(execute_async))
        .route("/v1/executions/:id", get(get_execution))
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BYTES))
        .layer(SetRequestIdLayer::new(
            REQUEST_ID_HEADER.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER.clone()))
        .layer(GovernorLayer {
            config: Arc::new(rate_limit),
        })
        .with_state(state)
}
async fn health() -> &'static str {
    "ok"
}
async fn execute(
    State(state): State<AppState>,
    Json(request): Json<ExecutionRequest>,
) -> Result<Json<ExecutionResult>, (StatusCode, String)> {
    state
        .runtime
        .execute(request)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}
async fn execute_async(
    State(state): State<AppState>,
    Json(request): Json<ExecutionRequest>,
) -> Result<(StatusCode, Json<Uuid>), (StatusCode, String)> {
    request
        .validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let id = request.id;
    state.executions.insert(id, JobStatus::Running);
    let runtime = state.runtime.clone();
    let executions = state.executions.clone();
    tokio::spawn(async move {
        let status = match runtime.execute(request).await {
            Ok(result) => JobStatus::Completed { result },
            Err(error) => JobStatus::Failed {
                error: error.to_string(),
            },
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
}
