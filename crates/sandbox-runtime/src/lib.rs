//! Language runtime adapters. WASM is implemented first; other runtimes fail closed.
#![deny(unsafe_code)]
#![warn(missing_docs)]

use sandbox_common::{ExecutionRequest, ExecutionResult, Language, SandboxError};
use sandbox_core::{log_execution_start, SandboxEngine};
use std::time::Instant;
use wasmtime::{Engine, Instance, Module, Store};

/// Runtime dispatcher.
pub struct Runtime {
    engine: SandboxEngine,
    wasm: Engine,
}
impl Default for Runtime {
    fn default() -> Self {
        Self::new().expect("default Wasmtime configuration must be valid")
    }
}
impl Runtime {
    /// Create a runtime with a conservative Wasmtime engine.
    pub fn new() -> Result<Self, SandboxError> {
        Engine::new(&wasmtime::Config::new())
            .map(|wasm| Self {
                engine: SandboxEngine::new(),
                wasm,
            })
            .map_err(|e| SandboxError::Runtime(e.to_string()))
    }
    /// Execute a request. WASM `code` is a hex-encoded module.
    pub async fn execute(
        &self,
        request: ExecutionRequest,
    ) -> Result<ExecutionResult, SandboxError> {
        self.engine.validate(&request)?;
        log_execution_start(&request);
        match request.language {
            Language::Wasm => self.execute_wasm(&request),
            other => Err(SandboxError::UnsupportedLanguage(other)),
        }
    }
    fn execute_wasm(&self, request: &ExecutionRequest) -> Result<ExecutionResult, SandboxError> {
        let started = Instant::now();
        let bytes = hex::decode(&request.code).map_err(|_| {
            SandboxError::InvalidRequest("WASM code must be hex-encoded bytes".into())
        })?;
        let module =
            Module::new(&self.wasm, bytes).map_err(|e| SandboxError::Runtime(e.to_string()))?;
        let mut store = Store::new(&self.wasm, ());
        let instance = Instance::new(&mut store, &module, &[])
            .map_err(|e| SandboxError::Runtime(e.to_string()))?;
        if let Ok(start) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
            start
                .call(&mut store, ())
                .map_err(|e| SandboxError::Runtime(e.to_string()))?;
        }
        Ok(ExecutionResult {
            id: request.id,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            duration_ms: started.elapsed().as_millis(),
            completed_at: chrono::Utc::now(),
        })
    }
}
