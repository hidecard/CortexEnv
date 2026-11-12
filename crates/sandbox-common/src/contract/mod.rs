//! Versioned API v1 execution contract.
#![deny(unsafe_code)]
#![warn(missing_docs)]

use crate::{ExecutionRequest, ExecutionResult, Language, NetworkPolicy, ResourceLimits};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod validation;

/// The first frozen wire-contract version.
pub const EXECUTION_CONTRACT_V1: &str = "v1";

/// User-facing execution mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Data analysis intent; the current executor runs the supplied code directly.
    Analyze,
    /// A bounded user or automation script.
    Script,
}

impl Default for ExecutionMode {
    fn default() -> Self {
        Self::Analyze
    }
}

/// Where the execution is scheduled.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTarget {
    /// Run on the local CortexEnv node.
    Local,
    /// Dispatch to a future remote worker.
    Remote,
}

impl Default for ExecutionTarget {
    fn default() -> Self {
        Self::Local
    }
}

/// A logical workspace owned by the submitting principal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceRef {
    /// Stable workspace identifier.
    pub id: String,
}

/// A requested output artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRequest {
    /// Relative path inside the execution workspace.
    pub path: String,
    /// Expected media type, such as `text/csv` or `image/png`.
    pub media_type: String,
}

/// An artifact returned by a completed execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactManifestEntry {
    /// Relative artifact path.
    pub path: String,
    /// Media type supplied by the runtime or validated request.
    pub media_type: String,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Lowercase SHA-256 digest of the artifact bytes.
    pub sha256: String,
}

/// Stable lifecycle states for an asynchronous job.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Accepted but not yet started.
    Queued,
    /// Supervisor resources are being prepared.
    Starting,
    /// Runtime is executing.
    Running,
    /// Runtime returned a result.
    Completed,
    /// Execution failed before a successful result.
    Failed,
    /// Deadline was exceeded.
    TimedOut,
    /// User or operator requested cancellation.
    Cancelled,
    /// Retention or expiration policy removed the job.
    Expired,
}

/// Stable machine-readable error categories for clients.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Request shape or value is invalid.
    InvalidRequest,
    /// Authentication or authorization failed.
    Unauthorized,
    /// The requested runtime or feature is unavailable.
    Unsupported,
    /// Admission or resource quota is exhausted.
    ResourceLimit,
    /// Execution exceeded its deadline.
    TimedOut,
    /// Required isolation or policy control is unavailable.
    IsolationUnavailable,
    /// Runtime rejected or could not execute the payload.
    RuntimeFailure,
    /// The job or artifact is not visible to this principal.
    NotFound,
    /// Unexpected service failure; details must not be exposed.
    Internal,
}

/// A stable error envelope returned by API v1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorEnvelope {
    /// Stable error category.
    pub code: ErrorCode,
    /// Safe client-facing message.
    pub message: String,
    /// Whether retrying the same request may succeed.
    pub retryable: bool,
}

/// Versioned request schema for API v1.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionRequestV1 {
    /// Frozen wire version, currently `v1`.
    pub schema_version: String,
    /// Stable request identifier.
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    /// Runtime selected for this request.
    pub runtime: Language,
    /// Optional runtime version/profile pin.
    #[serde(default)]
    pub runtime_version: Option<String>,
    /// User-facing execution mode.
    #[serde(default)]
    pub mode: ExecutionMode,
    /// Optional runtime entrypoint.
    #[serde(default)]
    pub entrypoint: Option<String>,
    /// Optional principal-owned workspace.
    #[serde(default)]
    pub workspace: Option<WorkspaceRef>,
    /// Python/JavaScript source or hex-encoded WASM module.
    pub code: String,
    /// Resource ceilings.
    #[serde(default)]
    pub resources: ResourceLimits,
    /// Network policy.
    #[serde(default)]
    pub network: NetworkPolicy,
    /// Artifacts requested from the workspace.
    #[serde(default)]
    pub artifacts: Vec<ArtifactRequest>,
    /// Local or future remote target.
    #[serde(default)]
    pub target: ExecutionTarget,
}

impl From<ExecutionRequest> for ExecutionRequestV1 {
    fn from(request: ExecutionRequest) -> Self {
        Self {
            schema_version: EXECUTION_CONTRACT_V1.into(),
            id: request.id,
            runtime: request.language,
            runtime_version: None,
            mode: ExecutionMode::Script,
            entrypoint: None,
            workspace: None,
            code: request.code,
            resources: request.limits,
            network: request.network,
            artifacts: Vec::new(),
            target: ExecutionTarget::Local,
        }
    }
}

/// Versioned response schema for a completed or failed execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionResultV1 {
    /// Frozen wire version, currently `v1`.
    pub schema_version: String,
    /// Lifecycle state of the job.
    pub state: JobState,
    /// Result when the job completed or timed out.
    #[serde(default)]
    pub result: Option<ExecutionResult>,
    /// Safe error when the job failed.
    #[serde(default)]
    pub error: Option<ErrorEnvelope>,
    /// Validated output artifacts.
    #[serde(default)]
    pub artifacts: Vec<ArtifactManifestEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_request_round_trips_and_defaults_safely() {
        let request = ExecutionRequestV1 {
            schema_version: EXECUTION_CONTRACT_V1.into(),
            id: Uuid::new_v4(),
            runtime: Language::Python,
            runtime_version: None,
            mode: ExecutionMode::Analyze,
            entrypoint: None,
            workspace: None,
            code: "print(1)".into(),
            resources: ResourceLimits::default(),
            network: NetworkPolicy::default(),
            artifacts: Vec::new(),
            target: ExecutionTarget::Local,
        };
        let json = serde_json::to_string(&request).unwrap();
        let decoded: ExecutionRequestV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, request);
        assert!(decoded.validate().is_ok());
    }

    #[test]
    fn legacy_request_maps_to_frozen_v1_contract() {
        let legacy = ExecutionRequest {
            id: Uuid::new_v4(),
            language: Language::Wasm,
            code: "0061736d01000000".into(),
            limits: ResourceLimits::default(),
            network: NetworkPolicy::Deny,
        };
        let versioned = ExecutionRequestV1::from(legacy);
        assert_eq!(versioned.schema_version, EXECUTION_CONTRACT_V1);
        assert_eq!(versioned.mode, ExecutionMode::Script);
        assert_eq!(versioned.target, ExecutionTarget::Local);
    }

    #[test]
    fn invalid_version_and_remote_target_fail_closed() {
        let mut request = ExecutionRequestV1::from(ExecutionRequest {
            id: Uuid::new_v4(),
            language: Language::Python,
            code: "print(1)".into(),
            limits: ResourceLimits::default(),
            network: NetworkPolicy::Deny,
        });
        request.schema_version = "v2".into();
        assert!(request.validate().is_err());
        request.schema_version = EXECUTION_CONTRACT_V1.into();
        request.target = ExecutionTarget::Remote;
        assert!(request.validate().is_err());
    }
}
