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


## Phase 12: Universal Execution Contract

The platform must support both single-file code and full project/workspace execution. Keep one versioned contract for local and remote execution.

- [ ] Add `runtime`, `version`, `mode`, `template`, `entrypoint`, `workspace`, `network`, `resources`, `artifacts`, and `target` fields to the execution model
- [ ] Support `target: local` and `target: remote` with identical request/result semantics
- [ ] Support script, build, test, preview, simulation, and service execution modes
- [ ] Add structured artifact results for files, charts, images, reports, binaries, models, and logs
- [ ] Add build-step and run-step separation
- [ ] Add runtime adapter registry instead of language-specific dispatch branches
- [ ] Add runtime capability discovery and version negotiation
- [ ] Add environment-template manifest format
- [ ] Add template version pinning and compatibility checks
- [ ] Add workspace file manifest, checksums, upload/download, and cleanup policy
- [ ] Add deterministic execution metadata: runtime version, template version, policy, and resource limits

## Phase 13: Language and Runtime Adapter Coverage

### Portable runtimes and scripting

- [ ] WASM/WASI and Rust-to-WASM execution profile
- [ ] JavaScript execution profile
- [ ] TypeScript transpile/run profile
- [ ] Python execution profile
- [ ] Shell/Bash execution profile
- [ ] Lua execution profile
- [ ] Ruby execution profile
- [ ] PHP execution profile
- [ ] R execution profile
- [ ] Julia execution profile

### Compiled languages

- [ ] C build/test/run profile
- [ ] C++ build/test/run profile
- [ ] Rust build/test/run profile
- [ ] Go build/test/run profile
- [ ] Zig build/test/run profile
- [ ] Java/Kotlin build/test/run profile
- [ ] C#/.NET build/test/run profile
- [ ] Swift build/test/run profile
- [ ] Fortran build/test/run profile
- [ ] Compiler toolchain version selection
- [ ] Build artifact signing/checksum and restricted run policy
- [ ] Separate build environment from production run environment

### Data and query languages

- [ ] SQL execution against an explicitly selected database or fixture
- [ ] PostgreSQL/SQLite/MySQL query profiles
- [ ] R statistics profile
- [ ] Julia scientific-computing profile
- [ ] Notebook execution profile
- [ ] Query timeout, row limit, result-size limit, and read/write policy
- [ ] Database credential isolation and secret-redaction tests

### Runtime quality

- [ ] Per-runtime startup command and signal policy
- [ ] Per-runtime environment-variable policy
- [ ] Per-runtime filesystem and network policy
- [ ] Per-runtime resource profile
- [ ] Per-runtime seccomp profile
- [ ] Runtime health checks and compatibility tests
- [ ] Runtime CVE and dependency update process

## Phase 14: Web and Application Framework Templates

### Frontend and web

- [ ] HTML/CSS/JavaScript live-preview workspace
- [ ] Static HTML/CSS preview server with isolated origin
- [ ] React + Vite + TypeScript template
- [ ] Vue + Vite template
- [ ] Svelte template
- [ ] Angular template
- [ ] Next.js template
- [ ] Nuxt template
- [ ] Build logs, preview URL, port policy, and artifact export
- [ ] npm/pnpm/yarn lockfile detection
- [ ] Offline package-cache support for frontend templates

### Backend frameworks

- [ ] FastAPI template
- [ ] Django template
- [ ] Flask template
- [ ] Spring Boot template
- [ ] Quarkus template
- [ ] ASP.NET template
- [ ] Laravel template
- [ ] Rails template
- [ ] Phoenix template
- [ ] Axum template
- [ ] Actix template
- [ ] Go Gin/Fiber template
- [ ] Framework health check and graceful shutdown
- [ ] Preview proxy with private-by-default ports

## Phase 15: AI and Data-Science Environments

- [ ] Python data-science template
- [ ] Python machine-learning CPU template
- [ ] Optional GPU environment with explicit scheduling and quota
- [ ] R statistics template
- [ ] Julia science template
- [ ] SQL analytics template
- [ ] NumPy, pandas, Polars, SciPy, and scikit-learn profiles
- [ ] PyTorch, TensorFlow, JAX, and Hugging Face profiles
- [ ] OpenCV and image-processing profile
- [ ] GeoPandas, Rasterio, GDAL, and geospatial profile
- [ ] Matplotlib, Plotly, and chart artifact support
- [ ] CSV, JSON, Excel, Parquet, PDF, image, and report artifacts
- [ ] Notebook import/export and reproducible execution metadata
- [ ] Local LLM integration for offline agent mode
- [ ] Remote OpenAI-compatible provider integration for online agent mode
- [ ] Embeddings/vector-store policy for local and remote deployments
- [ ] Model, token, execution-step, and GPU usage accounting

