# Security Testing Checklist

This checklist records what can be verified against the current implementation and what remains unverified until isolation is complete across runtimes and supported deployments. Completing this checklist document is not a penetration-test certification and does not make CortexEnv a hostile-code boundary.

## Safety rules

- Run security tests only in a disposable, unprivileged development VM or CI runner. Do not test against a production service or with production credentials.
- Do not run destructive payloads, fork bombs, kernel exploits, or unrestricted network probes on the host.
- Do not grant the API container elevated privileges, the Docker socket, host namespaces, or broad host mounts for these tests.
- Keep test results, logs, and generated request bodies free of API keys, JWT secrets, and other credentials.
- Until the full child-supervisor and OS isolation matrix is implemented, use only explicitly benign, non-destructive tests such as policy checks and the conditional namespace smoke test. Never submit hostile code.

## Automated checks currently available

Run from the repository root:

```bash
cargo fmt --all -- --check
cargo test --workspace
```

The current tests cover request validation, backward-compatible decoding and range checks for CPU quotas, cgroup `cpu.max` mapping, denied-by-default policy planning, traversal rejection in the temporary-workspace path resolver, API-key/JWT protection, request IDs, async duplicate IDs, principal-scoped async job ownership, bounded global and per-principal execution admission/history/rate limiting, sanitized runtime errors, WASM timeout interruption and request-bounded linear-memory growth, JavaScript console capture/error/deadline behavior, and Python stdout/stderr/exit status/deadline/output caps plus unsupported network-policy rejection. Linux tests verify the Python launcher waits for the parent to attach it before `exec`, and missing cgroup controller files fail before user source runs. On Linux targets with `waitid(WNOWAIT)` support, Python timeout/exit cleanup kills descendants in its process group; a configured cgroup additionally uses `cgroup.kill` when available. A conditional benign bubblewrap smoke test checks all six namespace identities, zero child capabilities, read-only root and `/usr`, 64 MiB workspace and 16 MiB `/tmp` bounds, workspace writes, and inability to connect to a host loopback listener; it skips when bubblewrap or rootless namespaces are unavailable. Without the namespace opt-in, or without a configured cgroup/supported `cgroup.kill`, escaped descendants and host network/filesystem isolation are not contained.

The `sandbox-core::security` tests cover parsing `trusted`, `restricted`/`production`, and `hostile-code` modes and verify that only trusted development mode permits the current in-process adapters. A deployment smoke check should set `CORTEXENV_SECURITY_MODE=restricted` and confirm execution is rejected before runtime startup.

## Phase 1 smoke scripts

The following scripts are safe prerequisite probes, not a production security certification:

```bash
# Unit checks for the fail-closed child-hardening and cgroup foundations.
cargo test -p sandbox-core linux::supervisor::tests -- --nocapture
cargo test -p sandbox-core linux::cgroup::tests -- --nocapture

# Kernel seccomp primitive in a forked child. No runtime code is executed.
python3 scripts/security/seccomp-kernel-smoke.py

# cgroup v2 controller and delegated-root preflight. Read-only.
CORTEXENV_RUN_ISOLATION_SMOKE=1 bash scripts/security/cgroup-v2-smoke.sh

# Namespace visibility and unprivileged user mapping in a forked child.
# Run only in a disposable unprivileged Linux VM/CI runner.
CORTEXENV_RUN_ISOLATION_SMOKE=1 bash scripts/security/phase1-isolation-smoke.sh

# Run all of the above through one guarded command.
bash scripts/security/run-phase1-smoke.sh
```

Exit status `77` means a kernel feature was unavailable or refused by the container and was reported as `SKIP`; it must not be converted into a passing production-isolation result. The cgroup script checks that `cpu`, `memory`, and `pids` controllers plus the required limit files exist; it does not mutate the host cgroup tree. The namespace script compares the child namespace inode identities with the host PID 1 namespaces and checks nested PID visibility. The seccomp script checks `PR_SET_NO_NEW_PRIVS` followed by `SECCOMP_MODE_STRICT` only in its child. It does not test CortexEnv's runtime-specific BPF profile because that backend remains intentionally fail-closed until a real filter implementation is added.

## API negative and capacity checks

Use a disposable local instance and non-secret test credentials. Verify:

