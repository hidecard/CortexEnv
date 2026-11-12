# CortexEnv v0.1 Private Analyst

## 1. Product scope and release policy

CortexEnv v0.1 is a **private, single-node execution service for trusted internal analysts, developers, and operators**. Its primary job is to accept an approved data-analysis request, run Python or WebAssembly with explicit resource limits, and return bounded results and artifacts through a versioned API.

This release is for a private enterprise deployment behind an authenticated gateway. It is not a public code-hosting service, an unrestricted malware laboratory, a live-trading system, a device-control platform, or a general-purpose multi-tenant cloud.

The service must run with `CORTEXENV_SECURITY_MODE=trusted` only until the dedicated Linux child supervisor is complete. `restricted`, `production`, and `hostile-code` modes must fail closed rather than silently using the current in-process adapters. The current implementation must not be advertised as a hostile-code security boundary.

## 2. Personas

| Persona | Needs | Allowed responsibility |
| --- | --- | --- |
| Internal analyst | Upload approved data, ask analysis questions, inspect tables/charts, download reports | Owns analysis jobs and approved datasets |
| Developer | Integrate the API, reproduce a job, inspect bounded logs, test runtime behavior | Creates service integrations and test jobs |
| Operator | Configure deployment, quotas, retention, monitoring, and incident response | Manages the private node; does not read user data unless policy permits |

Authentication, authorization, and audit controls must distinguish these responsibilities before a pilot. A static API key is a development convenience and represents one shared principal; it is not a replacement for user identity or organization-level authorization.

## 3. Top five MVP workflows

1. **Dataset upload:** an analyst uploads a CSV, JSON, or Parquet file into a private workspace. The service validates file type, size, ownership, and retention metadata before making it available to a job.
2. **Natural-language analysis:** an analyst submits a question against an approved workspace. The agent produces a bounded Python analysis request with network denied by default and records model/runtime/policy metadata.
3. **Python analysis execution:** the service validates the request, admits it under per-principal quotas, runs it through the child supervisor, captures bounded stdout/stderr, and returns a structured result.
4. **Chart/table/report generation:** a job writes approved output artifacts such as tables, PNG charts, or HTML/PDF reports into its workspace. The service returns an artifact manifest with checksums and expiry metadata.
5. **History and export:** the analyst reviews job status and bounded output, downloads an owned artifact, or deletes the workspace and its retained data according to policy.

## 4. Supported and unsupported workloads

### Supported in the v0.1 product definition

- Python and WebAssembly runtimes only.
- CSV, JSON, and Parquet input.
- PNG charts, HTML reports, and PDF reports as controlled output artifacts.
- Analysis and script execution modes; build and test modes are not part of the first user-facing release.
- Approved data processing with pandas, Polars, NumPy, and Matplotlib profiles where those dependencies are pre-installed and explicitly allowlisted.
- Default-deny network access and explicit resource ceilings.
- Private single-node deployment with authenticated API access.

### Explicitly unsupported

- Unrestricted internet access, proxy bypass, DNS tunneling, or active network testing.
- Arbitrary package installation, compiler toolchains, or downloading executable code.
- Host filesystem, Docker socket, cloud metadata, device, GPU, or hardware access.
- Live trading, financial account actions, credential use, or external side effects.
- Public hostile-code execution, malware analysis, exploit development, or multi-tenant isolation claims before the supervisor exit gate passes.
- Long-lived secrets in source, environment, logs, artifacts, prompts, or model output.

## 5. Threat model

### Assets

- User datasets, uploaded files, generated artifacts, and job history.
- API credentials, model-provider credentials, deployment secrets, and host metadata.
- Availability of the API and the integrity of execution results.
- Principal ownership boundaries and audit records.

### Adversaries

- Malicious or accidental user-submitted Python/WASM code.
- A malicious input file containing prompt-injection instructions or parser abuse.
- A model-generated instruction attempting to escape the workspace, access secrets, or call an unapproved tool.
- An authenticated user attempting to read another principal's job, artifact, or logs.
- A compromised dependency or malformed runtime payload.
- An operator mistake that enables a production security mode without the required controls.

### Security objectives

