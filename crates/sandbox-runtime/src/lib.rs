//! Language runtime adapters. WASM is implemented first; other runtimes fail closed.
#![deny(unsafe_code)]
#![warn(missing_docs)]

use sandbox_common::{ExecutionRequest, ExecutionResult, Language, SandboxError};
use sandbox_core::{log_execution_start, SandboxEngine};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::time::sleep;
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
        let mut config = wasmtime::Config::new();
        config.epoch_interruption(true);
        Engine::new(&config)
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
            Language::Wasm => self.execute_wasm(request).await,
            other => Err(SandboxError::UnsupportedLanguage(other)),
        }
    }

    async fn execute_wasm(
        &self,
        request: ExecutionRequest,
    ) -> Result<ExecutionResult, SandboxError> {
        let started = Instant::now();
        let timed_out = Arc::new(AtomicBool::new(false));
        let timer_timed_out = Arc::clone(&timed_out);
        let timer_engine = self.wasm.clone();
        let timeout = request.limits.timeout();
        let timer = tokio::spawn(async move {
            sleep(timeout).await;
            timer_timed_out.store(true, Ordering::Release);
            timer_engine.increment_epoch();
        });

        let worker_engine = self.wasm.clone();
        let worker_request = request.clone();
        let result = tokio::task::spawn_blocking(move || {
            execute_wasm_sync(&worker_engine, &worker_request)
        })
        .await
        .map_err(|error| SandboxError::Runtime(error.to_string()))?;
        timer.abort();

        if timed_out.load(Ordering::Acquire) {
            return Ok(self.engine.timeout_result(&request, started));
        }
        result
    }
}

fn execute_wasm_sync(
    engine: &Engine,
    request: &ExecutionRequest,
) -> Result<ExecutionResult, SandboxError> {
    let started = Instant::now();
    let bytes = hex::decode(&request.code).map_err(|_| {
        SandboxError::InvalidRequest("WASM code must be hex-encoded bytes".into())
    })?;
    let module = Module::new(engine, bytes).map_err(|e| SandboxError::Runtime(e.to_string()))?;
    let mut store = Store::new(engine, ());
    store.set_epoch_deadline(1);
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

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_common::{Language, NetworkPolicy, ResourceLimits};

    #[tokio::test]
    async fn wasm_execution_is_interrupted_at_deadline() {
        // A minimal module whose _start function loops forever.
        let infinite_loop = concat!(
            "0061736d01000000",
            "010401600000",
            "03020100",
            "070a01065f73746172740000",
            "0a0901070003400c000b0b"
        );
        let result = Runtime::default()
            .execute(ExecutionRequest {
                id: uuid::Uuid::new_v4(),
                language: Language::Wasm,
                code: infinite_loop.into(),
                limits: ResourceLimits {
                    timeout_ms: 25,
                    ..Default::default()
                },
                network: NetworkPolicy::Deny,
            })
            .await
            .unwrap();
        assert!(result.timed_out);
    }
}
