//! Language runtime adapters for WebAssembly, JavaScript, and Python.
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

mod javascript;
mod python;

/// Runtime dispatcher.
pub struct Runtime {
    engine: SandboxEngine,
}
impl Default for Runtime {
    fn default() -> Self {
        Self::new().expect("default Wasmtime configuration must be valid")
    }
}
impl Runtime {
    /// Create a runtime with a conservative Wasmtime engine.
    pub fn new() -> Result<Self, SandboxError> {
        // Validate Wasmtime configuration at service startup. Each execution
        // receives a distinct Engine so one job's timeout tick cannot interrupt
        // unrelated jobs sharing a global epoch counter.
        create_wasm_engine()?;
        Ok(Self {
            engine: SandboxEngine::new(),
        })
    }
    /// Execute a request. WASM `code` is a hex-encoded module; JavaScript and Python `code` is source text.
    pub async fn execute(
        &self,
        request: ExecutionRequest,
    ) -> Result<ExecutionResult, SandboxError> {
        self.engine.validate(&request)?;
        log_execution_start(&request);
        match request.language {
            Language::Wasm => self.execute_wasm(request).await,
            Language::JavaScript => javascript::execute(&request).await,
            Language::Python => python::execute(&request).await,
        }
    }

    async fn execute_wasm(
        &self,
        request: ExecutionRequest,
    ) -> Result<ExecutionResult, SandboxError> {
        let started = Instant::now();
        let timed_out = Arc::new(AtomicBool::new(false));
        let timer_timed_out = Arc::clone(&timed_out);
        let worker_engine = create_wasm_engine()?;
        let timer_engine = worker_engine.clone();
        let timeout = request.limits.timeout().saturating_sub(started.elapsed());
        if timeout.is_zero() {
            return Ok(self.engine.timeout_result(&request, started));
        }
        let timer = tokio::spawn(async move {
            sleep(timeout).await;
            timer_timed_out.store(true, Ordering::Release);
            // Module validation/compilation can outlast short request timeouts.
            // Keep advancing this execution's private engine epoch until the
            // worker is joined, so its store cannot miss the one deadline tick.
            loop {
                timer_engine.increment_epoch();
                sleep(std::time::Duration::from_millis(10)).await;
            }
        });

        let worker_request = request.clone();
        let result =
            tokio::task::spawn_blocking(move || execute_wasm_sync(&worker_engine, &worker_request))
                .await
                .map_err(|error| SandboxError::Runtime(error.to_string()))?;
        timer.abort();

        if timed_out.load(Ordering::Acquire) {
            return Ok(self.engine.timeout_result(&request, started));
        }
        result
    }
}

fn create_wasm_engine() -> Result<Engine, SandboxError> {
    let mut config = wasmtime::Config::new();
    config.epoch_interruption(true);
    Engine::new(&config).map_err(|error| SandboxError::Runtime(error.to_string()))
}

fn execute_wasm_sync(
    engine: &Engine,
    request: &ExecutionRequest,
) -> Result<ExecutionResult, SandboxError> {
    let started = Instant::now();
    let bytes = hex::decode(&request.code)
        .map_err(|_| SandboxError::InvalidRequest("WASM code must be hex-encoded bytes".into()))?;
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