- [ ] Requests to execution routes are rejected with `401` when `CORTEXENV_API_KEY` is configured and no valid key is supplied.
- [ ] Empty or whitespace-only `CORTEXENV_API_KEY`/`CORTEXENV_JWT_SECRET` values behave as unset; never allow an empty `Authorization` or `x-api-key` credential to authenticate.
- [ ] `/health` remains available without API credentials; keep `/metrics` on a private network or behind an authenticated proxy in deployed environments.
- [ ] Malformed JSON, missing/invalid request fields, and malformed WASM produce client-safe errors without host paths or internal engine diagnostics.
- [ ] JavaScript/Python syntax/runtime errors return the expected exit status and bounded stderr; verify no host paths or internal diagnostics are disclosed by the API.
- [ ] Python requests for `loopback` or `allowlist` network access are rejected; do not interpret `network: deny` as host-enforced isolation.
- [ ] A request body larger than 16 MiB is rejected before JSON parsing.
- [ ] Sending requests beyond the configured global rate limit returns `429`; avoid using a public/shared service for this check.
- [ ] With `CORTEXENV_PER_KEY_REQUESTS_PER_SECOND=1` and `CORTEXENV_PER_KEY_REQUEST_BURST=1`, verify a JWT subject's next request receives `429` and a `Retry-After` header, while a different valid subject has its own allowance.
- [ ] Verify idle per-principal buckets expire after 10 minutes; when the 10,000-entry cap is reached, confirm unknown principals are rejected instead of growing the map without bound.
- [ ] With `CORTEXENV_MAX_CONCURRENT_EXECUTIONS=1`, hold one benign execution in progress and verify another request receives `503 Service Unavailable`.
- [ ] With authentication enabled and `CORTEXENV_MAX_EXECUTIONS_PER_KEY=1`, hold one benign execution for a principal and verify another gets `429`, while a different JWT `sub` can execute; verify the quota slot is reclaimed after completion or cancellation.
- [ ] Reusing an async job UUID returns `409 Conflict` and does not replace the original status.
- [ ] With two distinct JWT subjects, verify each can read/stream its own async job but receives `404` for the other subject's job; verify the shared static API key behaves as one principal.
- [ ] With `CORTEXENV_MAX_STORED_EXECUTIONS=1`, complete multiple benign async jobs and verify the oldest completed result is evicted while a running job remains addressable.
- [ ] When a result is evicted, both the status endpoint and WebSocket stream report it as not found; document this retention behavior for clients.

## Linux isolation checks — partial implementation; release gate remains blocked

The smoke test below covers only the opt-in Python bubblewrap path on hosts that pass its namespace preflight. It is benign, is not an escape test, and does not certify the deployment image or other runtimes. Keep the hostile-code prohibition above until the full supervisor matrix and release gate pass.

- [x] Conditional smoke test verifies Python runs in distinct user, mount, PID, network, UTS, and IPC namespaces when bubblewrap and rootless user namespaces are available.
- [ ] Verify namespace setup and fail-closed behavior in every supported container/deployment environment; JavaScript and WASM still run in-process.
- [ ] Verify unprivileged UID/GID mapping and rootless operation.
- [ ] Verify the complete minimal root filesystem, writable per-job workspace, symlink/path traversal protection, and cleanup after success, timeout, and crash.
- [ ] Verify cgroup v2 memory, swap, PID, CPU, and file-descriptor limits against the child process, including cleanup after forced termination.
- [ ] Verify `PR_SET_NO_NEW_PRIVS` and per-runtime seccomp profiles with allow/deny syscall tests (the current Python bubblewrap path drops capabilities but does not install seccomp).
- [ ] Verify default-deny network behavior and any explicitly supported loopback/allowlist policy across runtimes, including DNS and IPv4/IPv6 cases; only a local-loopback separation smoke check currently exists for Python.
- [ ] Verify parent-enforced wall-clock timeout, hard child termination, bounded stdout/stderr capture, and output backpressure.
- [ ] Attempt namespace, filesystem, process, and network escape tests only in a disposable VM designed for hostile-code testing; record kernel, container runtime, and configuration versions.

## Release gate

Do not advertise production isolation or accept hostile multi-tenant workloads until every applicable child-supervisor check above has an automated test, the tests run in the supported deployment environment, and failures are fail-closed. The current `IsolationPlan`, cgroup helper, and traversal-safe resolver are foundations, not proof of a complete sandbox boundary.
