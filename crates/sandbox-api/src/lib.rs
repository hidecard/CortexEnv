//! Axum API for synchronous and asynchronous sandbox execution.
#![deny(unsafe_code)]
#![warn(missing_docs)]
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension, Path, Query, State,
    },
    http::{header::HeaderName, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use dashmap::{mapref::entry::Entry, DashMap};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use sandbox_common::{ExecutionRequest, ExecutionResult, SandboxError};
use sandbox_runtime::Runtime;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
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
const DEFAULT_MAX_EXECUTIONS_PER_PRINCIPAL: usize = 2;
const MAX_EXECUTIONS_PER_PRINCIPAL: usize = 1024;
const DEFAULT_PRINCIPAL_REQUESTS_PER_SECOND: u32 = 5;
const DEFAULT_PRINCIPAL_REQUEST_BURST: u32 = 10;
const MAX_PRINCIPAL_REQUEST_RATE: u32 = 10_000;
const DEFAULT_MAX_PRINCIPAL_BUCKETS: usize = 10_000;
const PRINCIPAL_BUCKET_IDLE_TTL: Duration = Duration::from_secs(600);
const PRINCIPAL_BUCKET_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);
const MAX_PRINCIPAL_ID_BYTES: usize = 256;
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
    /// Optional HS256 JWT verification secret.
    pub jwt_secret: Option<Arc<String>>,
    /// Prometheus-compatible process and execution metrics.
    pub metrics: Arc<ApiMetrics>,
    /// Bounds the number of executions admitted at once.
    pub execution_slots: Arc<Semaphore>,
    /// Bounds concurrent executions for each authenticated principal.
    principal_execution_quota: Arc<PrincipalExecutionQuota>,
    /// Bounds request rate for each authenticated principal.
    principal_request_rate_limiter: Arc<PrincipalRequestRateLimiter>,
}

#[derive(Clone)]
struct PrincipalId(String);

struct PrincipalExecutionQuota {
    limit: usize,
    active: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl PrincipalExecutionQuota {
    fn new(limit: usize) -> Self {
        Self {
            limit: limit.clamp(1, MAX_EXECUTIONS_PER_PRINCIPAL),
            active: Mutex::new(HashMap::new()),
        }
    }

    fn acquire(
        self: &Arc<Self>,
        principal: &str,
    ) -> Result<PrincipalExecutionPermit, (StatusCode, String)> {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let semaphore = active
            .entry(principal.to_owned())
            .or_insert_with(|| Arc::new(Semaphore::new(self.limit)))
            .clone();
        let permit = semaphore.clone().try_acquire_owned().map_err(|_| {
            (
                StatusCode::TOO_MANY_REQUESTS,
                "per-principal execution quota is full".into(),
            )
        })?;
        Ok(PrincipalExecutionPermit {
            quota: Arc::clone(self),
            principal: principal.to_owned(),
            semaphore,
            permit: Some(permit),
        })
    }
}

struct PrincipalExecutionPermit {
    quota: Arc<PrincipalExecutionQuota>,
    principal: String,
    semaphore: Arc<Semaphore>,
    permit: Option<OwnedSemaphorePermit>,
}

impl Drop for PrincipalExecutionPermit {
    fn drop(&mut self) {
        drop(self.permit.take());
        let mut active = self
            .quota
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active.get(&self.principal).is_some_and(|current| {
            Arc::ptr_eq(current, &self.semaphore) && current.available_permits() == self.quota.limit
        }) {
            active.remove(&self.principal);
        }
    }
}

#[derive(Clone, Copy)]
struct PrincipalRequestRateConfig {
    requests_per_second: u32,
    burst: u32,
}

struct RequestTokenBucket {
    tokens: f64,
    updated_at: Instant,
    last_seen: Instant,
}

struct PrincipalRequestRateState {
    buckets: HashMap<String, RequestTokenBucket>,
    last_cleanup: Instant,
}

struct PrincipalRequestRateLimiter {
    requests_per_second: u32,
    burst: u32,
    max_principals: usize,
    idle_ttl: Duration,
    cleanup_interval: Duration,
    state: Mutex<PrincipalRequestRateState>,
}

impl PrincipalRequestRateLimiter {
    fn new(
        config: PrincipalRequestRateConfig,
        max_principals: usize,
        idle_ttl: Duration,
        cleanup_interval: Duration,
    ) -> Self {
        let now = Instant::now();
        Self {
            requests_per_second: config
                .requests_per_second
                .clamp(1, MAX_PRINCIPAL_REQUEST_RATE),
            burst: config.burst.clamp(1, MAX_PRINCIPAL_REQUEST_RATE),
            max_principals: max_principals.max(1),
            idle_ttl,
            cleanup_interval,
            state: Mutex::new(PrincipalRequestRateState {
                buckets: HashMap::new(),
                last_cleanup: now,
            }),
        }
    }