1. Validate requests and resource limits before execution.
2. Keep each job's files and results within its owned workspace and artifact boundary.
3. Deny network and secret access by default and fail closed when enforcement is unavailable.
4. Bound CPU, memory, process count, file descriptors, disk, output, and execution time.
5. Prevent cross-principal job/result/artifact access.
6. Terminate the complete child process tree and clean up resources on completion, timeout, cancellation, or crash.
7. Preserve an auditable record of authentication, execution, policy, artifact, and deletion events without leaking secrets.
8. Avoid claiming stronger isolation than the deployed controls and evidence support.

### Known residual risks before the Phase 2 exit gate

The current adapters are not a complete hostile-code boundary. Python has an optional bubblewrap path for namespaces, read-only root/system mounts, bounded private tmpfs, and capability dropping, but it is not enabled by default, does not expose approved host input/output mounts, and does not install seccomp. UID/GID mapping evidence, runtime-specific seccomp-BPF profiles, reliable cgroup attachment for every runtime, complete network/filesystem policy, JavaScript/WASM supervision, and parent-side lifecycle guarantees remain incomplete. The service therefore remains suitable only for trusted internal development/private pilot use until the required controls and adversarial evidence are complete.

## 6. Security modes

| Mode | Intended use | Required behavior |
| --- | --- | --- |
| `trusted` | Local development and approved internal testing | May use current adapters; never advertise hostile-code isolation; must still validate limits and deny unsupported policies |
| `restricted` | Future private pilot | Must require the dedicated child supervisor, namespaces, cgroups, seccomp, filesystem/network policy, and evidence checks; fail closed if any control is absent |
| `production` | Future controlled deployment | Same controls as `restricted`, plus authenticated identity, durable jobs/artifacts, audit, backups, monitoring, and release verification |
| `hostile-code` | Future security-tested execution | Requires a separate security review and complete adversarial evidence; unavailable in v0.1 |

## 7. Data classification, retention, and deletion

- **Public:** source code and documentation intentionally published in the repository. No credentials or user data.
- **Internal:** service configuration, operational metrics, and non-sensitive audit metadata. Retain for 30 days by default.
- **Confidential:** uploaded datasets, source code, stdout/stderr, generated artifacts, prompts, and model responses. Retain for 7 days by default, configurable downward by the operator and never extended by a client request without policy approval.
- **Restricted:** credentials, signing keys, tokens, raw security findings, and host metadata. Never place these in job workspaces or artifacts; store only in an approved secret manager and redact from logs.

Deletion must remove the job record, workspace, artifacts, and derived previews after the retention deadline or an authorized deletion request. Durable storage and deletion workers are Phase 5 work; until then, in-memory job state is lost on restart and must not be presented as durable retention.

## 8. Security decision record template

Every change that affects execution, isolation, data handling, or release claims should record:

- **Decision ID and date:**
- **Owner and reviewers:**
- **Change summary:**
- **Threat/model impact:**
- **Assets and principals affected:**
- **Controls added or removed:**
- **Fail-closed behavior:**
- **Evidence:** tests, logs, CI run, VM image, and reproduction command.
- **Residual risk and known limitations:**
- **Rollback plan:**
- **Release decision:** accepted for trusted development / private pilot / blocked.

## 9. MVP acceptance criteria

The v0.1 private analyst release is acceptable only when all of the following are true:

- An authenticated test user can upload an approved dataset, run an analysis, view bounded output, and download an owned artifact.
- Unsupported language, mode, file type, network policy, resource limit, and security mode requests are rejected with stable machine-readable errors.
- User A cannot read User B's jobs, workspaces, artifacts, logs, or status streams.
- A timeout, cancellation, child crash, or supervisor failure cannot crash the API process and leaves no orphaned execution resource.
- The deployment fails closed when a required isolation control is missing; no restricted/production/hostile-code request reaches the current adapters.
- Python and WASM runtime behavior, resource ceilings, startup/shutdown, and version pins are documented and tested.
- Job, artifact, audit, retention, backup/restore, and deletion behavior is durable and verified before a production-like pilot.
- CI, dependency audit, image scan, security regression evidence, monitoring, incident runbook, and an independent security review are complete before any external beta or hostile-code claim.

This document is a product and security boundary, not proof that the current repository already satisfies every criterion. The checkboxes in `TODO.md` track implementation evidence separately.
