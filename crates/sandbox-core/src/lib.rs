//! Fail-closed process isolation primitives.
#![deny(unsafe_code)]
#![warn(missing_docs)]
use sandbox_common::{ExecutionRequest, ExecutionResult, ResourceLimits, SandboxError};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use tempfile::{tempdir, TempDir};
use tracing::debug;

/// Linux-only cgroup v2 and child-supervisor isolation contracts.
#[cfg(target_os = "linux")]
pub mod linux;

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

/// A private per-execution directory removed automatically when dropped.
pub struct ExecutionWorkspace {
    root: TempDir,
}
impl std::fmt::Debug for ExecutionWorkspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecutionWorkspace")
            .field("root", &self.root.path())
            .finish()
    }
}
impl ExecutionWorkspace {
    /// Create a new isolated temporary workspace.
    pub fn new() -> Result<Self, SandboxError> {
        tempdir()
            .map(|root| Self { root })
            .map_err(|_| SandboxError::Isolation)
    }
    /// Return the workspace root.
    pub fn root(&self) -> &Path {
        self.root.path()
    }
    /// Resolve a relative path without allowing traversal outside the workspace.
    pub fn safe_path(&self, relative: impl AsRef<Path>) -> Result<PathBuf, SandboxError> {
        let relative = relative.as_ref();
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(SandboxError::InvalidRequest(
                "path must stay inside the execution workspace".into(),
            ));
        }
        let candidate = self.root().join(relative);
        if candidate.exists() {
            let canonical = candidate
                .canonicalize()
                .map_err(|_| SandboxError::Isolation)?;
            if !canonical.starts_with(self.root()) {
                return Err(SandboxError::InvalidRequest(
                    "path must stay inside the execution workspace".into(),
                ));
            }
            Ok(canonical)
        } else {
            Ok(candidate)
        }
    }
}

/// Explicit plan passed to a platform-specific supervisor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolationPlan {
    pub memory_bytes: u64,
    pub process_count: u32,
    pub file_descriptors: u64,
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
    #[test]
    fn workspace_rejects_traversal_and_cleans_up() {
        let path = {
            let workspace = ExecutionWorkspace::new().unwrap();
            let root = workspace.root().to_path_buf();
            assert!(workspace.safe_path("../outside").is_err());
            assert!(workspace
                .safe_path("nested/file.txt")
                .unwrap()
                .starts_with(&root));
            root
        };
        assert!(!path.exists());
    }
}
