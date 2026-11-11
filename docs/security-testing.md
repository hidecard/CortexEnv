# Security Testing Checklist

This checklist records what can be verified against the current implementation and what must remain blocked until an OS-isolated child supervisor exists. Completing this checklist document is not a penetration-test certification and does not make CortexEnv a hostile-code boundary.

## Safety rules

- Run security tests only in a disposable, unprivileged development VM or CI runner. Do not test against a production service or with production credentials.
- Do not run destructive payloads, fork bombs, kernel exploits, or unrestricted network probes on the host.
- Do not grant the API container elevated privileges, the Docker socket, host namespaces, or broad host mounts for these tests.
- Keep test results, logs, and generated request bodies free of API keys, JWT secrets, and other credentials.
- Until the child supervisor and OS isolation controls are implemented, use only benign malformed-input and policy-validation tests. Never submit hostile code.

## Automated checks currently available

Run from the repository root:

```bash
cargo fmt --all -- --check
cargo test --workspace
```

The current tests cover request validation, backward-compatible decoding and range checks for CPU quotas, cgroup `cpu.max` mapping, denied-by-default policy planning, traversal rejection in the temporary-workspace path resolver, API-key/JWT protection, request IDs, async duplicate IDs, principal-scoped async job ownership, bounded global and per-principal execution admission/history/rate limiting, sanitized runtime errors, WASM timeout interruption, JavaScript console capture/error/deadline behavior, and Python stdout/stderr/exit status/deadline/output caps plus unsupported network-policy rejection. Linux tests verify the Python launcher waits for the parent to attach it before `exec`, and missing cgroup controller files fail before user source runs. On Linux targets with `waitid(WNOWAIT)` support, Python timeout/exit cleanup kills descendants in its process group; a configured cgroup additionally uses `cgroup.kill` when available. Without a configured cgroup or supported `cgroup.kill`, escaped descendants and host network/filesystem isolation are not contained.

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

## Linux isolation checks — blocked pending child supervisor

The following are **not yet safe or meaningful to claim as implemented**. Leave them unchecked until execution is moved into a dedicated child process and the corresponding controls are wired before untrusted code runs:

- [ ] Verify PID, mount, user, network, UTS, and IPC namespaces from inside the child.
- [ ] Verify unprivileged UID/GID mapping and rootless operation.
- [ ] Verify a minimal read-only root filesystem, writable per-job workspace, symlink/path traversal protection, and cleanup after success, timeout, and crash.
- [ ] Verify cgroup v2 memory, swap, PID, CPU, and file-descriptor limits against the child process, including cleanup after forced termination.
- [ ] Verify `PR_SET_NO_NEW_PRIVS`, capability dropping, and per-runtime seccomp profiles with allow/deny syscall tests.
- [ ] Verify default-deny network behavior and any explicitly supported loopback/allowlist policy from inside the child, including DNS and IPv4/IPv6 cases.
- [ ] Verify parent-enforced wall-clock timeout, hard child termination, bounded stdout/stderr capture, and output backpressure.
- [ ] Attempt namespace, filesystem, process, and network escape tests only in a disposable VM designed for hostile-code testing; record kernel, container runtime, and configuration versions.

## Release gate

Do not advertise production isolation or accept hostile multi-tenant workloads until every applicable child-supervisor check above has an automated test, the tests run in the supported deployment environment, and failures are fail-closed. The current `IsolationPlan`, cgroup helper, and traversal-safe resolver are foundations, not proof of a complete sandbox boundary.
