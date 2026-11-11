# CortexEnv — Ordered Production Roadmap

This checklist is ordered by dependency. A checked item means the repository currently contains an implementation or documented foundation; it does not by itself mean the feature is production-certified. Do not claim a hostile-code security boundary until the child supervisor, OS isolation controls, and the security integration suite are complete.

## Phase 0: Scope, Threat Model, and Release Policy

- [ ] Define the first supported deployment mode: trusted internal, private enterprise, or public multi-tenant
- [ ] Document the threat model for AI-generated code, user-submitted code, plugins, files, secrets, network access, and tenant isolation
- [ ] Define supported and unsupported workloads for WASM, JavaScript, and Python
- [ ] Define security claims and deployment modes (`trusted`, `restricted`, `hostile-code`) with explicit prerequisites
- [ ] Define the Linux baseline, kernel requirements, container runtime assumptions, and required privileges
- [ ] Define data classification, retention, deletion, and audit requirements
- [ ] Add a security decision record for every capability that can access files, network, secrets, or external tools

## Phase 1: Core Sandbox Engine

- [x] Workspace and all crates created
- [x] `sandbox-common`: `ExecutionRequest`, `ExecutionResult`, `Language`, and `NetworkPolicy`
- [x] `sandbox-common`: error types, default configuration, and bounded resource validation
- [x] `sandbox-core`: resource-limit validation and explicit fail-closed isolation plan
- [x] `sandbox-core`: Linux cgroup v2 controller manager for CPU/memory/swap/pids limits
- [x] `sandbox-runtime`: WASM module loading and `_start` invocation with Wasmtime
- [x] `sandbox-runtime`: WASM wall-clock timeout with Wasmtime epoch interruption
- [x] `sandbox-runtime`: request WASM linear-memory limit with one instance and memory per store
- [x] `sandbox-runtime`: JavaScript support through the `deno_core` adapter
- [x] `sandbox-runtime`: Python child execution with supported Linux process-group cleanup
- [x] `sandbox-runtime`: opt-in cgroup v2 attachment before Python source execution
- [x] `sandbox-cli`: execute a hex-encoded WASM module and print a JSON result
- [x] Basic unit tests for shared types and resource limits
- [ ] Freeze and version the `ExecutionRequest` and `ExecutionResult` API schema
- [ ] Define stable machine-readable error codes and status transitions
- [ ] Document timeout, CPU, memory, process, file-descriptor, disk, and output-limit semantics
- [ ] Reconcile runtime documentation with the implementation and remove contradictory security/runtime claims
- [ ] Add a runtime behavior matrix for WASM, JavaScript, and Python

## Phase 2: Linux Child Supervisor and Strong Isolation

The items in this phase are the security boundary. They must run in a dedicated child/supervisor path rather than changing the API process itself.

- [ ] Implement a fail-closed per-execution child supervisor
- [ ] Validate the request and create the private execution workspace before spawning the child
- [ ] Create and configure the per-execution cgroup before releasing untrusted source to the runtime
- [ ] Attach the child and all descendants to the intended cgroup
- [ ] Implement Linux user namespace and unprivileged UID/GID mapping
- [ ] Implement mount namespace setup
- [ ] Implement PID namespace setup
- [ ] Implement network namespace setup
- [ ] Implement UTS and IPC namespace setup where required by the threat model
- [ ] Implement `chroot`/`pivot_root` or an equivalent minimal root filesystem
- [ ] Mount a read-only runtime root and a writable per-execution workspace
- [ ] Implement OverlayFS or tmpfs workspace policy where supported
- [ ] Set `PR_SET_NO_NEW_PRIVS` before running untrusted code
- [ ] Drop Linux capabilities before running untrusted code
- [ ] Implement runtime-specific seccomp-BPF profiles
- [ ] Enforce default-deny network policy at the host/kernel boundary
- [ ] Implement and document optional loopback/allowlist network policy, if supported
- [ ] Enforce filesystem access only through explicit workspace/input/output mounts
- [ ] Ensure secrets, host paths, Docker sockets, and application data are not inherited
- [ ] Implement process-tree supervision, cancellation, timeout, and descendant cleanup
- [ ] Use cgroup kill support where available for descendants that change process group/session
- [ ] Enforce output backpressure as well as output size caps
- [ ] Fail closed when required Linux isolation controls are missing or misconfigured
- [ ] Keep an explicit trusted-development mode separate from restricted/production mode

## Phase 3: Security and Isolation Verification

Run hostile-code tests only in a disposable, unprivileged Linux VM or isolated CI runner.