## Phase 16: Cybersecurity and Networking Workloads

### Cybersecurity profiles

- [ ] Static code-analysis profile
- [ ] Dependency and vulnerability-scanning profile
- [ ] YARA rule testing profile
- [ ] Sigma rule testing profile
- [ ] Suricata-rule testing profile
- [ ] PCAP analysis profile
- [ ] Log-analysis profile
- [ ] CTF and disposable-lab profile
- [ ] Malware-analysis profile restricted to a disposable isolated lab
- [ ] Explicit authorization and target allowlist for active testing
- [ ] Human approval for active network operations
- [ ] Security-tool output and evidence artifact collection
- [ ] Cybersecurity audit trail and lab expiration policy

### Networking profiles

- [ ] TCP/UDP server test profile
- [ ] HTTP/API and gRPC test profile
- [ ] DNS simulation profile
- [ ] MQTT and IoT-network test profile
- [ ] Packet parser and protocol-analysis profile
- [ ] Network-topology simulation profile
- [ ] Private-lab network namespace/profile
- [ ] Network policy modes: deny, loopback, private-lab, allowlist, isolated-bridge
- [ ] Hostname/IP/CIDR/port/protocol/bytes/time allowlist controls
- [ ] DNS, IPv4, IPv6, proxy-bypass, and egress-audit tests
- [ ] Never expose unrestricted public network access by default

## Phase 17: IoT, Robotics, Agriculture, and Finance Profiles

### IoT

- [ ] Device and sensor-data simulator
- [ ] MQTT broker integration through a controlled gateway
- [ ] CoAP profile
- [ ] Modbus simulator/profile
- [ ] Arduino and PlatformIO build profile
- [ ] ESP32 and MicroPython profile
- [ ] STM32/Zephyr/FreeRTOS build profile
- [ ] Raspberry Pi development profile without unrestricted host access
- [ ] Serial, BLE, GPIO, and USB access through an audited device gateway only
- [ ] Device ID, port, command, time, and approval policy

### Robotics

- [ ] ROS 2 workspace template
- [ ] ROS bag analysis and replay
- [ ] Gazebo simulation profile
- [ ] Webots simulation profile
- [ ] Isaac simulation profile where supported
- [ ] OpenCV, SLAM, and sensor-processing profiles
- [ ] Simulation-only default mode
- [ ] ROS topic/service permission policy
- [ ] Staging-robot approval workflow
- [ ] Direct motor/hardware control disabled by default

### Agriculture

- [ ] Soil and weather sensor analysis profile
- [ ] Crop-disease image analysis profile
- [ ] Irrigation recommendation workflow
- [ ] Yield prediction workflow
- [ ] Satellite and geospatial-data workflow
- [ ] Greenhouse automation simulation
- [ ] Farm dashboard and time-series artifact support
- [ ] MQTT/device-gateway integration with approval and audit

### Finance

- [ ] Financial-data analysis profile
- [ ] Portfolio simulation and backtesting profile
- [ ] Risk-calculation profile
- [ ] Budget/forecasting profile
- [ ] Fraud-detection model profile
- [ ] Invoice and financial-report workflow
- [ ] Historical/synthetic data mode by default
- [ ] Live account and broker credentials prohibited in ordinary sandboxes
- [ ] Live-trading or order-placement workflow requires a separate approved service and human approval
- [ ] Financial audit log, retention, and reproducibility metadata

## Phase 18: Offline and Online Execution Modes

### Shared architecture

- [ ] Use one API contract for local and remote execution
- [ ] Add `LocalExecutionClient` and `RemoteExecutionClient`
- [ ] Add local/remote target selection in the GUI
- [ ] Keep the same result, job, artifact, and error rendering for both modes
- [ ] Add capability discovery so the GUI knows which templates are available locally or remotely
- [ ] Add explicit offline/online connection status

### Offline/local mode

- [ ] Package or install `cortexenv-server` for local use
- [ ] Add Tauri-managed local engine lifecycle: start, health check, stop, and restart
- [ ] Add local filesystem/workspace policy
- [ ] Add local runtime and environment-template bundle
- [ ] Add offline Python package cache or local package registry
- [ ] Add offline npm/package cache or local registry
- [ ] Add local model server support through an OpenAI-compatible endpoint
- [ ] Add local history, artifacts, and quota storage
- [ ] Add offline-first error and retry behavior
- [ ] Ensure local execution still enforces supervisor, filesystem, network, and resource isolation