    fn check(&self, principal: &str) -> Result<(), u64> {
        self.check_at(principal, Instant::now())
    }

    fn check_at(&self, principal: &str, now: Instant) -> Result<(), u64> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if now.saturating_duration_since(state.last_cleanup) >= self.cleanup_interval {
            state.buckets.retain(|_, bucket| {
                now.saturating_duration_since(bucket.last_seen) < self.idle_ttl
            });
            state.last_cleanup = now;
        }

        if !state.buckets.contains_key(principal) && state.buckets.len() >= self.max_principals {
            return Err(self.idle_ttl.as_secs().max(1));
        }

        let bucket = state
            .buckets
            .entry(principal.to_owned())
            .or_insert_with(|| RequestTokenBucket {
                tokens: f64::from(self.burst),
                updated_at: now,
                last_seen: now,
            });
        let elapsed = now
            .saturating_duration_since(bucket.updated_at)
            .as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * f64::from(self.requests_per_second))
            .min(f64::from(self.burst));
        bucket.updated_at = now;
        bucket.last_seen = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let retry_after = ((1.0 - bucket.tokens) / f64::from(self.requests_per_second))
                .ceil()
                .max(1.0) as u64;
            Err(retry_after)
        }
    }
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
    router_with_auth_config(
        runtime,
        std::env::var("CORTEXENV_API_KEY").ok(),
        std::env::var("CORTEXENV_JWT_SECRET").ok(),
        configured_max_concurrent_executions(),
    )
}

/// Build the API router with an explicit optional API key.
pub fn router_with_api_key(runtime: Arc<Runtime>, api_key: Option<String>) -> Router {
    router_with_auth_config(
        runtime,
        api_key,
        None,
        configured_max_concurrent_executions(),
    )
}

