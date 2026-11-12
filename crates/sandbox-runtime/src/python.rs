//! Python child-process adapter.
//!
//! This adapter is a process runner, not a complete security boundary. On Linux
//! targets with `waitid(WNOWAIT)` support it kills members of the child's
//! process group when execution ends or times out. `CORTEXENV_ENABLE_LINUX_SANDBOX=true`
//! opts into bubblewrap namespaces, a read-only root, bounded private tmpfs
//! locations, and capability dropping; setup failure never falls back to an
//! unrestricted interpreter. `CORTEXENV_CGROUP_ROOT` separately opts into cgroup
//! v2 resource controls and group cleanup when the kernel supports it. Without
//! the namespace opt-in, [`NetworkPolicy::Deny`] is not enforced by the host.
//! Seccomp and complete cross-runtime isolation remain unavailable; do not use
//! this adapter as protection against hostile code.

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
use nix::{
    errno::Errno,
    sys::signal::{kill, killpg, Signal},
    unistd::Pid,
};
use sandbox_common::{ExecutionRequest, ExecutionResult, NetworkPolicy, SandboxError};
#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
use sandbox_core::linux::{ChildSupervisor, ChildSupervisorConfig, PreparedExecution};
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
/// timeout and after the interpreter exits. Setting
/// `CORTEXENV_ENABLE_LINUX_SANDBOX=true` opts into the partial bubblewrap
/// namespace/mount path. If `CORTEXENV_CGROUP_ROOT` points to a writable
/// delegated cgroup v2 subtree, the child is attached before Python source is
/// released from a stop/exec gate. Targets without required Linux wait/signal
/// support do not enable cgroup execution.
pub(crate) async fn execute(request: &ExecutionRequest) -> Result<ExecutionResult, SandboxError> {
    let cgroup_root = std::env::var_os("CORTEXENV_CGROUP_ROOT").map(PathBuf::from);
    let use_linux_namespaces = configured_linux_sandbox()?;
    execute_with_options(request, cgroup_root.as_deref(), use_linux_namespaces).await
}

