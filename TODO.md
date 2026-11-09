# AI Sandbox Engine – Progress Tracker

## Phase 1: Core Sandbox Engine
- [x] Workspace + all crates created
- [x] `sandbox-common`: `ExecutionRequest`, `ExecutionResult`, `Language`, `NetworkPolicy`
- [x] `sandbox-common`: error types, default configuration, and bounded resource validation
- [x] `sandbox-core`: resource-limit validation and explicit fail-closed isolation plan
- [ ] `sandbox-core`: privileged Linux namespaces/cgroups/seccomp/capability enforcement (reserved for hardening)
- [x] `sandbox-runtime`: WASM module loading and `_start` invocation with Wasmtime
- [ ] `sandbox-runtime`: JavaScript support (`deno_core` adapter)
- [ ] `sandbox-runtime`: Python support (isolated process)
- [x] `sandbox-cli`: execute a hex-encoded WASM module and print JSON result
- [x] Basic unit tests for core types and resource limits

## Phase 2: Virtual File System & Strong Isolation
- [ ] Per-execution TempDir / temporary workspace with auto-cleanup
- [ ] Path traversal protection + canonicalize
- [ ] chroot / pivot_root implementation
- [ ] Mount namespace + OverlayFS or tmpfs
- [ ] Network policy enforcement (default deny + optional whitelist)
- [ ] User namespace (rootless)
- [ ] Refined seccomp profiles per language
- [ ] Integration tests for isolation (path traversal, network, etc.)

## Phase 3: High-Performance API Server
- [x] Axum server router skeleton (`/health`, `/v1/execute`)
- [ ] Runnable server binary and graceful shutdown
- [ ] POST `/v1/execute/async` + job ID
- [ ] GET `/v1/executions/:id`
- [ ] WebSocket `/v1/stream`
- [ ] Warm worker pool + idle timeout
- [ ] Request validation middleware, request IDs, and rate limiting
- [ ] In-memory result storage (DashMap)

## Phase 4: Autonomous AI Agent & Self-Healing
- [x] Provider-neutral `execute_code` tool trait
- [ ] LLM client abstraction and providers
- [ ] ReAct / self-correction loop (max iterations)
- [ ] Stateful sessions + optional persistent workspace
- [ ] End-to-end self-healing test

## Phase 5: Security Hardening & Production
- [ ] API key authentication
- [ ] JWT support (optional)
- [ ] Per-key rate limit + quota
- [ ] Penetration testing checklist
- [ ] Prometheus metrics and OpenTelemetry traces
- [ ] Multi-stage non-root Dockerfile and `docker-compose.yml`
- [ ] Health checks + graceful shutdown
- [ ] Architecture documentation

## Cross-Cutting
- [x] Observability dependency and tracing hooks from Phase 1
- [ ] Multi-tenancy quota design
- [ ] Rootless support
- [ ] Comprehensive error messages without leaking host information

## Distribution
- [x] Cross-platform `cortexenv` binary name
- [x] Windows release build (`cortexenv-windows-x86_64.exe`)
- [x] Debian packaging metadata with `cargo-deb`
- [x] Tag-based GitHub Release workflow for Windows and Debian artifacts
