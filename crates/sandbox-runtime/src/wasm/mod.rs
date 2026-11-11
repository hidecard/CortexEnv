//! Wasmtime execution and per-store resource limits.

use sandbox_common::{ExecutionRequest, ExecutionResult, SandboxError};
use wasmtime::{Engine, Instance, Module, Store, StoreLimitsBuilder};

pub(super) fn create_engine() -> Result<Engine, SandboxError> {
    let mut config = wasmtime::Config::new();
    config.epoch_interruption(true);
    Engine::new(&config).map_err(|error| SandboxError::Runtime(error.to_string()))
}

pub(super) fn execute_wasm_sync(
    engine: &Engine,
    request: &ExecutionRequest,
) -> Result<ExecutionResult, SandboxError> {
    let started = std::time::Instant::now();
    let bytes = hex::decode(&request.code)
        .map_err(|_| SandboxError::InvalidRequest("WASM code must be hex-encoded bytes".into()))?;
    let module =
        Module::new(engine, bytes).map_err(|error| SandboxError::Runtime(error.to_string()))?;
    let memory_limit = usize::try_from(request.limits.memory_bytes).map_err(|_| {
        SandboxError::InvalidRequest(
            "WASM memory limit exceeds this platform's address size".into(),
        )
    })?;
    let store_limits = StoreLimitsBuilder::new()
        .memory_size(memory_limit)
        .instances(1)
        .memories(1)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(engine, store_limits);
    store.limiter(|limits| limits);
    store.set_epoch_deadline(1);
    let instance = Instance::new(&mut store, &module, &[])
        .map_err(|error| SandboxError::Runtime(error.to_string()))?;
    if let Ok(start) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
        start
            .call(&mut store, ())
            .map_err(|error| SandboxError::Runtime(error.to_string()))?;
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

    #[test]
    fn enforces_the_request_memory_limit_during_wasm_growth() {
        // One-page memory with an _start that tries to grow it by one more page.
        let grow_beyond_limit = concat!(
            "0061736d01000000",
            "010401600000",
            "03020100",
            "0503010001",
            "070a0106065f73746172740000",
            "0a09010700410140001a0b"
        );
        let mut limits = ResourceLimits::default();
        limits.memory_bytes = 65_536;
        let request = ExecutionRequest {
            id: uuid::Uuid::new_v4(),
            language: Language::Wasm,
            code: grow_beyond_limit.into(),
            limits,
            network: NetworkPolicy::Deny,
        };
        let engine = create_engine().unwrap();

        let result = execute_wasm_sync(&engine, &request);

        assert!(matches!(result, Err(SandboxError::Runtime(_))));
    }
}
