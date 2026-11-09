//! Shared request, result, configuration, and error types.
#![deny(unsafe_code)]
#![warn(missing_docs)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt, time::Duration};
use thiserror::Error;
use uuid::Uuid;

/// Supported execution languages.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// WebAssembly.
    Wasm,
    /// JavaScript source executed by the deno_core adapter.
    JavaScript,
    /// Python source executed by a child interpreter process.
    Python,
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Wasm => "wasm",
            Self::JavaScript => "javascript",
            Self::Python => "python",
        })
    }
}

/// Network access policy. The default is deny.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    /// No network access.
    Deny,
    /// Loopback-only access.
    Loopback,
    /// Explicit host allowlist (future phase).
    Allowlist(Vec<String>),
}
impl Default for NetworkPolicy {
    fn default() -> Self {
        Self::Deny
    }
}

/// Per-execution resource ceilings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceLimits {
    /// Wall-clock timeout in milliseconds.
    pub timeout_ms: u64,
    /// Memory ceiling in bytes.
    pub memory_bytes: u64,
    /// Process/thread ceiling.
    pub process_count: u32,
    /// Open-file ceiling.
    pub file_descriptors: u64,
    /// Captured output ceiling per stream.
    pub output_bytes: usize,
}
impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            timeout_ms: 5_000,
            memory_bytes: 128 * 1024 * 1024,
            process_count: 32,
            file_descriptors: 64,
            output_bytes: 1024 * 1024,
        }
    }
}
impl ResourceLimits {
    /// Validate a limit set before execution.
    pub fn validate(&self) -> Result<(), SandboxError> {
        if self.timeout_ms == 0 || self.timeout_ms > 300_000 {
            return Err(SandboxError::InvalidRequest(
                "timeout_ms must be between 1 and 300000".into(),
            ));
        }
        if !(64 * 1024..=8 * 1024 * 1024 * 1024).contains(&self.memory_bytes) {
            return Err(SandboxError::InvalidRequest(
                "memory_bytes is outside the supported range".into(),
            ));
        }
        if self.process_count == 0
            || self.process_count > 4096
            || self.file_descriptors == 0
            || self.file_descriptors > 4096
            || self.output_bytes == 0
            || self.output_bytes > 16 * 1024 * 1024
        {
            return Err(SandboxError::InvalidRequest(
                "resource limits are outside the supported range".into(),
            ));
        }
        Ok(())
    }
    /// Return the timeout as a standard duration.
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
}

/// A validated unit of untrusted work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    /// Stable request identifier.
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    /// Runtime to invoke.
    pub language: Language,
    /// JavaScript/Python source text or a hex-encoded WASM module.
    pub code: String,
    /// Resource ceilings.
    #[serde(default)]
    pub limits: ResourceLimits,
    /// Network policy.
    #[serde(default)]
    pub network: NetworkPolicy,
}
impl ExecutionRequest {
    /// Validate request shape and limits.
    pub fn validate(&self) -> Result<(), SandboxError> {
        if self.code.is_empty() || self.code.len() > 16 * 1024 * 1024 {
            return Err(SandboxError::InvalidRequest(
                "code must be non-empty and <= 16 MiB".into(),
            ));
        }
        self.limits.validate()
    }
}

/// Result of one execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Request identifier.
    pub id: Uuid,
    /// Exit status.
    pub exit_code: Option<i32>,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
    /// Whether the deadline was exceeded.
    pub timed_out: bool,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u128,
    /// Completion time.
    pub completed_at: DateTime<Utc>,
}

/// Errors exposed by the engine.
#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("invalid request: {0}")]
    /// Request validation failed.
    InvalidRequest(String),
    #[error("unsupported language: {0}")]
    /// The selected runtime is not enabled.
    UnsupportedLanguage(Language),
    #[error("resource limit exceeded: {0}")]
    /// A resource ceiling could not be applied or was exceeded.
    ResourceLimit(String),
    #[error("execution timed out")]
    /// The execution exceeded its deadline.
    Timeout,
    #[error("runtime error: {0}")]
    /// The language runtime rejected the program.
    Runtime(String),
    #[error("isolation setup failed")]
    /// The isolation boundary could not be prepared.
    Isolation,
    #[error("internal error")]
    /// An unexpected internal failure occurred.
    Internal,
}

/// Service configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// API bind address.
    pub bind_addr: String,
    /// Default limits.
    #[serde(default)]
    pub default_limits: ResourceLimits,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:8080".into(),
            default_limits: ResourceLimits::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_deny_network() {
        assert_eq!(NetworkPolicy::default(), NetworkPolicy::Deny);
        assert!(ResourceLimits::default().validate().is_ok());
    }
    #[test]
    fn rejects_zero_timeout() {
        let mut l = ResourceLimits::default();
        l.timeout_ms = 0;
        assert!(l.validate().is_err());
    }

    #[test]
    fn rejects_excessive_output_limit() {
        let mut limits = ResourceLimits::default();
        limits.output_bytes = 16 * 1024 * 1024 + 1;
        assert!(limits.validate().is_err());
    }
}
