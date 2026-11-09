//! JavaScript execution with a deliberately minimal `deno_core` runtime.
//!
//! This runtime installs only a small console-capture op and disables the
//! built-in direct-print op; it does not install Deno filesystem, network, or
//! process APIs. V8 execution is terminated at the wall-clock deadline, but
//! this is not OS-level isolation and this adapter does not enforce the
//! request's memory, process-count, or descriptor limits.
//! Promise/event-loop work is not driven after the source script returns.
use deno_core::{op2, JsRuntime, RuntimeOptions};
use sandbox_common::{ExecutionRequest, ExecutionResult, NetworkPolicy, SandboxError};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::{sync::oneshot, time::sleep};

#[derive(Clone)]
struct OutputCapture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    limit: usize,
}

impl OutputCapture {
    fn new(limit: usize) -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
            limit,
        }
    }

    fn push(&mut self, stream: &str, text: &str) {
        let destination = match stream {
            "stdout" => &mut self.stdout,
            "stderr" => &mut self.stderr,
            _ => return,
        };
        let remaining = self.limit.saturating_sub(destination.len());
        let mut count = text.len().min(remaining);
        while !text.is_char_boundary(count) {
            count -= 1;
        }
        destination.extend_from_slice(&text.as_bytes()[..count]);
    }

    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

#[op2(fast)]
fn op_capture_output(
    #[state] output: &mut OutputCapture,
    #[string] stream: &str,
    #[string] text: &str,
) {
    output.push(stream, text);
}

deno_core::extension!(sandbox_javascript, ops = [op_capture_output]);

/// Execute JavaScript source in a `deno_core` V8 isolate.
///
/// The caller is expected to validate the request before dispatch. This
/// function validates it as well so it remains safe when used directly.
pub(crate) async fn execute(request: &ExecutionRequest) -> Result<ExecutionResult, SandboxError> {
    request.validate()?;
    if request.network != NetworkPolicy::Deny {
        return Err(SandboxError::InvalidRequest(
            "the JavaScript adapter only accepts network: deny until network isolation is available"
                .into(),
        ));
    }
    let started = Instant::now();
    let timeout = request.limits.timeout();
    let request = request.clone();
    let timed_out = Arc::new(AtomicBool::new(false));
    let timer_timed_out = Arc::clone(&timed_out);
    let (handle_tx, handle_rx) = oneshot::channel::<deno_core::v8::IsolateHandle>();

    // A timeout around the JoinHandle alone would leave synchronous JavaScript
    // running in the blocking pool. Instead, request V8 termination and always
    // join that worker before returning.
    let timer = tokio::spawn(async move {
        sleep(timeout).await;
        if let Ok(isolate_handle) = handle_rx.await {
            timer_timed_out.store(true, Ordering::Release);
            let _ = isolate_handle.terminate_execution();
        }
    });

    let worker_timed_out = Arc::clone(&timed_out);
    let worker = tokio::task::spawn_blocking(move || {
        execute_sync(&request, started, handle_tx, worker_timed_out)
    });
    let result = worker
        .await
        .map_err(|error| SandboxError::Runtime(error.to_string()))?;
    timer.abort();
    result
}

