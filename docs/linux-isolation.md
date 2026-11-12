# Linux isolation boundary

CortexEnv currently executes WASM and JavaScript in the API process. On Linux targets with `waitid(WNOWAIT)` support, Python runs in a child process and gets process-group cleanup. Setting `CORTEXENV_CGROUP_ROOT` opts Python into a delegated cgroup v2 subtree: a trusted shell launcher stops itself, the parent observes that stop and attaches its PID to the cgroup, then resumes it so it can `exec` Python and read user source. CPU, memory, swap, and process-count limits are written before the source runs. Timeout/exit cleanup uses the process group and, when available, `cgroup.kill` to include descendants that changed process group. Without the setting there is no per-execution cgroup; without `cgroup.kill`, process-group escape remains possible. The Python flow still does not apply namespaces, seccomp, capability dropping, or host network/filesystem isolation. The API must not call `unshare`, change the server's cgroups, or install a process-wide seccomp filter from an HTTP handler, because those operations would affect the API server itself. A complete child supervisor is still required, especially for WASM and JavaScript.

## Implemented foundation

On Linux, `sandbox-core::linux` provides a fail-closed cgroup v2 controller manager:

- `CgroupV2::create` creates a per-execution cgroup only below a mount exposing `cgroup.controllers`.
- `memory.max` is set to the requested memory ceiling.
- `memory.swap.max` is set to `0`.
- `pids.max` is set to the requested process ceiling.
- `cpu.max` is set from `cpu_quota_millis` using a 100 ms period (`1000` milli-CPUs equals one CPU core).
- `attach_pid` writes the child PID to `cgroup.procs`.
- The Python adapter only enables cgroup-backed execution when `CORTEXENV_CGROUP_ROOT` is explicitly set; it waits for a trusted launcher to stop, attaches the launcher, then sends `SIGCONT` before user code is read.
- `CgroupV2::kill_all` writes `1` to `cgroup.kill` when available, and drop cleanup waits briefly for `cgroup.events` to report `populated 0` before removing the group.
- `PreparedExecution::attach_child` attaches the future supervisor PID to the admitted cgroup before source release, while `PreparedExecution::kill_children` provides the parent-side cgroup-wide cancellation hook.
- `PreparedExecution::release_source` returns a `ChildLaunchPermit` only after a non-zero child PID has been attached; the future supervisor must hold this permit before writing source or starting the runtime.
- Missing cgroup v2 support or an unwritable controller causes `SandboxError::Isolation`; it never silently falls back to an unrestricted execution.
- On supported Linux targets, the Python adapter uses a distinct process group and kills same-group descendants on timeout and leader exit before reaping the leader. This is best-effort process cleanup, not a containment boundary; a descendant may escape the group.

`linux::isolation_plan` also records the intended child namespace set (user, mount, PID, network, UTS, and IPC), network-deny policy, and runtime seccomp profile selection without changing the current process.

## Child-hardening foundation

`sandbox-core::linux::apply_child_isolation` is now the child-only hardening entry point. It validates the request, requests the user/mount/PID/network/UTS/IPC namespaces, applies `RLIMIT_NOFILE`, sets `PR_SET_NO_NEW_PRIVS`, and clears effective, permitted, inheritable, ambient, and bounding capabilities. It is not called from the API process and it does not launch a runtime.

The function currently fails closed when seccomp is required because the pinned `seccompiler` 0.1.0 dependency in this repository is a placeholder without a BPF filter API. This is intentional: the code must not claim seccomp protection or run user code without a real filter. The next implementation step is to replace that dependency/backend and wire this entry point into a dedicated child supervisor with UID/GID mapping, mount setup, cgroup attachment, and parent-side lifecycle supervision.

## Production safety gate

`CORTEXENV_SECURITY_MODE` controls whether the current in-process adapters may run. `trusted` is the backwards-compatible development default. `restricted`, `production`, and `hostile-code` cause `Runtime::new` to fail with `SandboxError::Isolation` before the API service can start; the execute-time check remains defense in depth for callers that already hold a runtime. This is required because the dedicated child supervisor and real runtime-specific seccomp profile are not yet wired. Production deployments must set `CORTEXENV_SECURITY_MODE=restricted`; changing the value to `trusted` is an explicit decision to run only trusted local code and is not a hostile-code security boundary.

## Required child-supervisor sequence

1. Validate the request and create a private execution workspace.
2. Create the cgroup and write the CPU, memory, swap, and PID limits.
3. Spawn the dedicated child supervisor.
4. In the child, create user/mount/PID/network/UTS/IPC namespaces and map an unprivileged UID/GID.
5. Mount a minimal read-only root with a writable temporary workspace.
6. Set `PR_SET_NO_NEW_PRIVS`, apply the runtime-specific seccomp-BPF profile, and drop capabilities.
7. Attach the child PID to `cgroup.procs` before starting the runtime.
8. Supervise output, deadline, exit status, and cleanup from the parent.

The namespace and seccomp steps are intentionally still marked pending in `TODO.md`: implementing them in the in-process Wasmtime path would weaken isolation rather than improve it. Until the child supervisor is wired in, do not treat CortexEnv as a hostile-code security boundary.