fn configured_max_concurrent_executions() -> usize {
    std::env::var("CORTEXENV_MAX_CONCURRENT_EXECUTIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=MAX_CONCURRENT_EXECUTIONS).contains(value))
        .unwrap_or(DEFAULT_MAX_CONCURRENT_EXECUTIONS)
}

fn configured_max_executions_per_principal() -> usize {
    std::env::var("CORTEXENV_MAX_EXECUTIONS_PER_KEY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=MAX_EXECUTIONS_PER_PRINCIPAL).contains(value))
        .unwrap_or(DEFAULT_MAX_EXECUTIONS_PER_PRINCIPAL)
}

fn configured_principal_request_rate() -> PrincipalRequestRateConfig {
    let read_limit = |name: &str, default| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .filter(|value| (1..=MAX_PRINCIPAL_REQUEST_RATE).contains(value))
            .unwrap_or(default)
    };
    PrincipalRequestRateConfig {
        requests_per_second: read_limit(
            "CORTEXENV_PER_KEY_REQUESTS_PER_SECOND",
            DEFAULT_PRINCIPAL_REQUESTS_PER_SECOND,
        ),
        burst: read_limit(
            "CORTEXENV_PER_KEY_REQUEST_BURST",
            DEFAULT_PRINCIPAL_REQUEST_BURST,
        ),
    }
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
    router_with_auth_config(runtime, api_key, None, max_concurrent_executions)
}

/// Build the API router with optional API-key/JWT authentication and capacity.
///
/// JWTs must use HS256, include a non-empty `sub` claim and a valid `exp` claim;
/// the verification secret must contain at least 32 bytes.
pub fn router_with_auth_config(
    runtime: Arc<Runtime>,
    api_key: Option<String>,
    jwt_secret: Option<String>,
    max_concurrent_executions: usize,
) -> Router {
    router_with_auth_and_quota(
        runtime,
        api_key,
        jwt_secret,
        max_concurrent_executions,
        configured_max_executions_per_principal(),
        configured_principal_request_rate(),
    )
}

fn router_with_auth_and_quota(
    runtime: Arc<Runtime>,
    api_key: Option<String>,
    jwt_secret: Option<String>,
    max_concurrent_executions: usize,
    max_executions_per_principal: usize,
    principal_request_rate: PrincipalRequestRateConfig,
) -> Router {
    let api_key = api_key.filter(|secret| !secret.trim().is_empty());
    let jwt_secret = jwt_secret.filter(|secret| !secret.trim().is_empty());
    let max_stored_executions = std::env::var("CORTEXENV_MAX_STORED_EXECUTIONS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=MAX_STORED_EXECUTIONS).contains(value))
        .unwrap_or(DEFAULT_MAX_STORED_EXECUTIONS);
    if jwt_secret
        .as_ref()
        .is_some_and(|secret| secret.as_bytes().len() < 32)
    {
        tracing::error!(
            "CORTEXENV_JWT_SECRET must contain at least 32 bytes; JWT authentication will reject all tokens"
        );
    }
    let state = AppState {
        runtime,
        executions: Arc::new(ExecutionStore::new(max_stored_executions)),
        api_key: api_key.map(Arc::new),
        jwt_secret: jwt_secret.map(Arc::new),
        metrics: Arc::new(ApiMetrics::default()),
        execution_slots: Arc::new(Semaphore::new(
            max_concurrent_executions.clamp(1, MAX_CONCURRENT_EXECUTIONS),
        )),
        principal_execution_quota: Arc::new(PrincipalExecutionQuota::new(
            max_executions_per_principal,
        )),
        principal_request_rate_limiter: Arc::new(PrincipalRequestRateLimiter::new(
            principal_request_rate,
            DEFAULT_MAX_PRINCIPAL_BUCKETS,
            PRINCIPAL_BUCKET_IDLE_TTL,
            PRINCIPAL_BUCKET_CLEANUP_INTERVAL,
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
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    state.metrics.record_request();
    let exempt = matches!(request.uri().path(), "/health" | "/metrics");
    let auth_configured = state.api_key.is_some() || state.jwt_secret.is_some();
    let principal = if exempt || !auth_configured {
        None
    } else {
        let bearer = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        let header_key = request
            .headers()
            .get("x-api-key")
            .and_then(|value| value.to_str().ok());
        let matches_key = |candidate: Option<&str>, expected: &str| {
            candidate
                .map(|value| expected.as_bytes().ct_eq(value.as_bytes()).into())
                .unwrap_or(false)
        };
        let api_key_matches = state.api_key.as_ref().is_some_and(|expected| {
            matches_key(bearer, expected.as_str()) || matches_key(header_key, expected.as_str())
        });
        if api_key_matches {
            Some(PrincipalId("api-key".into()))
        } else {
            bearer
                .and_then(|token| {
                    state
                        .jwt_secret
                        .as_ref()
                        .and_then(|secret| jwt_subject(token, secret))
                })
                .map(PrincipalId)
        }
    };

    if auth_configured && !exempt && principal.is_none() {
        return (
            StatusCode::UNAUTHORIZED,
            [("www-authenticate", "Bearer")],
            "unauthorized",
        )
            .into_response();
    }

    if !exempt {
        if let Some(principal) = principal.as_ref() {
            if let Err(retry_after) = state.principal_request_rate_limiter.check(&principal.0) {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("retry-after", retry_after.to_string())],
                    "per-key request rate limit exceeded",
                )
                    .into_response();
            }
        }
    }

    request.extensions_mut().insert(principal);
    next.run(request).await
}

#[derive(Debug, Deserialize)]
struct JwtClaims {
    sub: String,
    exp: u64,
}

fn jwt_subject(token: &str, secret: &str) -> Option<String> {
    if token.len() > 8 * 1024 || secret.as_bytes().len() < 32 {
        return None;
    }
    let validation = Validation::new(Algorithm::HS256);
    decode::<JwtClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .ok()
    .and_then(|data| {
        (!data.claims.sub.trim().is_empty()
            && data.claims.sub.len() <= MAX_PRINCIPAL_ID_BYTES
            && data.claims.exp > 0)
            .then_some(data.claims.sub)
    })
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
    Extension(principal): Extension<Option<PrincipalId>>,
    Json(request): Json<ExecutionRequest>,
) -> Result<Json<ExecutionResult>, (StatusCode, String)> {
    let _principal_permit = acquire_principal_execution_slot(&state, principal.as_ref())?;
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
    Extension(principal): Extension<Option<PrincipalId>>,
    Json(request): Json<ExecutionRequest>,
) -> Result<(StatusCode, Json<Uuid>), (StatusCode, String)> {
    request
        .validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let principal_permit = acquire_principal_execution_slot(&state, principal.as_ref())?;
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
        let _principal_permit = principal_permit;
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

fn acquire_principal_execution_slot(
    state: &AppState,
    principal: Option<&PrincipalId>,
) -> Result<Option<PrincipalExecutionPermit>, (StatusCode, String)> {
    principal
        .map(|principal| state.principal_execution_quota.acquire(&principal.0))
        .transpose()
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
            jwt_secret: None,
            metrics: Arc::new(ApiMetrics::default()),
            execution_slots: Arc::new(Semaphore::new(1)),
            principal_execution_quota: Arc::new(PrincipalExecutionQuota::new(1)),
            principal_request_rate_limiter: Arc::new(PrincipalRequestRateLimiter::new(
                PrincipalRequestRateConfig {
                    requests_per_second: 10,
                    burst: 10,
                },
                10,
                Duration::from_secs(60),
                Duration::from_secs(1),
            )),
        };
        let _permit = acquire_execution_slot(&state).unwrap();
        let error = acquire_execution_slot(&state).unwrap_err();
        assert_eq!(error.0, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.1, "execution capacity is full");
    }

    #[test]
    fn principal_execution_quota_isolated_and_reclaims_idle_entries() {
        let quota = Arc::new(PrincipalExecutionQuota::new(1));
        let alice = quota.acquire("alice").unwrap();
        let rejected = match quota.acquire("alice") {
            Err(error) => error,
            Ok(_) => panic!("same-principal request must be rejected at quota"),
        };
        assert_eq!(rejected.0, StatusCode::TOO_MANY_REQUESTS);
        assert!(quota.acquire("bob").is_ok());

        drop(alice);
        assert!(!quota
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key("alice"));
        assert!(quota.acquire("alice").is_ok());
    }

    #[test]
    fn principal_rate_limiter_refills_isolates_and_reclaims_bounded_buckets() {
        let limiter = PrincipalRequestRateLimiter::new(
            PrincipalRequestRateConfig {
                requests_per_second: 1,
                burst: 1,
            },
            1,
            Duration::from_secs(10),
            Duration::from_secs(1),
        );
        let start = Instant::now();

        assert!(limiter.check_at("alice", start).is_ok());
        assert_eq!(limiter.check_at("alice", start), Err(1));
        assert_eq!(limiter.check_at("bob", start), Err(10));
        assert!(limiter
            .check_at("alice", start + Duration::from_secs(1))
            .is_ok());
        assert!(limiter
            .check_at("bob", start + Duration::from_secs(12))
            .is_ok());
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
    async fn blank_auth_secrets_do_not_enable_empty_credential_authentication() {
        let app = router_with_auth_config(
            Arc::new(Runtime::default()),
            Some("   ".into()),
            Some(String::new()),
            1,
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/executions/{}", Uuid::new_v4()))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn jwt_auth_requires_hs256_subject_and_unexpired_expiry() {
        #[derive(Serialize)]
        struct Claims<'a> {
            sub: &'a str,
            exp: u64,
        }

        let secret = "cortexenv-test-secret-that-is-at-least-32-bytes";
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = |exp| {
            jsonwebtoken::encode(
                &jsonwebtoken::Header::new(Algorithm::HS256),
                &Claims { sub: "tester", exp },
                &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
            )
            .unwrap()
        };
        assert_eq!(
            jwt_subject(&token(now + 3600), secret).as_deref(),
            Some("tester")
        );
        let oversized_subject = "x".repeat(MAX_PRINCIPAL_ID_BYTES + 1);
        let oversized_token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(Algorithm::HS256),
            &Claims {
                sub: &oversized_subject,
                exp: now + 3600,
            },
            &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap();
        assert!(jwt_subject(&oversized_token, secret).is_none());
        let app =
            router_with_auth_config(Arc::new(Runtime::default()), None, Some(secret.into()), 1);
        let job_path = format!("/v1/executions/{}", Uuid::new_v4());

        let valid = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&job_path)
                    .header("authorization", format!("Bearer {}", token(now + 3600)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(valid.status(), StatusCode::NOT_FOUND);

        let expired = app
            .oneshot(
                Request::builder()
                    .uri(&job_path)
                    .header("authorization", format!("Bearer {}", token(now - 3600)))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);
        assert!(jwt_subject(&token(now + 3600), "too-short").is_none());
    }

    #[tokio::test]
    async fn request_rate_limit_is_per_jwt_subject_and_returns_retry_after() {
        #[derive(Serialize)]
        struct Claims<'a> {
            sub: &'a str,
            exp: u64,
        }

        let secret = "cortexenv-test-secret-that-is-at-least-32-bytes";
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = |sub: &str| {
            jsonwebtoken::encode(
                &jsonwebtoken::Header::new(Algorithm::HS256),
                &Claims {
                    sub,
                    exp: now + 3600,
                },
                &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
            )
            .unwrap()
        };
        let app = router_with_auth_and_quota(
            Arc::new(Runtime::default()),
            None,
            Some(secret.into()),
            4,
            4,
            PrincipalRequestRateConfig {
                requests_per_second: 1,
                burst: 1,
            },
        );
        let request = |sub: &str| {
            Request::builder()
                .uri(format!("/v1/executions/{}", Uuid::new_v4()))
                .header("authorization", format!("Bearer {}", token(sub)))
                .body(Body::empty())
                .unwrap()
        };

        let first = app.clone().oneshot(request("alice")).await.unwrap();
        assert_eq!(first.status(), StatusCode::NOT_FOUND);
        let limited = app.clone().oneshot(request("alice")).await.unwrap();
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(limited.headers()["retry-after"], "1");
        let different_principal = app.oneshot(request("bob")).await.unwrap();
        assert_eq!(different_principal.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn async_execution_quota_is_per_jwt_subject_and_releases_on_completion() {
        #[derive(Serialize)]
        struct Claims<'a> {
            sub: &'a str,
            exp: u64,
        }

        let secret = "cortexenv-test-secret-that-is-at-least-32-bytes";
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = |sub: &str| {
            jsonwebtoken::encode(
                &jsonwebtoken::Header::new(Algorithm::HS256),
                &Claims {
                    sub,
                    exp: now + 3600,
                },
                &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
            )
            .unwrap()
        };
        let app = router_with_auth_and_quota(
            Arc::new(Runtime::default()),
            None,
            Some(secret.into()),
            4,
            1,
            PrincipalRequestRateConfig {
                requests_per_second: 100,
                burst: 100,
            },
        );
        let long_script = "const until = Date.now() + 1000; while (Date.now() < until) {}";
        let request = |sub: &str, code: &str| {
            let request = ExecutionRequest {
                id: Uuid::new_v4(),
                language: sandbox_common::Language::JavaScript,
                code: code.into(),
                limits: sandbox_common::ResourceLimits {
                    timeout_ms: 5000,
                    ..Default::default()
                },
                network: sandbox_common::NetworkPolicy::Deny,
            };
            Request::builder()
                .method("POST")
                .uri("/v1/execute/async")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", token(sub)))
                .body(Body::from(serde_json::to_vec(&request).unwrap()))
                .unwrap()
        };

        let first = app
            .clone()
            .oneshot(request("alice", long_script))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::ACCEPTED);

        let same_principal = app
            .clone()
            .oneshot(request("alice", "console.log('must be rejected');"))
            .await
            .unwrap();
        assert_eq!(same_principal.status(), StatusCode::TOO_MANY_REQUESTS);

        let different_principal = app
            .clone()
            .oneshot(request("bob", long_script))
            .await
            .unwrap();
        assert_eq!(different_principal.status(), StatusCode::ACCEPTED);

        tokio::time::sleep(Duration::from_millis(1500)).await;
        let after_completion = app
            .oneshot(request("alice", "console.log('slot reclaimed');"))
            .await
            .unwrap();
        assert_eq!(after_completion.status(), StatusCode::ACCEPTED);
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
