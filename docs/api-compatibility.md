# CortexEnv API v1 compatibility policy

`docs/openapi-v1.yaml` and `sandbox-common::contract` define the frozen v1 wire contract. Clients should send `schema_version: "v1"` and use the versioned request/result fields. The legacy Rust `ExecutionRequest` remains available as an internal compatibility adapter and maps to v1 with `mode: script`, `target: local`, no workspace, and no artifact requests.

## Compatibility rules

- Additive optional response fields may be added in v1.
- Existing field meanings, enum values, error codes, and lifecycle states must not be repurposed.
- New required request fields require a new schema version.
- Unknown request versions fail closed with `invalid_request`.
- Remote targets are represented in the schema but remain rejected until a remote worker exists.
- The current local executor honors runtime, code, resource ceilings, and network policy. It rejects non-null `runtime_version`, `entrypoint`, or `workspace`, and non-empty `artifacts`, rather than silently dropping those fields. The `mode` value is currently descriptive metadata; it does not select a separate analyst-orchestration path.
- Fields that belong to the versioned schema (including `runtime`, `mode`, `workspace`, `artifacts`, and `target`) require `schema_version: "v1"`; legacy-shaped requests containing them are rejected instead of being deserialized with unknown fields ignored.
- Clients must treat unknown response fields as ignorable and unknown enum values as an unsupported response version.
- A deprecated field remains readable for at least one minor release after a replacement is available, then is removed only in a new schema version.
- Error messages are for humans and may change; clients must branch on `ErrorCode`.
- API v1 does not promise durable jobs or artifacts until the Phase 5 storage exit gate passes.

## Release procedure

A contract change requires an updated OpenAPI file, serde round-trip tests, API route tests, a compatibility note, and a `TODO.md` entry. The change must pass Linux and Windows CI before it is merged to `main`. Breaking changes require a new `/v2` route or a new `schema_version`, migration notes, and a rollback plan.