- [ ] Add integration tests for namespace visibility and unprivileged identity
- [ ] Add filesystem escape tests for absolute paths, traversal, symlinks, and mount boundaries
- [ ] Add tests for read/write access outside the per-job workspace
- [ ] Add network-deny tests covering TCP, UDP, DNS, IPv4, IPv6, and proxy bypass
- [ ] Add tests for any supported loopback or allowlist network mode
- [ ] Add CPU quota and CPU exhaustion tests
- [ ] Add memory and swap limit tests
- [ ] Add PID/fork-bomb and descendant cleanup tests
- [ ] Add file-descriptor exhaustion tests
- [ ] Add disk/temp-file quota tests
- [ ] Add stdout/stderr flood and output-backpressure tests
- [ ] Add timeout tests during compilation, startup, execution, and shutdown
- [ ] Add seccomp deny tests for runtime-specific forbidden syscalls
- [ ] Add capability and privilege-escalation tests
- [ ] Add API-process survival tests after child crash, kill, and resource exhaustion
- [ ] Add concurrent-execution isolation tests
- [ ] Add cross-principal job/result access tests
- [ ] Add fuzz tests for request parsing, WASM input, and runtime adapters
- [ ] Add dependency audit, SBOM generation, image scan, and secret scanning
- [ ] Map every security claim to an automated test and recorded evidence
- [ ] Publish known limitations and a security disclosure policy
- [ ] Complete an independent security review before public hostile-code workloads

## Phase 4: API Server and Execution Reliability

- [x] Axum router skeleton with `/health` and `/v1/execute`
- [x] Runnable server binary and graceful shutdown
- [x] `POST /v1/execute/async` with job ID
- [x] `GET /v1/executions/:id`
- [x] WebSocket `/v1/stream`
- [x] Request-validation middleware and request IDs
- [x] Global API rate limiting
- [x] Bounded concurrent execution admission and overload responses
- [x] Duplicate async ID protection
- [x] Bounded completed-job history with oldest-result eviction
- [x] In-memory result storage
- [x] Authenticated async job ownership checks for status and WebSocket routes
- [ ] Freeze and publish an OpenAPI specification for API v1
- [ ] Add explicit job states: queued, starting, running, completed, failed, timed out, cancelled, expired
- [ ] Add job cancellation API and supervisor cancellation path
- [ ] Add idempotency keys and safe retry behavior
- [ ] Add queue/backpressure behavior separate from execution workers
- [ ] Add warm worker pool and idle timeout after isolation semantics are stable
- [ ] Add worker health checks, heartbeat, orphan-job recovery, and graceful draining
- [ ] Add pagination/filtering for execution history
- [ ] Add callback/webhook support for completed asynchronous jobs

## Phase 5: Durable Platform, Tenancy, and Accounting

- [x] Optional static API-key authentication
- [x] Optional JWT authentication
- [x] Per-authenticated-principal concurrent execution quota
- [x] In-memory per-authenticated-principal request rate limit
- [x] In-process principal-scoped async-job ownership boundary
- [ ] Define organization, user, team, project, service-account, and API-key data models
- [ ] Add durable database-backed execution/job records
- [ ] Add durable artifact metadata and object-storage integration
- [ ] Add durable usage and quota accounting
- [ ] Add distributed rate limits and concurrent-execution quotas
- [ ] Add per-user, per-team, per-project, and organization policy inheritance
- [ ] Add tenant-isolation tests across jobs, files, artifacts, logs, and metrics
- [ ] Add job/result retention and deletion policies
- [ ] Add idempotent database migrations and rollback guidance
- [ ] Add backup, restore, and disaster-recovery verification
- [ ] Add secret-management integration and secret-redaction tests
- [ ] Add RBAC for user, developer, operator, and administrator roles

## Phase 6: Observability and Production Operations

- [x] Observability dependency and tracing hooks from Phase 1
- [x] Prometheus-compatible metrics and execution counters
- [x] Health checks and graceful shutdown
- [x] Multi-stage non-root Dockerfile and Compose deployment
- [x] Architecture documentation
- [ ] Add OpenTelemetry traces across API, queue, supervisor, runtime, and storage
- [ ] Add structured JSON logs with execution, tenant, request, and trace correlation
- [ ] Add metrics for queue latency, cold starts, runtime duration, timeouts, isolation failures, and resource kills
- [ ] Add dashboards and alert rules
- [ ] Keep metrics private or protect them with an authenticated proxy
- [ ] Add startup configuration validation and safe defaults
- [ ] Add TLS/reverse-proxy deployment guidance
- [ ] Add container/image signing and pinned release artifacts
- [ ] Add Linux VM deployment guide
- [ ] Add Kubernetes/Helm deployment after the single-node deployment is stable
- [ ] Add Terraform or equivalent infrastructure examples
- [ ] Add incident-response, upgrade, rollback, and capacity-planning runbooks
- [ ] Define SLOs for API availability, job completion, latency, and isolation failures

## Phase 7: Stateful Agent and Workspace

- [x] Provider-neutral `execute_code` tool trait
- [x] Provider-neutral LLM client abstraction
- [x] OpenAI-compatible Chat Completions client
- [x] Bounded ReAct/self-correction loop
- [ ] Add stateful agent sessions
- [ ] Add persistent per-session workspace with expiration and storage quota
- [ ] Add conversation and tool-call history
- [ ] Add file/artifact awareness to agent tools
- [ ] Add package/environment templates with allowlisted dependencies
- [ ] Add agent time, step, token, and execution budgets
- [ ] Add approval gates for network, external tools, secrets, and destructive actions
- [ ] Add prompt-injection boundaries for untrusted execution output
- [ ] Add end-to-end self-healing test: error → repair → rerun → final result
- [ ] Add agent failure recovery and partial-result behavior
- [ ] Add model/provider configuration without hard-coded secrets