async fn execute_with_options(
    request: &ExecutionRequest,
    cgroup_root: Option<&Path>,
    use_linux_namespaces: bool,
) -> Result<ExecutionResult, SandboxError> {
    #[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
    if use_linux_namespaces {
        return Err(SandboxError::Isolation);
    }

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
    let output_limit = request.limits.output_bytes;
    let stdout_capture = Arc::new(Mutex::new(Vec::new()));
    let stderr_capture = Arc::new(Mutex::new(Vec::new()));
    let stdout_for_run = Arc::clone(&stdout_capture);
    let stderr_for_run = Arc::clone(&stderr_capture);
    let code = request.code.clone();
    let interpreter = python_executable();
    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    let admission = if let Some(root) = cgroup_root {
        ExecutionAdmission::Prepared(ChildSupervisor.prepare(
            request,
            &ChildSupervisorConfig {
                cgroup_root: Some(root.to_path_buf()),
                require_cgroup: true,
            },
        )?)
    } else {
        ExecutionAdmission::Workspace(ExecutionWorkspace::new()?)
    };
    #[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
    let admission = if cgroup_root.is_some() {
        return Err(SandboxError::Isolation);
    } else {
        ExecutionAdmission::Workspace(ExecutionWorkspace::new()?)
    };
    let workspace_path = admission.workspace_path();
    let execution =
        async move {
            let mut command = python_command(
                interpreter,
                admission.requires_attach(),
                use_linux_namespaces,
            );
            #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
            command.process_group(0);
            command
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
            let mut process_group = ProcessGroupGuard::new(child.id(), Some(admission))?;
            #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
            if process_group.requires_attach() {
                wait_for_child_stop(&mut child).await?;
                process_group.attach_and_release(child.id().ok_or(SandboxError::Isolation)?)?;
                resume_child(child.id().ok_or(SandboxError::Isolation)?)?;
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

fn configured_linux_sandbox() -> Result<bool, SandboxError> {
    let value = match std::env::var("CORTEXENV_ENABLE_LINUX_SANDBOX") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => return Err(SandboxError::Isolation),
    };
    parse_linux_sandbox_setting(value.as_deref())
}

fn parse_linux_sandbox_setting(value: Option<&str>) -> Result<bool, SandboxError> {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("") | Some("0") | Some("false") => Ok(false),
        Some("1") | Some("true") => Ok(true),
        _ => Err(SandboxError::Isolation),
    }
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
enum ExecutionAdmission {
    Prepared(PreparedExecution),
    Workspace(ExecutionWorkspace),
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
impl ExecutionAdmission {
    fn workspace_path(&self) -> PathBuf {
        match self {
            Self::Prepared(prepared) => prepared.workspace().root().to_path_buf(),
            Self::Workspace(workspace) => workspace.root().to_path_buf(),
        }
    }

    fn requires_attach(&self) -> bool {
        matches!(self, Self::Prepared(_))
    }
}

#[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
enum ExecutionAdmission {
    Workspace(ExecutionWorkspace),
}

#[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
impl ExecutionAdmission {
    fn workspace_path(&self) -> PathBuf {
        match self {
            Self::Workspace(workspace) => workspace.root().to_path_buf(),
        }
    }

    fn requires_attach(&self) -> bool {
        false
    }
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
fn python_command(
    interpreter: OsString,
    wait_for_cgroup: bool,
    use_linux_namespaces: bool,
) -> Command {
    let launch_program = if use_linux_namespaces {
        resolve_python_for_shell(OsString::from("bwrap"))
    } else if wait_for_cgroup {
        resolve_python_for_shell(interpreter.clone())
    } else {
        interpreter.clone()
    };
    let mut command = if wait_for_cgroup {
        let mut shell = Command::new("/bin/sh");
        shell.args(["-c", "kill -STOP $$; exec \"$@\"", "cortexenv-python"]);
        shell.arg(launch_program);
        shell
    } else {
        Command::new(launch_program)
    };
    if use_linux_namespaces {
        command.args([
            "--unshare-all",
            "--die-with-parent",
            "--cap-drop",
            "ALL",
            "--ro-bind",
            "/usr",
            "/usr",
            "--ro-bind-try",
            "/lib",
            "/lib",
            "--ro-bind-try",
            "/lib64",
            "/lib64",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--dir",
            "/workspace",
            "--size",
            "67108864",
            "--tmpfs",
            "/workspace",
            "--size",
            "16777216",
            "--tmpfs",
            "/tmp",
            "--remount-ro",
            "/",
            "--chdir",
            "/workspace",
            "--setenv",
            "PATH",
            "/usr/local/bin:/usr/bin:/bin",
            "--",
        ]);
        command.arg(resolve_python_for_shell(interpreter));
    }
    command.args(["-I", "-S", "-"]);
    command
}

#[cfg(not(all(target_os = "linux", not(target_env = "uclibc"))))]
fn python_command(
    interpreter: OsString,
    _wait_for_cgroup: bool,
    _use_linux_namespaces: bool,
) -> Command {
    let mut command = Command::new(interpreter);
    command.args(["-I", "-S", "-"]);
    command
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
    admission: Option<ExecutionAdmission>,
}

#[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
impl ProcessGroupGuard {
    fn new(pid: Option<u32>, admission: Option<ExecutionAdmission>) -> Result<Self, SandboxError> {
        let pid = pid.ok_or(SandboxError::Isolation)?;
        let pid = i32::try_from(pid).map_err(|_| SandboxError::Isolation)?;
        Ok(Self {
            pgid: Pid::from_raw(pid),
            armed: true,
            admission,
        })
    }

    fn requires_attach(&self) -> bool {
        self.admission
            .as_ref()
            .is_some_and(ExecutionAdmission::requires_attach)
    }

    fn attach_and_release(&mut self, pid: u32) -> Result<(), SandboxError> {
        let Some(ExecutionAdmission::Prepared(prepared)) = self.admission.as_mut() else {
            return Err(SandboxError::Isolation);
        };
        prepared.attach_child(pid)?;
        let _permit = prepared.release_source()?;
        Ok(())
    }

    fn terminate(&mut self) -> Result<(), SandboxError> {
        let result = loop {
            match killpg(self.pgid, Signal::SIGKILL) {
                Ok(()) | Err(Errno::ESRCH) => break Ok(()),
                Err(Errno::EINTR) => continue,
                Err(_) => break Err(SandboxError::Isolation),
            }
        };
        if let Some(ExecutionAdmission::Prepared(prepared)) = &self.admission {
            let _ = prepared.kill_children();
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
    fn new(
        _pid: Option<u32>,
        _admission: Option<ExecutionAdmission>,
    ) -> Result<Self, SandboxError> {
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

    #[test]
    fn linux_sandbox_setting_is_explicit_and_rejects_typos() {
        assert!(!parse_linux_sandbox_setting(None).unwrap());
        assert!(!parse_linux_sandbox_setting(Some(" false ")).unwrap());
        assert!(parse_linux_sandbox_setting(Some("true")).unwrap());
        assert!(parse_linux_sandbox_setting(Some("1")).unwrap());
        assert!(matches!(
            parse_linux_sandbox_setting(Some("tru")),
            Err(SandboxError::Isolation)
        ));
    }

    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    #[tokio::test]
    async fn bubblewrap_mode_uses_rootless_linux_namespaces_when_available() {
        use std::os::unix::fs::MetadataExt;

        let bwrap = resolve_python_for_shell(OsString::from("bwrap"));
        if !Path::new(&bwrap).is_file() {
            eprintln!("skipping bubblewrap smoke test: bwrap is not installed");
            return;
        }
        let preflight = std::process::Command::new(&bwrap)
            .args([
                "--unshare-all",
                "--die-with-parent",
                "--cap-drop",
                "ALL",
                "--ro-bind",
                "/usr",
                "/usr",
                "--ro-bind-try",
                "/lib",
                "/lib",
                "--ro-bind-try",
                "/lib64",
                "/lib64",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--dir",
                "/workspace",
                "--size",
                "67108864",
                "--tmpfs",
                "/workspace",
                "--size",
                "16777216",
                "--tmpfs",
                "/tmp",
                "--remount-ro",
                "/",
                "--",
                "/usr/bin/true",
            ])
            .output();
        if !preflight.is_ok_and(|output| output.status.success()) {
            eprintln!("skipping bubblewrap smoke test: rootless namespaces are unavailable");
            return;
        }
        let host_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let host_port = host_listener.local_addr().unwrap().port();
        let namespaces = ["user", "mnt", "pid", "net", "ipc", "uts"];
        let host_ids = namespaces
            .iter()
            .map(|name| {
                (
                    *name,
                    std::fs::metadata(format!("/proc/self/ns/{name}"))
                        .unwrap()
                        .ino(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        let code = format!(
            "import os, socket\n{}\ncaps = next(line.split()[1] for line in open('/proc/self/status') if line.startswith('CapEff:'))\nprint('cap_eff=' + caps)\nprint('root_readonly=' + str(bool(os.statvfs('/').f_flag & 1)))\nprint('usr_readonly=' + str(bool(os.statvfs('/usr').f_flag & 1)))\nopen('/workspace/.namespace-smoke', 'w').write('ok')\nos.unlink('/workspace/.namespace-smoke')\nprint('workspace_write=ok')\nprint('workspace_bytes=' + str(os.statvfs('/workspace').f_bsize * os.statvfs('/workspace').f_blocks))\nprint('tmp_bytes=' + str(os.statvfs('/tmp').f_bsize * os.statvfs('/tmp').f_blocks))\ns = socket.socket()\ns.settimeout(0.2)\nresult = s.connect_ex(('127.0.0.1', {host_port}))\ns.close()\nprint('host_loopback_connect=' + str(result))",
            namespaces
                .iter()
                .map(|name| format!(
                    "print('{}=' + str(os.stat('/proc/self/ns/{}').st_ino))",
                    name, name
                ))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let mut request = request(&code);
        request.limits.timeout_ms = 10_000;

        let result = execute_with_options(&request, None, true).await;
        drop(host_listener);
        let result = match result {
            Ok(result) => result,
            Err(SandboxError::Runtime(message)) if message.contains("No such file") => {
                eprintln!("skipping bubblewrap smoke test: namespace setup is unavailable");
                return;
            }
            other => other.unwrap(),
        };
        assert_eq!(result.exit_code, Some(0), "{result:?}");
        let details = result
            .stdout
            .lines()
            .filter_map(|line| line.split_once('='))
            .collect::<std::collections::HashMap<_, _>>();
        for name in namespaces {
            assert_ne!(details[name].parse::<u64>().unwrap(), host_ids[name]);
        }
        assert_eq!(u64::from_str_radix(details["cap_eff"], 16).unwrap(), 0);
        assert_eq!(details["root_readonly"], "True");
        assert_eq!(details["usr_readonly"], "True");
        assert_eq!(details["workspace_write"], "ok");
        assert!(details["workspace_bytes"].parse::<u64>().unwrap() <= 67_108_864);
        assert!(details["tmp_bytes"].parse::<u64>().unwrap() <= 16_777_216);
        assert_ne!(details["host_loopback_connect"], "0");
    }

    #[cfg(all(target_os = "linux", not(target_env = "uclibc")))]
    #[tokio::test]
    async fn cgroup_gate_waits_for_parent_continue_before_exec() {
        let mut command = python_command(python_executable(), true, false);
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

        let result = execute_with_options(&request, Some(&root), false).await;

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
