//! Python child-process adapter.
//!
//! This adapter is a process runner, not a security boundary. It does **not**
//! enforce OS-level memory, process-count, network, or filesystem isolation,
//! and it does not kill child-process descendants. In particular, accepting
//! [`NetworkPolicy::Deny`] does not mean that network access is actually
//! blocked by the current host. Do not use this adapter as protection against
//! hostile code.

use sandbox_common::{ExecutionRequest, ExecutionResult, NetworkPolicy, SandboxError};
use sandbox_core::ExecutionWorkspace;
use std::{
    ffi::OsString,
    io,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

/// Run Python source in a fresh working directory using a direct child process.
///
/// The interpreter receives source on stdin and starts in isolated mode. Only
/// a minimal environment is provided to the child; secrets from the parent
/// environment are not inherited. The process is killed if this future is
/// dropped, but descendants are outside that guarantee.
pub(crate) async fn execute(request: &ExecutionRequest) -> Result<ExecutionResult, SandboxError> {
    match &request.network {
        NetworkPolicy::Deny => {}
        _ => {
            return Err(SandboxError::InvalidRequest(
                "the Python adapter only accepts network: deny until network isolation is available"
                    .into(),
            ));
        }
    }

    let started = Instant::now();
    let workspace = ExecutionWorkspace::new()?;
    let output_limit = request.limits.output_bytes;
    let stdout_capture = Arc::new(Mutex::new(Vec::new()));
    let stderr_capture = Arc::new(Mutex::new(Vec::new()));
    let stdout_for_run = Arc::clone(&stdout_capture);
    let stderr_for_run = Arc::clone(&stderr_capture);
    let code = request.code.clone();
    let interpreter = python_executable();
    let workspace_path = workspace.root().to_path_buf();

    let execution =
        async move {
            let mut command = Command::new(interpreter);
            command
                .args(["-I", "-S", "-"])
                .current_dir(workspace_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .env_clear()
                .env("PYTHONIOENCODING", "utf-8")
                .kill_on_drop(true);

            let mut child = command.spawn().map_err(|error| {
                SandboxError::Runtime(format!("could not start Python: {error}"))
            })?;

            let stdin = child.stdin.take().ok_or_else(|| {
                SandboxError::Runtime("Python child stdin was not available".into())
            })?;
            let stdout = child.stdout.take().ok_or_else(|| {
                SandboxError::Runtime("Python child stdout was not available".into())
            })?;
            let stderr = child.stderr.take().ok_or_else(|| {
                SandboxError::Runtime("Python child stderr was not available".into())
            })?;

            let write_source = async move {
                let mut stdin = stdin;
                stdin.write_all(code.as_bytes()).await?;
                stdin.shutdown().await
            };
            let drain_stdout = drain_stream(stdout, stdout_for_run, output_limit);
            let drain_stderr = drain_stream(stderr, stderr_for_run, output_limit);
            let wait_child = child.wait();

            let (write_result, stdout_result, stderr_result, status_result) =
                tokio::join!(write_source, drain_stdout, drain_stderr, wait_child);

            write_result.map_err(|error| {
                SandboxError::Runtime(format!("could not send source to Python: {error}"))
            })?;
            stdout_result.map_err(|error| {
                SandboxError::Runtime(format!("could not read Python stdout: {error}"))
            })?;
            stderr_result.map_err(|error| {
                SandboxError::Runtime(format!("could not read Python stderr: {error}"))
            })?;
            let status = status_result.map_err(|error| {
                SandboxError::Runtime(format!("could not wait for Python: {error}"))
            })?;

            Ok::<_, SandboxError>(status.code())
        };

    let (exit_code, timed_out) =
        match tokio::time::timeout(request.limits.timeout(), execution).await {
            Ok(result) => (result?, false),
            Err(_) => (None, true),
        };

    Ok(ExecutionResult {
        id: request.id,
        exit_code,
        stdout: captured_text(&stdout_capture, output_limit),
        stderr: captured_text(&stderr_capture, output_limit),
        timed_out,
        duration_ms: started.elapsed().as_millis(),
        completed_at: chrono::Utc::now(),
    })
}

fn python_executable() -> OsString {
    if let Some(executable) = std::env::var_os("CORTEXENV_PYTHON") {
        if !executable.is_empty() {
            return executable;
        }
    }

    #[cfg(windows)]
    {
        OsString::from("python")
    }
    #[cfg(not(windows))]
    {
        OsString::from("python3")
    }
}

async fn drain_stream<R>(
    mut stream: R,
    capture: Arc<Mutex<Vec<u8>>>,
    limit: usize,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 8192];
    loop {
        let count = stream.read(&mut buffer).await?;
        if count == 0 {
            return Ok(());
        }
        let mut captured = capture
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let remaining = limit.saturating_sub(captured.len());
        captured.extend_from_slice(&buffer[..count.min(remaining)]);
    }
}

fn captured_text(capture: &Mutex<Vec<u8>>, limit: usize) -> String {
    let bytes = capture
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if text.len() > limit {
        let mut end = limit;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_common::{Language, NetworkPolicy, ResourceLimits};

    fn request(code: &str) -> ExecutionRequest {
        ExecutionRequest {
            id: uuid::Uuid::new_v4(),
            language: Language::Python,
            code: code.into(),
            limits: ResourceLimits::default(),
            network: NetworkPolicy::Deny,
        }
    }

    #[tokio::test]
    async fn captures_stdout() {
        let result = execute(&request("print('hello from python')"))
            .await
            .unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout, "hello from python\n");
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn captures_stderr_and_nonzero_exit() {
        let result = execute(&request(
            "import sys; print('failure', file=sys.stderr); sys.exit(7)",
        ))
        .await
        .unwrap();
        assert_eq!(result.exit_code, Some(7));
        assert_eq!(result.stderr, "failure\n");
        assert!(!result.timed_out);
    }

    #[tokio::test]
    async fn terminates_child_on_timeout() {
        let mut request = request("import time; time.sleep(30)");
        request.limits.timeout_ms = 100;
        let result = execute(&request).await.unwrap();
        assert!(result.timed_out);
        assert!(result.duration_ms < 5_000);
    }

    #[tokio::test]
    async fn truncates_each_output_stream() {
        let mut request =
            request("print('x' * 4096); import sys; print('y' * 4096, file=sys.stderr)");
        request.limits.output_bytes = 32;
        let result = execute(&request).await.unwrap();
        assert_eq!(result.stdout, "x".repeat(32));
        assert_eq!(result.stderr, "y".repeat(32));
    }

    #[tokio::test]
    async fn rejects_network_policies_it_cannot_enforce() {
        let mut request = request("print('should not run')");
        request.network = NetworkPolicy::Loopback;
        let error = execute(&request).await.unwrap_err();
        assert!(matches!(error, SandboxError::InvalidRequest(_)));
    }
}