## Phase 8: End-User Web Application

The repository currently provides an engine/API, not a complete user-facing GUI. Build the GUI as a separate frontend/application layer against the versioned API.

- [ ] Create a web frontend project with a versioned CortexEnv API client
- [ ] Add login, logout, session management, and organization selection
- [ ] Add OIDC/SSO option for enterprise deployments
- [ ] Add project and workspace creation
- [ ] Add code editor with WASM, JavaScript, and Python selection
- [ ] Add run, stop, cancel, retry, and rerun controls
- [ ] Add live stdout/stderr console with reconnect/polling fallback
- [ ] Add clear exit-status, timeout, resource-limit, and isolation-error displays
- [ ] Add file upload, file browser, file preview, and download
- [ ] Add artifact list and result download
- [ ] Add execution history with search, filters, pagination, and retention information
- [ ] Add AI chat interface with file attachments and tool/execution activity
- [ ] Add table, chart, image, and report result rendering
- [ ] Add user/team usage and quota dashboard
- [ ] Add API-key management for authorized users
- [ ] Add administrator screens for users, policies, jobs, quotas, and audit logs
- [ ] Add role-aware routing and frontend permission checks
- [ ] Add Burmese/English localization
- [ ] Add responsive layout and accessibility checks
- [ ] Add frontend unit, integration, and end-to-end tests
- [ ] Add error/reconnect states for WebSocket, API, worker, and storage failures

## Phase 9: SDKs and Integrations

- [x] Cross-platform `cortexenv` binary name
- [x] Windows release build (`cortexenv-windows-x86_64.exe`)
- [x] Debian packaging metadata with `cargo-deb`
- [x] Tag-based GitHub Release workflow for Windows and Debian artifacts
- [ ] Publish Python SDK
- [ ] Publish TypeScript/JavaScript SDK
- [ ] Publish Rust SDK usage examples
- [ ] Generate clients from OpenAPI
- [ ] Improve CLI with `execute`, `jobs`, `sessions`, `files`, and `artifacts` commands
- [ ] Add MCP server for execution and artifact tools
- [ ] Add VS Code extension or developer-console integration
- [ ] Add Jupyter integration for private data analysis
- [ ] Add n8n/Temporal/Airflow integration examples
- [ ] Add LangChain/LangGraph adapter examples
- [ ] Add GitHub Actions/GitLab CI execution action
- [ ] Add webhook and event-consumer examples
- [ ] Add API compatibility and SDK versioning policy

## Phase 10: Product MVP and Trusted Pilot

Choose one reference product before expanding into a general platform.

- [ ] Choose a first product: Private AI Data Analyst, Private Coding Agent, Education Coding Lab, or Secure Plugin Runtime
- [ ] Define the first user persona and top five user workflows
- [ ] Build a complete happy-path demo from login to result/export
- [ ] Add product-specific templates, prompts, policies, and sample data
- [ ] Add onboarding and in-product help
- [ ] Add privacy, data-retention, and acceptable-use notices
- [ ] Run a trusted pilot with controlled users and workloads
- [ ] Measure task success, completion rate, latency, timeout rate, cost, and failure recovery
- [ ] Collect security, reliability, and usability feedback
- [ ] Fix pilot blockers and document operating procedures
- [ ] Run an external beta only after the applicable security release gate passes
- [ ] Define support, issue triage, release cadence, and compatibility policy

## Phase 11: Release and End-User Readiness Gate

A release can be called end-user ready only when all applicable items below are complete.

- [ ] Security mode and supported deployment are clearly documented
- [ ] Required isolation controls fail closed when unavailable
- [ ] Security integration tests pass in the supported deployment environment
- [ ] API v1 and SDK contracts are versioned
- [ ] Jobs, quotas, users, artifacts, and audit events survive service restart
- [ ] Backup and restore have been tested
- [ ] Monitoring, alerts, and runbooks exist
- [ ] GUI supports login, workspace, upload, execution, output, history, and export
- [ ] Error handling and reconnect behavior are tested
- [ ] Privacy, retention, deletion, and acceptable-use policies are published
- [ ] Installation and upgrade documentation is complete
- [ ] A reference product has passed a trusted pilot
- [ ] Release artifacts are reproducible, scanned, and signed where supported
- [ ] Security limitations are stated honestly in README and product documentation

## Product Goal

The intended end state is not only a Rust runtime. It is a self-hosted execution platform for private AI agents and enterprise workloads:

```text
Web GUI / SDK / MCP / IDE
          ↓
Versioned CortexEnv API
          ↓
Durable Queue + Policy + Audit + Quota
          ↓
Fail-closed Linux Supervisor
          ↓
Isolated WASM / JavaScript / Python Worker
          ↓
Result + Logs + Artifacts
```

The first recommended end-user product is a **Private AI Data Analyst** or **Private AI Coding Agent**. These products exercise the engine, agent, workspace, artifact, audit, and private-deployment capabilities without requiring CortexEnv to compete immediately as a general-purpose public cloud sandbox.