fn execute_sync(
    request: &ExecutionRequest,
    started: Instant,
    handle_tx: oneshot::Sender<deno_core::v8::IsolateHandle>,
    timed_out: Arc<AtomicBool>,
) -> Result<ExecutionResult, SandboxError> {
    let mut extension = sandbox_javascript::init_ops_and_esm();
    extension.middleware_fn = Some(Box::new(|op| match op.name {
        "op_print" => op.disable(),
        _ => op,
    }));
    let mut runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![extension],
        ..Default::default()
    });
    runtime
        .op_state()
        .borrow_mut()
        .put(OutputCapture::new(request.limits.output_bytes));

    let isolate_handle = runtime.v8_isolate().thread_safe_handle();
    // If the async caller disappeared, there is no deadline request to service.
    let _ = handle_tx.send(isolate_handle);

    let bootstrap_result = runtime.execute_script(
        "<sandbox-console>",
        r#"
            globalThis.console = Object.freeze({
              log: (...values) => Deno.core.ops.op_capture_output("stdout", values.map(String).join(" ") + "\n"),
              info: (...values) => Deno.core.ops.op_capture_output("stdout", values.map(String).join(" ") + "\n"),
              debug: (...values) => Deno.core.ops.op_capture_output("stdout", values.map(String).join(" ") + "\n"),
              warn: (...values) => Deno.core.ops.op_capture_output("stderr", values.map(String).join(" ") + "\n"),
              error: (...values) => Deno.core.ops.op_capture_output("stderr", values.map(String).join(" ") + "\n")
            });
            "#,
    );

    let script_result = match bootstrap_result {
        Ok(_) if !timed_out.load(Ordering::Acquire) => {
            Some(runtime.execute_script("<sandbox-script>", request.code.clone()))
        }
        Ok(_) => None,
        Err(_) if timed_out.load(Ordering::Acquire) => None,
        Err(error) => return Err(SandboxError::Runtime(error.to_string())),
    };
    let did_time_out = timed_out.load(Ordering::Acquire);
    let mut output = runtime
        .op_state()
        .borrow()
        .borrow::<OutputCapture>()
        .clone();
    if did_time_out {
        output.push("stderr", "execution timed out\n");
    }
    let (exit_code, stderr) = if did_time_out {
        (None, output.stderr())
    } else {
        match script_result.expect("non-timeout execution must have a script result") {
            Ok(_) => (Some(0), output.stderr()),
            Err(error) => {
                output.push("stderr", &format!("{error}\n"));
                (Some(1), output.stderr())
            }
        }
    };

    Ok(ExecutionResult {
        id: request.id,
        exit_code,
        stdout: output.stdout(),
        stderr,
        timed_out: did_time_out,
        duration_ms: started.elapsed().as_millis(),
        completed_at: chrono::Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_common::{Language, NetworkPolicy, ResourceLimits};

    fn request(code: &str, timeout_ms: u64) -> ExecutionRequest {
        ExecutionRequest {
            id: uuid::Uuid::new_v4(),
            language: Language::JavaScript,
            code: code.into(),
            limits: ResourceLimits {
                timeout_ms,
                output_bytes: 32,
                ..Default::default()
            },
            network: NetworkPolicy::Deny,
        }
    }

    #[tokio::test]
    async fn executes_javascript_and_captures_bounded_console_output() {
        let result = execute(&request(
            "try { Deno.core.print('uncapped'); } catch (_) {} console.log('hello', 'world');",
            1_000,
        ))
        .await
        .unwrap();

        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout, "hello world\n");
        assert!(result.stderr.is_empty());
        assert!(!result.timed_out);
        assert!(result.duration_ms < 1_000);

        let bounded = execute(&request(
            "console.log('012345678901234567890123456789012345');",
            1_000,
        ))
        .await
        .unwrap();
        assert_eq!(bounded.stdout.len(), 32);
    }

    #[tokio::test]
    async fn javascript_exception_is_reported_as_failure() {
        let result = execute(&request(
            "console.log('before'); throw new Error('broken');",
            1_000,
        ))
        .await
        .unwrap();

        assert_eq!(result.exit_code, Some(1));
        assert_eq!(result.stdout, "before\n");
        assert!(result.stderr.contains("broken"), "{}", result.stderr);
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn infinite_loop_is_terminated_at_deadline() {
        let result = execute(&request("for (;;) {}", 25)).await.unwrap();

        assert!(result.timed_out);
        assert_eq!(result.exit_code, None);
        assert!(result.stderr.contains("execution timed out"));
        assert!(result.duration_ms < 2_000, "worker did not stop promptly");
    }

    #[tokio::test]
    async fn rejects_network_policies_it_cannot_enforce() {
        let mut request = request("console.log('should not run');", 1_000);
        request.network = NetworkPolicy::Loopback;
        let error = execute(&request).await.unwrap_err();
        assert!(matches!(error, SandboxError::InvalidRequest(_)));
    }
}
