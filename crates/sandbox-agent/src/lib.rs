//! Agent-facing execution tool abstraction.
#![deny(unsafe_code)]
#![warn(missing_docs)]

use async_trait::async_trait;
use sandbox_common::{ExecutionRequest, ExecutionResult, SandboxError};
use std::sync::Arc;

/// Runtime used by the built-in local execution tool.
pub use sandbox_runtime::Runtime;

/// Language-model interfaces and OpenAI-compatible HTTP provider.
pub mod llm;
/// Bounded code-execution and self-correction loop.
pub mod react;

/// Provider-neutral code execution tool.
#[async_trait]
pub trait ExecuteCode: Send + Sync {
    /// Execute one request.
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, SandboxError>;
}

#[async_trait]
impl<T> ExecuteCode for Arc<T>
where
    T: ExecuteCode + ?Sized,
{
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, SandboxError> {
        (**self).execute(request).await
    }
}

/// Local runtime tool implementation.
pub struct RuntimeTool {
    runtime: Arc<Runtime>,
}
impl RuntimeTool {
    /// Construct a tool around a runtime.
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self { runtime }
    }
}
#[async_trait]
impl ExecuteCode for RuntimeTool {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult, SandboxError> {
        self.runtime.execute(request).await
    }
}
