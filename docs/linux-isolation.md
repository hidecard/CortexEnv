# Linux isolation boundary

CortexEnv currently executes WASM and JavaScript in the API process. The Python adapter starts a direct interpreter child with a temporary working directory, but does not apply namespaces, cgroups, seccomp, or other OS isolation; network access and descendant processes are not contained. The API must not call `unshare`, change the server's cgroups, or install a process-wide seccomp filter from an HTTP handler, because those operations would affect the API server itself. The safe architecture is a dedicated child supervisor: validate the request, spawn a child, apply isolation before it runs untrusted code, attach it to its cgroup, and supervise its timeout, output, and process tree.

## Implemented foundation

On Linux, `sandbox-core::linux` provides a fail-closed cgroup v2 controller manager:

- `CgroupV2::create` creates a per-execution cgroup only below a mount exposing `cgroup.controllers`.
- `memory.max` is set to the requested memory ceiling.
- `memory.swap.max` is set to `0`.
- `pids.max` is set to the requested process ceiling.
- `attach_pid` writes the child PID to `cgroup.procs`.
- Missing cgroup v2 support or an unwritable controller causes `SandboxError::Isolation`; it never silently falls back to an unrestricted execution.

`linux::isolation_plan` also records the intended child namespace set (user, mount, PID, network, UTS, and IPC), network-deny policy, and runtime seccomp profile selection without changing the current process.

## Required child-supervisor sequence

1. Validate the request and create a private execution workspace.
2. Create the cgroup and write the memory, swap, and PID limits.
3. Spawn the dedicated child supervisor.
4. In the child, create user/mount/PID/network/UTS/IPC namespaces and map an unprivileged UID/GID.
5. Mount a minimal read-only root with a writable temporary workspace.
6. Set `PR_SET_NO_NEW_PRIVS`, apply the runtime-specific seccomp-BPF profile, and drop capabilities.
7. Attach the child PID to `cgroup.procs` before starting the runtime.
8. Supervise output, deadline, exit status, and cleanup from the parent.

The namespace and seccomp steps are intentionally still marked pending in `TODO.md`: implementing them in the in-process Wasmtime path would weaken isolation rather than improve it. Until the child supervisor is wired in, do not treat CortexEnv as a hostile-code security boundary.
