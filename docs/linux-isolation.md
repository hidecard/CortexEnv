# Linux isolation boundary

CortexEnv still executes WASM and JavaScript in the API process. On supported Linux targets, Python runs in a child process with process-group cleanup. Setting `CORTEXENV_ENABLE_LINUX_SANDBOX=true` opts Python into bubblewrap: `--unshare-all` creates user, mount, PID, network, IPC, and UTS namespaces; the sandbox root and `/usr` plus available `/lib`/`/lib64` trees are read-only; `/tmp` is a private 16 MiB tmpfs; `/workspace` is a private writable 64 MiB tmpfs; and all child capabilities are dropped. No host workspace path is mounted into the namespace. This mode fails closed if bubblewrap is missing or the host/container disallows namespace setup; it never retries as an unrestricted Python process. Without this opt-in, Python's `network: deny` value is not enforced by the host.

Setting `CORTEXENV_CGROUP_ROOT` separately opts Python into a delegated cgroup v2 subtree: a trusted shell launcher stops itself, the parent observes that stop and attaches its PID to the cgroup, then resumes it so it can `exec` the configured launcher and Python can read user source. CPU, memory, swap, and process-count limits are written before source execution. Timeout/exit cleanup uses the process group and, when available, `cgroup.kill` to include descendants that changed process group. Without the cgroup setting, those resource ceilings are not host-enforced; without `cgroup.kill`, a descendant that escaped its process group may survive. The optional bubblewrap path does not install seccomp, and WASM/JavaScript do not yet use a child supervisor. The API must not call `unshare`, change the server's cgroups, or install a process-wide seccomp filter from an HTTP handler, because those operations would affect the API server itself.

## Implemented foundation

On Linux, `sandbox-core::linux` provides a fail-closed cgroup v2 controller manager:

- `CgroupV2::create` creates a per-execution cgroup only below a mount exposing `cgroup.controllers`.
- `memory.max` is set to the requested memory ceiling.
- `memory.swap.max` is set to `0`.
- `pids.max` is set to the requested process ceiling.
- `cpu.max` is set from `cpu_quota_millis` using a 100 ms period (`1000` milli-CPUs equals one CPU core).
- `attach_pid` writes the child PID to `cgroup.procs`.
- The Python adapter only enables cgroup-backed execution when `CORTEXENV_CGROUP_ROOT` is explicitly set; it now obtains a `ChildSupervisor::prepare` admission, waits for a trusted launcher to stop, attaches the launcher through `PreparedExecution::attach_child`, obtains `ChildLaunchPermit`, then sends `SIGCONT` before user code is read.
- `CgroupV2::kill_all` writes `1` to `cgroup.kill` when available, and drop cleanup waits briefly for `cgroup.events` to report `populated 0` before removing the group.
- `PreparedExecution::attach_child` attaches the future supervisor PID to the admitted cgroup before source release, while `PreparedExecution::kill_children` provides the parent-side cgroup-wide cancellation hook.
- `PreparedExecution::release_source` returns a `ChildLaunchPermit` only after a non-zero child PID has been attached; the future supervisor must hold this permit before writing source or starting the runtime.
- Missing cgroup v2 support or an unwritable controller causes `SandboxError::Isolation`; it never silently falls back to an unrestricted execution.
- On supported Linux targets, the Python adapter uses a distinct process group and kills same-group descendants on timeout and leader exit before reaping the leader. This is best-effort process cleanup, not a containment boundary; a descendant may escape the group.
- With `CORTEXENV_ENABLE_LINUX_SANDBOX=true`, Python is launched through bubblewrap with user/mount/PID/network/IPC/UTS namespaces, a read-only sandbox root and system-library binds, private 16 MiB `/tmp` and 64 MiB `/workspace` tmpfs mounts, and dropped capabilities. A benign conditional test checks namespace identities, capability state, mount/workspace behavior, and separation from a host loopback listener.
- Bubblewrap and cgroup settings are independent. When both are enabled, the stopped launcher is attached to the cgroup before bubblewrap starts; cgroup cleanup can cover descendants that leave the process group only when `cgroup.kill` exists.

`linux::isolation_plan` also records the intended child namespace set (user, mount, PID, network, UTS, and IPC), network-deny policy, and runtime seccomp profile selection without changing the current process.

## Child-hardening foundation

`sandbox-core::linux::apply_child_isolation` is the child-only hardening entry point. It validates resource limits, creates the user namespace and maps the launching identity through `setgroups=deny`, `uid_map`, and `gid_map`, requests the mount/PID/network/UTS/IPC namespaces, applies `RLIMIT_NOFILE`, sets `PR_SET_NO_NEW_PRIVS`, clears effective, permitted, inheritable, ambient, and bounding capabilities, then installs the configured seccomp profile when required. `prepare_mount_namespace` additionally makes `/` private, binds an explicit workspace, and bind-remounts an explicit runtime directory read-only. These helpers are not called from the API process and do not launch a runtime.

The placeholder dependency has been replaced with upstream `seccompiler` 0.5. The current WASM baseline filter denies selected high-risk syscalls with `EPERM` and allows other syscalls by default; it is a denylist, not a complete least-privilege profile. Installation applies to the calling thread, so the entry point must run in a dedicated single-threaded child before runtime startup. No general runtime launch path invokes these helpers yet; pivoting to a minimal root, complete filesystem policy, and parent-side lifecycle supervision remain required before production use.

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

The optional Python bubblewrap path implements part of the namespace/mount sequence only; it is not enabled by default and is not a general supervisor. Runtime-specific seccomp profiles, the complete filesystem policy, deployment-environment verification, and child supervisors for WASM/JavaScript remain pending in `TODO.md`. Until those controls are validated together, do not treat CortexEnv as a hostile-code security boundary.
