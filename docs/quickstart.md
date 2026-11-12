# CortexEnv Quickstart and Usage Guide

This guide walks through a safe local development flow for CortexEnv v0.1. It is intentionally explicit about the difference between **trusted development mode** and the incomplete production isolation boundary.

> **Security warning:** The current WASM, JavaScript, and Python adapters are not a complete hostile-code sandbox. Use `trusted` mode only with code and data you trust. Do not expose the service to arbitrary public submissions. `restricted`, `production`, and `hostile-code` modes fail closed until the dedicated Linux child supervisor is complete.

## 1. Get the source and check prerequisites

Linux is the recommended development environment. Rust stable and Cargo are required. Python 3 is required only when executing Python requests. Bubblewrap and a host/container that permits unprivileged user namespaces are required only for the optional Python namespace mode. Docker is optional for the container flow.

```bash
git clone https://github.com/hidecard/CortexEnv.git
cd CortexEnv

# Optional if Rust is not installed.
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

rustc --version
cargo --version
python3 --version
```

## 2. Select the security mode explicitly

For local development with trusted code, set:

```bash
export CORTEXENV_SECURITY_MODE=trusted
```

This is the compatibility/development mode. It does not provide the complete child supervisor or seccomp boundary. Python can separately opt into partial bubblewrap namespaces with `CORTEXENV_ENABLE_LINUX_SANDBOX=true`; this does not enable isolation for WASM or JavaScript and is not a hostile-code boundary.

For a production-oriented configuration, use:

```bash
export CORTEXENV_SECURITY_MODE=restricted
```

At the current implementation stage this intentionally stops runtime startup with an isolation error. That is safer than silently running the in-process adapters as if they were production-isolated.

## 3. Format and test the workspace

Run the same checks used before a pull request or release:

```bash
cargo fmt --all -- --check
cargo test --workspace
```

To test only the Linux isolation foundations:

```bash
cargo test -p sandbox-core
```

## 4. Run the CLI WASM smoke test

Create the minimal valid empty WASM module and execute it with the CLI:

```bash
printf '0061736d01000000' > /tmp/empty.wasm.hex
cargo run -p sandbox-cli --bin cortexenv -- --file /tmp/empty.wasm.hex
```

The result is JSON containing the execution ID, exit code, stdout, stderr, timeout state, duration, and completion timestamp. Add a custom timeout when needed:

```bash
cargo run -p sandbox-cli --bin cortexenv -- \
  --file /tmp/empty.wasm.hex \
  --timeout-ms 1000
```

The CLI is a WASM smoke-test tool. The API and Rust runtime dispatcher also support JavaScript and Python.

## 5. Start the local API server

Start the server in one terminal:

```bash
export CORTEXENV_SECURITY_MODE=trusted
cargo run -p sandbox-api --bin cortexenv-server
```

The default bind address is `127.0.0.1:8080`. Override it with `CORTEXENV_BIND`:

```bash
CORTEXENV_BIND=127.0.0.1:9090 cargo run -p sandbox-api --bin cortexenv-server
```

Check health from a second terminal:

```bash
curl -i http://127.0.0.1:8080/health
```

`GET /health` is public and is intended for local checks and container health probes.

## 6. Submit synchronous executions

A minimal WASM request uses hexadecimal module bytes:

```bash
curl -sS http://127.0.0.1:8080/v1/execute \
  -H 'content-type: application/json' \
  -d '{
    "language": "wasm",
    "code": "0061736d01000000",
    "limits": {
      "timeout_ms": 5000,
      "memory_bytes": 134217728,
      "process_count": 32,
      "file_descriptors": 64,
      "output_bytes": 1048576
    },
    "network": "deny"
  }'
```

JavaScript and Python requests use source text:

```bash
curl -sS http://127.0.0.1:8080/v1/execute \
  -H 'content-type: application/json' \
  -d '{"language":"javascript","code":"console.log(\u0027hello from JS\u0027)","network":"deny"}'

curl -sS http://127.0.0.1:8080/v1/execute \
  -H 'content-type: application/json' \
  -d '{"language":"python","code":"print(\u0027hello from Python\u0027)","network":"deny"}'
```

The `id` field is generated when omitted. `network: deny` is the accepted policy for the current Python and JavaScript adapters. Host-level network isolation is enforced for Python only when the opt-in bubblewrap network namespace is enabled; otherwise it is not enforced. JavaScript network isolation remains incomplete.

For a local Linux development run with the partial Python namespace mode, install bubblewrap through the operating system package manager and set the environment variable before starting the API:

```bash
export CORTEXENV_ENABLE_LINUX_SANDBOX=true
cargo run -p sandbox-api --bin cortexenv-server
```

The mode uses user/mount/PID/network/IPC/UTS namespaces, read-only root/system mounts, 16 MiB `/tmp` and 64 MiB `/workspace` tmpfs mounts, and dropped child capabilities. If bubblewrap is absent or the host/container refuses namespace setup, Python execution fails rather than falling back. The namespace `/workspace` is ephemeral; current API requests do not yet provide host-mounted input/output artifacts.

## 7. Submit and follow an asynchronous job

Submit a background execution:

```bash
curl -sS http://127.0.0.1:8080/v1/execute/async \
  -H 'content-type: application/json' \
  -d '{"language":"python","code":"print(\u0027async hello\u0027)","network":"deny"}'
```

The response contains an execution ID. Poll it with:

```bash
curl -sS http://127.0.0.1:8080/v1/executions/<execution-id>
```

For streaming status, use a WebSocket client such as `websocat`:

```bash
websocat 'ws://127.0.0.1:8080/v1/stream?id=<execution-id>'
```

The current job store is process-local and in-memory. Jobs and completed results do not survive a server restart.

## 8. Enable API authentication for local integration tests

Use a static API key for a simple local setup:

```bash
export CORTEXENV_API_KEY='replace-with-a-local-test-key'
cargo run -p sandbox-api --bin cortexenv-server
```

Send the key using either supported header:

```bash
curl -sS http://127.0.0.1:8080/v1/execute \
  -H 'Authorization: Bearer replace-with-a-local-test-key' \
  -H 'content-type: application/json' \
  -d '{"language":"wasm","code":"0061736d01000000","network":"deny"}'
```

For JWT authentication, set a non-blank secret of at least 32 bytes and provide HS256 tokens with a non-empty `sub` and unexpired `exp` claim:

```bash
export CORTEXENV_JWT_SECRET='a-local-development-secret-at-least-32-bytes'
```

The health endpoint remains public. Job ownership, request quotas, and execution quotas are process-local and reset on restart.

## 9. Configure optional Python cgroup controls

On Linux, Python can use a delegated writable cgroup v2 subtree:

```bash
export CORTEXENV_CGROUP_ROOT=/sys/fs/cgroup/cortexenv-delegated
export CORTEXENV_SECURITY_MODE=trusted
cargo run -p sandbox-api --bin cortexenv-server
```

The subtree must expose the required controller files and allow creation of per-execution cgroups. Missing or unwritable controller files fail closed before Python source is released. This optional path applies resource controls and process cleanup; it is not a substitute for namespaces, mount isolation, or seccomp.

The cgroup and bubblewrap settings are independent. If both are enabled, the launcher is attached to the cgroup before bubblewrap starts; cgroup cleanup can also reach descendants that leave their process group when `cgroup.kill` is available.

Check the environment without submitting user code:

```bash
CORTEXENV_RUN_ISOLATION_SMOKE=1 \
  bash scripts/security/cgroup-v2-smoke.sh
```

## 10. Run the Linux isolation smoke tests

Run these only as an unprivileged user inside a disposable Linux VM or isolated CI runner:

```bash
CORTEXENV_RUN_ISOLATION_SMOKE=1 \
  bash scripts/security/run-phase1-smoke.sh
```

The runner performs unit checks and probes namespace visibility, unprivileged user mapping, cgroup v2 prerequisites, and kernel strict-seccomp availability. Exit status `77` means a kernel feature was unavailable or refused by the container and was reported as `SKIP`; it is not a production-isolation pass.

## 11. Run the Docker development flow

Build and start the API image:

```bash
docker compose up --build
curl -i http://127.0.0.1:8080/health
docker compose down
```

Or run the CI-equivalent smoke test:

```bash
bash scripts/docker-smoke-test.sh
```

The image runs as a non-root user with dropped capabilities and `no-new-privileges`, but the application-level hostile-code boundary remains incomplete.

For the partial Python-only bubblewrap path, run Compose with `CORTEXENV_ENABLE_LINUX_SANDBOX=true`. The host/container must allow unprivileged user namespaces; setup failure stops Python execution. The mode does not enable seccomp and is not production isolation.

## 12. Stop, clean up, and troubleshoot

Stop a local server with `Ctrl-C`. Stop Compose and remove its temporary containers with:

```bash
docker compose down
```

Common checks:

- If startup fails with `SandboxError::Isolation`, inspect `CORTEXENV_SECURITY_MODE`; `restricted`, `production`, and `hostile-code` intentionally fail until the dedicated child supervisor is available.
- If Python cannot start, verify `python3` is installed or set `CORTEXENV_PYTHON` to an approved interpreter executable.
- If a cgroup execution fails, verify that `CORTEXENV_CGROUP_ROOT` is a writable delegated v2 subtree with `cpu`, `memory`, and `pids` controllers.
- If an API request returns `400`, inspect the validation message and check timeout, memory, process, file-descriptor, output, language, and network fields.
- If an async job returns `404`, confirm that the request uses the same authenticated principal that submitted the job.

## 13. Read the security and API reference documents

Continue with these documents before integrating CortexEnv into another service:

- [`README.md`](../README.md) — project overview and complete API examples.
- [`openapi-v1.yaml`](openapi-v1.yaml) — frozen HTTP contract.
- [`api-compatibility.md`](api-compatibility.md) — compatibility and deprecation policy.
- [`linux-isolation.md`](linux-isolation.md) — current isolation boundary and supervisor sequence.
- [`security-testing.md`](security-testing.md) — safe checks and blocked hostile-code tests.
- [`product-scope-and-threat-model.md`](product-scope-and-threat-model.md) — supported workloads and security claims.
