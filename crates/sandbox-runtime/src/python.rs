//! Python child-process adapter.
//!
//! This adapter is a process runner, not a complete security boundary. On Linux
//! targets with `waitid(WNOWAIT)` support it kills members of the child's
//! process group when execution ends or times out. Setting
//! `CORTEXENV_CGROUP_ROOT` additionally opts the Python child into cgroup v2
//! CPU, memory, swap, and process-count limits before user code starts; the
//! cgroup's `cgroup.kill` file is used for cleanup when the kernel exposes it.
//! Without that opt-in, descendants that escape the process group can survive.
//! Neither mode enforces network or filesystem isolation, and accepting
//! [`NetworkPolicy::Deny`] does not mean network access is blocked by the
//! host. Do not use this adapter as protection against hostile code.

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
use nix::{
    errno::Errno,
    sys::signal::{kill, killpg, Signal},
    unistd::Pid,
};
use sandbox_common::{ExecutionRequest, ExecutionResult, NetworkPolicy, SandboxError};
#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
use sandbox_core::linux::CgroupV2;
use sandbox_core::ExecutionWorkspace;
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

/// Run Python source in a fresh working directory using a supervised child process.
///
/// The interpreter receives source on stdin and starts in isolated mode. Only
/// a minimal environment is provided to the child; secrets from the parent
/// environment are not inherited. On supported Linux targets the interpreter
/// receives its own process group; remaining group members are killed on
/// timeout and after the interpreter exits. If `CORTEXENV_CGROUP_ROOT` points
/// to a writable delegated cgroup v2 subtree, the child is attached before
/// Python source is released from a stop/exec gate. Targets without the
/// required Linux wait/signal support do not enable cgroup execution.
pub(crate) async fn execute(request: &ExecutionRequest) -> Result<ExecutionResult, SandboxError> {
    let cgroup_root = std::env::var_os("CORTEXENV_CGROUP_ROOT").map(PathBuf::from);
    execute_with_cgroup_root(request, cgroup_root.as_deref()).await
}

