//! Axum API for synchronous sandbox execution.
#![deny(unsafe_code)]
#![warn(missing_docs)]

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use sandbox_common::{ExecutionRequest, ExecutionResult};
use sandbox_runtime::Runtime;
use std::sync::Arc;

/// Shared API state.
#[derive(Clone)]
pub struct AppState {
    /// Runtime dispatcher.
    pub runtime: Arc<Runtime>,
}
/// Build the API router.
pub fn router(runtime: Arc<Runtime>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/execute", post(execute))
        .with_state(AppState { runtime })
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