### Online/remote mode

- [ ] Add HTTPS remote API endpoint configuration
- [ ] Add OIDC/JWT login and organization selection
- [ ] Add remote queue, worker scheduling, and autoscaling
- [ ] Add PostgreSQL, Redis, and object-storage deployment profile
- [ ] Add remote artifacts and durable execution history
- [ ] Add team sharing, quotas, audit, and policy management
- [ ] Add WebSocket reconnect and polling fallback
- [ ] Add online model-provider configuration
- [ ] Add remote execution cost and usage reporting

### Synchronization and fallback

- [ ] Define what data can remain local and what can sync remotely
- [ ] Add opt-in workspace/history synchronization
- [ ] Add conflict handling for local and remote session state
- [ ] Add local fallback for code execution when the remote service is unavailable
- [ ] Clearly disable or explain remote-only features while offline
- [ ] Never upload files or source code without explicit user policy/consent

## Phase 19: GUI, Desktop, and Developer Experience

### Web GUI

- [ ] React + TypeScript + Vite web application
- [ ] React Router and route-level authorization
- [ ] TanStack Query for API/server state
- [ ] Zustand or equivalent for local UI state
- [ ] Tailwind CSS and accessible component system
- [ ] Monaco Editor for IDE-like workspaces or CodeMirror for lightweight runners
- [ ] xterm.js or sanitized console renderer for live logs
- [ ] Workspace-type selector instead of a language-only selector
- [ ] Code Runner workspace
- [ ] Web App workspace
- [ ] Data Science workspace
- [ ] AI/ML workspace
- [ ] Cyber Lab workspace
- [ ] Network Lab workspace
- [ ] IoT Simulator workspace
- [ ] Robot Simulator workspace
- [ ] Agriculture Analysis workspace
- [ ] Finance Research workspace
- [ ] Custom Environment workspace
- [ ] Preview, build, run, test, simulate, and artifact actions
- [ ] Burmese/English localization and accessibility support

### Desktop/offline GUI

- [ ] Tauri 2 desktop application using the same React frontend
- [ ] Local CortexEnv server lifecycle management
- [ ] Local file picker and artifact export
- [ ] Local model configuration
- [ ] System notifications and tray behavior where useful
- [ ] Windows, macOS, and Linux packaging
- [ ] Tauri capability/permission audit
- [ ] Never execute user code in the Tauri process; route it through the sandbox supervisor

### Developer tools

- [ ] VS Code integration
- [ ] Jupyter integration
- [ ] CLI workspace/template commands
- [ ] Project import/export
- [ ] Git repository import with explicit permissions
- [ ] Build/test/preview logs and downloadable artifacts
- [ ] Webhook/MCP/IDE execution integration

## Phase 20: Optional Service Layer and Integration Boundaries

- [ ] Keep Rust Axum as the main control-plane and execution API
- [ ] Add FastAPI only as an optional Python data/ML/document service
- [ ] Define one shared OpenAPI/JSON Schema contract between services
- [ ] Add pandas/NumPy/report-generation service where the product needs it
- [ ] Keep arbitrary code execution inside CortexEnv isolation, never inside the FastAPI service process
- [ ] Add local LLM service integration through an OpenAI-compatible endpoint
- [ ] Add device gateway for IoT/robot hardware instead of direct sandbox hardware access
- [ ] Add simulation gateway for ROS/network/device simulations
- [ ] Add service-to-service authentication, mTLS where required, and audit correlation

## Final Universal Platform Readiness Gate

- [ ] At least one scripting runtime, one compiled runtime, one web template, one data template, and one simulation profile pass end-to-end tests
- [ ] Every runtime/template declares its version, resources, network policy, filesystem policy, and artifact policy
- [ ] Local and remote execution produce compatible results and artifacts
- [ ] Offline mode works without external network access for the documented local templates
- [ ] Online mode supports authentication, durable jobs, quotas, artifacts, and audit logs
- [ ] Domain profiles default to safe simulation or read-only modes
- [ ] Hardware, live trading, active network testing, and destructive operations require explicit policy and approval
- [ ] GUI can create a workspace, select a template, edit/upload files, build/run/simulate, view live output, and download artifacts
- [ ] SDK, CLI, GUI, API, and runtime-template versions have a compatibility policy
- [ ] Security, privacy, data-retention, and acceptable-use limitations are visible to end users
- [ ] Reference products pass trusted pilots in at least one offline and one online deployment
