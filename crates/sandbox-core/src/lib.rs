//! Fail-closed process isolation primitives.
#![deny(unsafe_code)]
#![warn(missing_docs)]

use sandbox_common::{ExecutionRequest, ExecutionResult, ResourceLimits, SandboxError};
use std::time::Instant;
use tracing::debug;

/// Resource limit validator and execution coordinator.
#[derive(Debug, Clone, Default)]
pub struct SandboxEngine;

impl SandboxEngine {
    /// Construct a new engine. No privileged setup is performed here.
    pub fn new() -> Self {
        Self
    }

    /// Validate a request before handing it to a language runtime.
    pub fn validate(&self, request: &ExecutionRequest) -> Result<(), SandboxError> {
        request.validate()
    }

    /// Build a timeout result for runtimes that are terminated by the supervisor.
    pub fn timeout_result(&self, request: &ExecutionRequest, started: Instant) -> ExecutionResult {
        ExecutionResult {
            id: request.id,
            exit_code: None,
            stdout: String::new(),
            stderr: "execution timed out".into(),
            timed_out: true,
            duration_ms: started.elapsed().as_millis(),
            completed_at: chrono::Utc::now(),
        }
    }

    /// Return the limits that a future OS isolation layer must apply.
    pub fn isolation_plan(&self, limits: &ResourceLimits) -> Result<IsolationPlan, SandboxError> {
        limits.validate()?;
        Ok(IsolationPlan {
            memory_bytes: limits.memory_bytes,
            process_count: limits.process_count,
            file_descriptors: limits.file_descriptors,
            network_enabled: false,
        })
    }
}

/// Explicit plan passed to a platform-specific supervisor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolationPlan {
    /// Memory ceiling.
    pub memory_bytes: u64,
    /// Process ceiling.
    pub process_count: u32,
    /// File descriptor ceiling.
    pub file_descriptors: u64,
    /// Whether network is enabled.
    pub network_enabled: bool,
}

/// Emit a structured record when an execution starts.
pub fn log_execution_start(request: &ExecutionRequest) {
    debug!(request_id = %request.id, language = %request.language, "sandbox execution started");
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_common::{ExecutionRequest, Language, NetworkPolicy};
    #[test]
    fn plan_is_network_fail_closed() {
        let r = ExecutionRequest {
            id: uuid::Uuid::new_v4(),
            language: Language::Wasm,
            code: "module".into(),
            limits: Default::default(),
            network: NetworkPolicy::Deny,
        };
        let plan = SandboxEngine::new().isolation_plan(&r.limits).unwrap();
        assert!(!plan.network_enabled);
    }
}
