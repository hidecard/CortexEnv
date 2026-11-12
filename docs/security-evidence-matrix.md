# CortexEnv security evidence matrix

This matrix separates implemented mechanisms from evidence that requires a real child supervisor and a disposable Linux environment. A unit test that writes a simulated cgroup file is not evidence of a kernel-enforced hostile-code boundary.

| Control | Current implementation | Evidence status | Release impact |
| --- | --- | --- | --- |
| Request validation before source release | `ChildSupervisor::prepare` validates before workspace/cgroup setup | Unit test passes | Required for all modes |
| Private execution workspace | `ExecutionWorkspace` owns a temporary directory and rejects traversal | Unit tests pass | Required for all modes |
| cgroup v2 pre-admission | `CgroupV2::create` validates controller presence and writes limits before launch | Unit tests pass; host preflight still required | Blocks restricted/production if unavailable |
| Child PID attachment | `CgroupV2::attach_pid` writes `cgroup.procs`; prepared execution exposes the handoff | Simulated attach test passes; real spawn handoff pending | Do not claim complete supervisor |
| CPU, memory, swap, and pids limits | `cpu.max`, `memory.max`, `memory.swap.max=0`, and `pids.max` are written | File-write tests pass; kernel exhaustion tests pending | Blocks hostile-code claim |
| cgroup kill | `cgroup.kill` is used during cleanup when available | Simulated controller test passes; descendant test pending | Required for cancellation |
| User/mount/PID/network namespaces | Child-side namespace flags exist in the hardening primitive | No real child integration evidence | Blocks restricted/production |
| UID/GID mapping and read-only root | Not implemented as a complete launch handoff | No evidence | Blocks restricted/production |
| `no_new_privs` and capability drop | Child-side primitive calls the relevant controls | No real child integration evidence | Blocks hostile-code claim |
| seccomp BPF | Intentionally fails closed because the pinned backend is not a usable filter implementation | Negative fail-closed test only | Execution must be blocked when required |
| Network deny and filesystem boundary | Policy and workspace validation exist, but kernel boundary is pending | No adversarial kernel evidence | Blocks restricted/production |
| Process-tree cancellation | Runtime-specific cleanup exists; cgroup-wide cleanup is not integrated with a real supervisor | Partial unit evidence | Blocks reliable production execution |

## Evidence rules

Evidence must identify the commit, kernel/runtime version, test command, disposable environment, and pass/fail output. Tests that submit code to the API are not sufficient for kernel-boundary claims. Namespace, seccomp, resource-exhaustion, and privilege tests must run in an unprivileged disposable Linux VM or isolated CI runner.

Until the real child supervisor and the corresponding adversarial tests pass, CortexEnv may be used in trusted development mode only. Restricted, production, and hostile-code execution must fail closed rather than relying on these simulated or partial controls.
