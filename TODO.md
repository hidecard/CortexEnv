# AI Sandbox Engine – Progress Tracker

## Phase 1: Core Sandbox Engine (Week 1–2)
- [x] Workspace + all crates created
- [x] sandbox-common: ExecutionRequest / ExecutionResult / Language / NetworkPolicy types
- [x] sandbox-common: Error types + Config
- [x] sandbox-core: Resource limits (CPU, Memory, Processes, FDs)
- [x] sandbox-core: Basic namespaces (PID, Mount, Network, UTS, IPC)
- [x] sandbox-core: Seccomp filter + capability dropping
- [x] sandbox-runtime: WASM support (wasmtime)
- [ ] sandbox-runtime: JavaScript support (deno_core)
- [ ] sandbox-runtime: Python support (isolated process)
- [ ] sandbox-cli: Can execute code and return stdout/stderr/exit_code
- [ ] Basic unit tests for core types and resource limits

## Phase 2: Virtual File System & Strong Isolation (Week 3–4)
- [ ] Per-execution TempDir / temporary workspace with auto-cleanup
- [ ] Path traversal protection + canonicalize
- [ ] chroot / pivot_root implementation
- [ ] Mount namespace + OverlayFS or tmpfs
- [ ] Network policy (default deny + optional whitelist)
- [ ] User namespace (rootless)
- [ ] Refined seccomp profiles per language
- [ ] Integration tests for isolation (path traversal, network, etc.)

## Phase 3: High-Performance API Server (Week 5)
- [ ] Axum server skeleton
- [ ] POST /v1/execute (synchronous)
- [ ] POST /v1/execute/async + job ID
- [ ] GET /v1/executions/:id
- [ ] WebSocket /v1/stream (real-time stdout/stderr)
- [ ] Warm worker pool + idle timeout
- [ ] Request validation middleware (size, timeout, language allowlist)
- [ ] Tracing + Request ID
- [ ] In-memory result storage (DashMap)
- [ ] Basic rate limiting

## Phase 4: Autonomous AI Agent & Self-Healing (Week 6–7)
- [ ] Tool schema definition for `execute_code`
- [ ] LLM client abstraction trait
- [ ] OpenAI / Claude / Ollama providers
- [ ] ReAct / self-correction loop (max iterations)
- [ ] Stateful sessions + optional persistent workspace
- [ ] End-to-end test: "Write Fibonacci and fix until it works"

## Phase 5: Security Hardening + Production (Week 8–10)
- [ ] API Key authentication
- [ ] JWT support (optional)
- [ ] Per-key rate limit + quota
- [ ] Penetration testing checklist executed
  - [ ] Fork bomb
  - [ ] Memory bomb
  - [ ] CPU exhaustion
  - [ ] Path traversal
  - [ ] Syscall escape attempts
  - [ ] Network escape
- [ ] Prometheus metrics
- [ ] OpenTelemetry traces
- [ ] Multi-stage Dockerfile (non-root, read-only rootfs)
- [ ] docker-compose.yml
- [ ] Health checks + graceful shutdown
- [ ] Documentation (README + architecture)

## Cross-Cutting
- [ ] Observability (tracing) from Phase 1
- [ ] Multi-tenancy quotas design
- [ ] Rootless support
- [ ] Comprehensive error messages without leaking host info