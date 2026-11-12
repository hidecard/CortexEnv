# Runtime Profiles and Capability Discovery

`sandbox-runtime` exposes static capability profiles for the currently registered adapters through `runtime_profiles()` and `Runtime::profile(language)`. These profiles describe what the adapter can do today; they are not a hostile-code security certification.

## Current profiles

| Language | Adapter | Input | Child process | Timeout | Runtime-local memory limit | Accepted network policy | Hostile-code boundary |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `wasm` | Wasmtime | Hexadecimal module bytes | No | Yes, per-execution epoch deadline | Yes, guest linear memory | `deny` | No |
| `javascript` | `deno_core` | UTF-8 source text | No | Yes, deadline/interrupt behavior | No complete OS memory limit | `deny` | No |
| `python` | `python3` | UTF-8 source text | Yes | Yes, supervised child deadline | Yes when optional cgroup v2 is configured | `deny` | No |

Python uses `-I -S -`, clears the child environment, and uses a process group. Setting `CORTEXENV_ENABLE_LINUX_SANDBOX=true` opts into bubblewrap user/mount/PID/network/IPC/UTS namespaces, a read-only root/system view, bounded private tmpfs at `/tmp` and `/workspace`, and capability dropping; setup failure has no unrestricted fallback. The namespace `/workspace` is ephemeral and does not mount host input/output artifacts. When `CORTEXENV_CGROUP_ROOT` points to a writable delegated cgroup v2 subtree, it separately uses the prepared admission lifecycle: workspace/cgroup creation, stopped-launcher attach, source-release permit, and cgroup cleanup.

## Rust discovery API

```rust
use sandbox_common::Language;
use sandbox_runtime::{runtime_profiles, Runtime};

let all = runtime_profiles();
let python = Runtime::profile(Language::Python).expect("Python is registered");
assert!(python.child_process);
assert_eq!(python.network, "deny");
```

The profile fields intentionally expose limitations such as `hostile_code_boundary: false`. Callers must not infer that `network: deny` blocks host networking unless the optional Python bubblewrap network namespace is enabled. The mode is partial: runtime-specific seccomp, full filesystem policy, deployment-matrix verification, and JavaScript/WASM isolation remain incomplete until the dedicated child supervisor is finished.

## Security-mode interaction

`CORTEXENV_SECURITY_MODE=trusted` permits the current adapters for trusted development. `restricted`, `production`, and `hostile-code` fail during runtime startup because the profiles currently report no complete hostile-code boundary.