async fn execute_with_cgroup_root(
    request: &ExecutionRequest,
    cgroup_root: Option<&Path>,
) -> Result<ExecutionResult, SandboxError> {
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
    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    let cgroup: Option<CgroupHandle> = cgroup_root
        .map(|root| CgroupV2::create(root, request.id, &request.limits))
        .transpose()?;
    #[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
    let cgroup: Option<CgroupHandle> = if cgroup_root.is_some() {
        return Err(SandboxError::Isolation);
    } else {
        None
    };
    let execution =
        async move {
            let mut command = python_command(interpreter, cgroup.is_some());
            #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
            command.process_group(0);
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
            let mut process_group = ProcessGroupGuard::new(child.id(), cgroup)?;
            #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
            if process_group.cgroup().is_some() {
                wait_for_child_stop(&mut child).await?;
                let pid = child.id().ok_or(SandboxError::Isolation)?;
                process_group
                    .cgroup()
                    .ok_or(SandboxError::Isolation)?
                    .attach_pid(pid)?;
                resume_child(pid)?;
            }

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
            let wait_child = wait_child_supervised(&mut child, &mut process_group);

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

    let execution_result = tokio::time::timeout(request.limits.timeout(), execution).await;
    let (exit_code, timed_out) = match execution_result {
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

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
type CgroupHandle = CgroupV2;

#[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
type CgroupHandle = ();

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
fn python_command(interpreter: OsString, wait_for_cgroup: bool) -> Command {
    let mut command = if wait_for_cgroup {
        let mut shell = Command::new("/bin/sh");
        shell.args(["-c", "kill -STOP $$; exec \"$@\"", "cortexenv-python"]);
        shell.arg(resolve_python_for_shell(interpreter));
        shell
    } else {
        Command::new(interpreter)
    };
    command
}

#[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
fn python_command(interpreter: OsString, _wait_for_cgroup: bool) -> Command {
    Command::new(interpreter)
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
fn resolve_python_for_shell(executable: OsString) -> OsString {
    if Path::new(&executable).components().count() > 1 {
        return executable;
    }
    let Some(search_path) = std::env::var_os("PATH") else {
        return executable;
    };
    for directory in std::env::split_paths(&search_path) {
        let candidate = directory.join(&executable);
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .unwrap_or(candidate)
                .into_os_string();
        }
    }
    executable
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
struct ProcessGroupGuard {
    pgid: Pid,
    armed: bool,
    cgroup: Option<CgroupV2>,
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
impl ProcessGroupGuard {
    fn new(pid: Option<u32>, cgroup: Option<CgroupV2>) -> Result<Self, SandboxError> {
        let pid = pid.ok_or(SandboxError::Isolation)?;
        let pid = i32::try_from(pid).map_err(|_| SandboxError::Isolation)?;
        Ok(Self {
            pgid: Pid::from_raw(pid),
            armed: true,
            cgroup,
        })
    }

    fn cgroup(&self) -> Option<&CgroupV2> {
        self.cgroup.as_ref()
    }

    fn terminate(&mut self) -> Result<(), SandboxError> {
        let result = loop {
            match killpg(self.pgid, Signal::SIGKILL) {
                Ok(()) | Err(Errno::ESRCH) => break Ok(()),
                Err(Errno::EINTR) => continue,
                Err(_) => break Err(SandboxError::Isolation),
            }
        };
        if let Some(cgroup) = &self.cgroup {
            let _ = cgroup.kill_all();
        }
        if result.is_ok() {
            self.armed = false;
        }
        result
    }
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
async fn wait_for_child_stop(child: &mut tokio::process::Child) -> Result<(), SandboxError> {
    let pid = child.id().ok_or(SandboxError::Isolation)?;
    let pid = i32::try_from(pid).map_err(|_| SandboxError::Isolation)?;
    loop {
        match nix::sys::wait::waitid(
            nix::sys::wait::Id::Pid(Pid::from_raw(pid)),
            nix::sys::wait::WaitPidFlag::WSTOPPED | nix::sys::wait::WaitPidFlag::WNOHANG,
        ) {
            Ok(nix::sys::wait::WaitStatus::Stopped(_, Signal::SIGSTOP)) => return Ok(()),
            Ok(nix::sys::wait::WaitStatus::StillAlive) => {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            Ok(_) => return Err(SandboxError::Isolation),
            Err(Errno::EINTR) => continue,
            Err(_) => return Err(SandboxError::Isolation),
        }
    }
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
fn resume_child(pid: u32) -> Result<(), SandboxError> {
    let pid = i32::try_from(pid).map_err(|_| SandboxError::Isolation)?;
    kill(Pid::from_raw(pid), Signal::SIGCONT).map_err(|_| SandboxError::Isolation)
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.terminate();
        }
    }
}

#[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
struct ProcessGroupGuard;

#[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
impl ProcessGroupGuard {
    fn new(_pid: Option<u32>, _cgroup: Option<()>) -> Result<Self, SandboxError> {
        Ok(Self)
    }
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
async fn wait_child_supervised(
    child: &mut tokio::process::Child,
    process_group: &mut ProcessGroupGuard,
) -> io::Result<ExitStatus> {
    let pid = child
        .id()
        .ok_or_else(|| io::Error::other("Python child PID is unavailable"))?;
    let pid = i32::try_from(pid).map_err(io::Error::other)?;
    loop {
        match nix::sys::wait::waitid(
            nix::sys::wait::Id::Pid(Pid::from_raw(pid)),
            nix::sys::wait::WaitPidFlag::WEXITED
                | nix::sys::wait::WaitPidFlag::WNOWAIT
                | nix::sys::wait::WaitPidFlag::WNOHANG,
        ) {
            Ok(nix::sys::wait::WaitStatus::StillAlive) => {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            Ok(_) => break,
            Err(Errno::EINTR) => continue,
            Err(_) => return Err(io::Error::other("could not observe Python child exit")),
        }
    }
    process_group
        .terminate()
        .map_err(|_| io::Error::other("could not terminate Python process group"))?;
    child.wait().await
}

#[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
async fn wait_child_supervised(
    child: &mut tokio::process::Child,
    _process_group: &mut ProcessGroupGuard,
) -> io::Result<ExitStatus> {
    child.wait().await
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

    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    #[tokio::test]
    async fn cgroup_gate_waits_for_parent_continue_before_exec() {
        let mut command = python_command(python_executable(), true);
        command
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().unwrap();
        let mut process_group = ProcessGroupGuard::new(child.id(), None).unwrap();
        let pid = child.id().unwrap();

        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            wait_for_child_stop(&mut child),
        )
        .await
        .unwrap()
        .unwrap();
        resume_child(pid).unwrap();
        let status = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            wait_child_supervised(&mut child, &mut process_group),
        )
        .await
        .unwrap()
        .unwrap();

        assert!(status.success());
    }

    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    #[tokio::test]
    async fn cgroup_setup_failure_prevents_python_from_starting() {
        let root =
            std::env::temp_dir().join(format!("cortexenv-cgroup-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("cgroup.controllers"), "cpu memory pids").unwrap();
        let marker = root.join("python-ran");
        let marker_literal = serde_json::to_string(marker.to_str().unwrap()).unwrap();
        let request = request(&format!("open({marker_literal}, 'w').write('started')"));

        let result = execute_with_cgroup_root(&request, Some(&root)).await;

        assert!(matches!(result, Err(SandboxError::Isolation)));
        assert!(!marker.exists());
        assert!(!root.join(format!("cortexenv-{}", request.id)).exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn captures_stdout() {
        let result = execute(&request("print('hello from python')"))
            .await
            .unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout, "hello from python\n");
        assert!(!result.timed_out);
    }

    #[cfg(not(windows))]
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

    #[cfg(not(windows))]
    #[tokio::test]
    async fn terminates_child_on_timeout() {
        let mut request = request("import time; time.sleep(30)");
        request.limits.timeout_ms = 100;
        let result = execute(&request).await.unwrap();
        assert!(result.timed_out);
        assert!(result.duration_ms < 5_000);
    }

    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    #[tokio::test]
    async fn kills_descendant_in_child_process_group_on_timeout() {
        let marker = std::env::temp_dir().join(format!(
            "cortexenv-descendant-{}.marker",
            uuid::Uuid::new_v4()
        ));
        let executable = python_executable();
        let python_path = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|directory| directory.join(&executable))
            .find(|candidate| candidate.is_file())
            .expect("Python interpreter is available on PATH");
        let python_literal = serde_json::to_string(python_path.to_str().unwrap()).unwrap();
        let marker_literal = serde_json::to_string(marker.to_str().unwrap()).unwrap();
        let descendant_code = format!(
            "import time,pathlib; time.sleep(1); pathlib.Path({marker_literal}).write_text('survived')"
        );
        let descendant_argument = serde_json::to_string(&descendant_code).unwrap();
        let source = format!(
            "import subprocess,time; subprocess.Popen([{python_literal}, '-I', '-S', '-c', {descendant_argument}]); print('spawned', flush=True); time.sleep(30)"
        );
        let mut request = request(&source);
        request.limits.timeout_ms = 500;

        let result = execute(&request).await.unwrap();
        assert!(result.timed_out, "child result: {result:?}");
        assert!(result.stdout.contains("spawned"), "{result:?}");
        tokio::time::sleep(std::time::Duration::from_millis(1_300)).await;
        let descendant_survived = marker.exists();
        let _ = std::fs::remove_file(&marker);
        assert!(
            !descendant_survived,
            "a child in the supervised process group outlived its parent"
        );
    }

    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    #[tokio::test]
    async fn kills_background_descendant_after_python_leader_exits() {
        let marker = std::env::temp_dir().join(format!(
            "cortexenv-background-descendant-{}.marker",
            uuid::Uuid::new_v4()
        ));
        let executable = python_executable();
        let python_path = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|directory| directory.join(&executable))
            .find(|candidate| candidate.is_file())
            .expect("Python interpreter is available on PATH");
        let python_literal = serde_json::to_string(python_path.to_str().unwrap()).unwrap();
        let marker_literal = serde_json::to_string(marker.to_str().unwrap()).unwrap();
        let descendant_code = format!(
            "import time,pathlib; time.sleep(1); pathlib.Path({marker_literal}).write_text('survived')"
        );
        let descendant_argument = serde_json::to_string(&descendant_code).unwrap();
        let source = format!(
            "import subprocess; subprocess.Popen([{python_literal}, '-I', '-S', '-c', {descendant_argument}], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL); print('spawned', flush=True)"
        );

        let result = execute(&request(&source)).await.unwrap();
        assert_eq!(result.exit_code, Some(0), "{result:?}");
        assert!(!result.timed_out);
        assert!(result.stdout.contains("spawned"), "{result:?}");
        tokio::time::sleep(std::time::Duration::from_millis(1_300)).await;
        let descendant_survived = marker.exists();
        let _ = std::fs::remove_file(&marker);
        assert!(
            !descendant_survived,
            "background child survived after its process-group leader exited"
        );
    }

    #[cfg(not(windows))]
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
