# CortexEnv — MVP-Focused Production TODO Roadmap

This is the authoritative implementation roadmap for **CortexEnv v0.1 Private Analyst**. The MVP target is a trusted internal/private-enterprise single-node deployment with Python and WASM runtimes, default-deny networking, durable jobs/artifacts, and a per-job child supervisor. It is not an unrestricted public sandbox, live-trading system, hardware-control platform, malware laboratory, multi-cloud autoscaler, or broad multi-runtime platform.

## Current repository progress

The repository already contains a Rust workspace, shared execution contracts with fail-closed rejection of unsupported v1 request metadata, WASM and JavaScript adapters, a Python child adapter, cgroup v2 foundations, bounded API admission/history, request IDs, authentication, per-principal quotas, async job ownership, Prometheus metrics, OpenAI-compatible model access, bounded ReAct execution, Docker health checks, and a fail-closed child-hardening foundation for Linux namespaces, `RLIMIT_NOFILE`, `PR_SET_NO_NEW_PRIVS`, and capability clearing.

The child-hardening foundation is not yet wired into a dedicated runtime supervisor. The placeholder dependency has been replaced with upstream `seccompiler` 0.5, and the child-only API can compile/install a baseline WASM denylist filter. It defaults to allowing other syscalls, is not a complete sandbox, and is not applied by current runtime launch paths. Restricted/production/hostile-code execution therefore remains fail-closed until runtime-specific profiles, UID/GID mapping, mount setup, cgroup attachment, and parent-side lifecycle supervision are implemented and tested in a disposable Linux environment. The Python adapter's optional `CORTEXENV_CGROUP_ROOT` path uses the prepared admission lifecycle: cgroup/workspace preparation, stopped-child attach, source-release permit, and cgroup-wide cleanup. Python also has a separate, disabled-by-default bubblewrap mode for partial namespace and mount isolation; it is not a production supervisor. WASM/JavaScript isolation, complete filesystem policy, and runtime-profile integration remain pending.

The runtime now exposes `CORTEXENV_SECURITY_MODE`: `trusted` is development-only, while `restricted`/`production`/`hostile-code` fail closed during runtime startup and before any current adapter starts. This prevents an operator from accidentally treating the in-process adapters as a production sandbox, but it does not replace the dedicated child supervisor. Linux also has a fail-closed pre-spawn admission coordinator that validates the request, reserves a private workspace, and prepares the cgroup before source release.

Phase 1 test scripts are available under [`scripts/security`](scripts/security/): they run safe unit checks, a forked-child kernel seccomp primitive probe, an opt-in namespace/user-mapping probe, and a read-only cgroup v2 delegated-root preflight. These scripts provide prerequisite evidence only; they do not mark the production supervisor or runtime-specific seccomp profile complete.

> ရည်ရွယ်ချက် — Private AI Data Analyst အတွက် လုံခြုံပြီး self-hosted execution platform MVP တစ်ခုကို အရင်ဆုံး production pilot အထိ ပြီးမြောက်စေရန်။
> ပထမဆုံး deployment: **trusted internal / private enterprise single-node**
> ပထမဆုံး runtime: **Python + WASM**
> Network: **deny by default**
> မလုပ်သေးသည့်အရာများ: unrestricted public sandbox, live trading, hardware control, malware lab, multi-cloud autoscaling, runtime/framework အများကြီး။

## Phase 0 — Product Scope, Threat Model, and Release Policy

### Goal ဘာကို လုံခြုံစွာ support လုပ်မလဲ၊ ဘာကို မလုပ်နိုင်သေးဘူးလဲဆိုတာ တိတိကျကျ သတ်မှတ်ရန်။
- [x] ပထမဆုံး product ကို **Private AI Data Analyst** အဖြစ် အတည်ပြုရန်
- [x] ပထမဆုံး user persona သတ်မှတ်ရန် — internal analyst, developer, operator
- [x] Top 5 user workflows ရေးရန်
- [x] Dataset upload
- [x] Natural-language analysis request
- [x] Python analysis execution
- [x] Chart/table/report generation
- [x] Artifact download and history review
- [x] Deployment mode ကို single-node private deployment အဖြစ် သတ်မှတ်ရန်
- [x] Supported runtime ကို Python + WASM အဖြစ် သတ်မှတ်ရန်
- [x] Supported file types သတ်မှတ်ရန် — CSV, JSON, Parquet, PNG, HTML/PDF report
- [x] Unsupported workload များ သတ်မှတ်ရန်
- [x] Unrestricted internet access
- [x] Arbitrary package installation
- [x] Host filesystem access
- [x] Hardware/device access
- [x] Live trading or financial account access
- [x] Active network testing
- [x] Threat model ရေးရန် — malicious code, malicious file, prompt injection, secret leakage, tenant/job crossover
- [x] Security modes သတ်မှတ်ရန် — `trusted`, `restricted`, `hostile-code`
- [x] MVP အတွက် security claim နှင့် limitation များ ရေးရန်
- [x] Data classification, retention, deletion policy သတ်မှတ်ရန်
- [x] Security decision record template ထည့်ရန်
- [x] MVP acceptance criteria ရေးရန်

### Exit gate

- Product scope တစ်မျိုးတည်း အတည်ပြုပြီး
- Threat model နှင့် limitation document ရှိပြီး
- Supported/unsupported workload များ review ပြီး

## Phase 1 — Universal Execution Contract and API Foundation

### Goal Local/remote နောက်ပိုင်း မပြောင်းလဲဘဲ သုံးနိုင်မည့် versioned execution contract တည်ဆောက်ရန်။
- [x] `ExecutionRequest` schema ကို version လုပ်ရန်
- [x] `ExecutionResult` schema ကို version လုပ်ရန်
- [x] `runtime`, `runtime_version`, `mode`, `entrypoint`, `workspace`, `network`, `resources`, `artifacts`, `target` fields ထည့်ရန်
- [x] `target: local | remote` ကို contract ထဲထည့်ပြီး implementation ကို နောက်ပိုင်းခွဲရန်
- [x] `mode: analyze | script | build | test` အနက် MVP အတွက် `analyze` နှင့် `script` သတ်မှတ်ရန်
- [x] Explicit job states သတ်မှတ်ရန်
- [x] queued
- [x] starting
- [x] running
- [x] completed
- [x] failed
- [x] timed_out
- [x] cancelled
- [x] expired
- [x] Stable machine-readable error codes သတ်မှတ်ရန်
- [x] Timeout, CPU, memory, process, disk, output-limit semantics ရေးရန်
- [x] Artifact manifest နှင့် checksum format သတ်မှတ်ရန်
- [x] OpenAPI v1 specification ရေးပြီး freeze လုပ်ရန်
- [x] Compatibility and deprecation policy ရေးရန်
- [x] API contract tests ထည့်ရန်
- [x] Runtime pins, entrypoints, workspace bindings, artifact requests, and remote targets ကို unsupported ဖြစ်လျှင် source execution/queue မလုပ်မီ explicit reject လုပ်ရန်; legacy-shaped requests တွင် v1-only fields ပါလာပါက `schema_version` မရှိဘဲ လက်မခံရန်

### Exit gate

- Schema, error codes, job states, OpenAPI v1 freeze ပြီး
- Client/server contract test အားလုံး pass ဖြစ်ပြီး

## Phase 2 — Secure Child Supervisor and Linux Isolation

### Goal Untrusted code ကို API process ထဲတွင် မလုပ်ဘဲ fail-closed child supervisor မှတစ်ဆင့် အထီးကျန် run စေရန်။
> `restricted`/`production` mode တွင် server health/metrics စတင်နိုင်သော်လည်း required isolation preflight မအောင်မြင်ပါက execution request အားလုံးကို fail closed လုပ်ရမည်။ `hostile-code` claim သည် independent security review မပြီးမချင်း မရရှိသေးပါ။
- [ ] Per-execution child supervisor တည်ဆောက်ရန်
- [x] Request validation ကို child spawn မလုပ်မီ အပြီးလုပ်ရန်
- [x] Private per-execution workspace ဖန်တီးရန်
- [x] Source release မလုပ်မီ cgroup v2 ဖန်တီးရန်
- [x] Child PID attach မပြီးမချင်း source-release permit မထုတ်ရန် (lifecycle contract)
- [x] Python adapter optional cgroup path ကို prepared admission lifecycle နှင့် ချိတ်ရန်
- [x] Python-only opt-in bubblewrap launcher with user/mount/PID/network/IPC/UTS namespaces, capability drop, read-only root, and bounded private tmpfs (not the general child supervisor)
- [x] Conditional harmless Python namespace smoke test for namespace IDs, filesystem/tmpfs limits, capability state, workspace writes, and host-loopback separation
- The remaining unchecked namespace, capability, filesystem, and network items below mean complete child-supervisor enforcement, verified UID/GID mapping, and support across runtimes/deployments; the Python opt-in does not satisfy those release gates.
- [ ] Child နှင့် descendants များကို cgroup ထဲ attach လုပ်ရန်
- [x] CPU, memory, swap, pids limits ကို cgroup v2 configuration အဖြစ်ရေးရန်; kernel exhaustion evidence မပြီးသေး
- [x] Child-side user namespace နှင့် unprivileged UID/GID mapping primitive ထည့်ရန်; general supervisor launch integration နှင့် VM evidence မပြီးသေး
- [x] Child-side mount namespace ကို private propagation အဖြစ်ပြင်ရန်; complete launch integration မပြီးသေး
- [ ] PID namespace ထည့်ရန်
- [ ] Network namespace ထည့်ရန်
- [x] Supplied runtime root ကို read-only bind-remount primitive ထည့်ရန်; pivot/chroot minimal root selection မပြီးသေး
- [x] Supplied workspace ကို သီးခြား writable bind mount primitive ထည့်ရန်; full host-root hiding မပြီးသေး
- [ ] Host paths, secrets, Docker socket, application data မ inheritance ဖြစ်စေရန်
- [ ] `PR_SET_NO_NEW_PRIVS` သတ်မှတ်ရန်
- [ ] Linux capabilities drop လုပ်ရန်
- [ ] Runtime-specific seccomp profile ထည့်ရန်
- [x] Placeholder `seccompiler` dependency ကို real BPF compiler ဖြင့်အစားထိုးပြီး child-only WASM denylist filter ကို kernel က apply လုပ်ကြောင်း စမ်းသပ်ရန် (complete allowlist/profile မဟုတ်ပါ၊ runtime launch path တွင် မချိတ်ရသေးပါ)
- [ ] Default-deny network policy enforce လုပ်ရန်
- [ ] Filesystem access ကို explicit workspace/input/output mount များထဲတွင် ကန့်သတ်ရန်
- [ ] Process-tree cancellation နှင့် descendant cleanup ထည့်ရန်
- [x] cgroup kill support အသုံးပြုရန်; real supervisor cancellation integration မပြီးသေး
- [ ] Stdout/stderr backpressure ထည့်ရန်
- [ ] Output size cap ထည့်ရန်
- [ ] Required isolation control မရှိပါက fail closed လုပ်ရန်
- [x] Trusted-development mode နှင့် restricted/production mode ခွဲထားရန်

### Exit gate

- [x] Supervisor မပါဘဲ explicit restricted/production/hostile-code mode မ run နိုင်ရန်
- [ ] Isolation control ပျက်ပါက request ကို block လုပ်ရန် — preflight/route integration evidence လိုသေးသည်
- [ ] API process crash မဖြစ်ဘဲ child failure ကို handle လုပ်နိုင်ရန် — real child supervisor လိုသေးသည်

## Phase 3 — Python and WASM Runtime Profiles

### Goal Runtime နှစ်ခုကိုသာ security profile ပြည့်စုံစွာ ထောက်ပံ့ရန်။
> လက်ရှိ WASM timeout/memory နှင့် Python child-process cleanup သည် baseline implementation ဖြစ်ပြီး full security profile မဟုတ်ပါ။ Phase 2 isolation ပြီးမှ runtime profile complete ဟု သတ်မှတ်မည်။
- [x] Python runtime startup command သတ်မှတ်ရန်
- [x] Python process-group cleanup ထည့်ရန်
- [ ] Python environment template သတ်မှတ်ရန်
- [ ] Allowlisted packages ထည့်ရန် — pandas, polars, numpy, matplotlib, scipy လိုအပ်သလောက်
- [x] Package installation ကို default ပိတ်ရန်
- [x] WASM/WASI runtime profile သတ်မှတ်ရန် — core WASM profile; WASI capabilities မပေးသေး
- [x] WASM epoch timeout ထည့်ရန်
- [x] WASM memory limit enforce လုပ်ရန်
- [ ] Per-runtime filesystem/network policy သတ်မှတ်ရန်
- [ ] Per-runtime resource profile သတ်မှတ်ရန်
- [ ] Runtime version pinning ထည့်ရန်
- [x] Runtime capability discovery ထည့်ရန် — `sandbox-runtime::runtime_profiles()` and [`docs/runtime-profiles.md`](docs/runtime-profiles.md)
- [ ] Runtime startup, signal, timeout behavior document လုပ်ရန်
- [ ] Unsupported runtime request ကို clear error ဖြင့် reject လုပ်ရန်

### Exit gate

- Python နှင့် WASM တို့အတွက် behavior matrix ပြီး
- Version pinning နှင့် resource/security profile test pass ဖြစ်ပြီး

## Phase 4 — Security Integration and Adversarial Testing

### Goal Security claim တစ်ခုချင်းစီအတွက် automated evidence ရှိစေရန်။
> Hostile-code tests များကို disposable, unprivileged Linux VM သို့မဟုတ် isolated CI runner တွင်သာ run ရန်။
- [x] Conditional benign Python bubblewrap namespace visibility smoke test ထည့်ရန် (deployment-dependent; not an escape test)
- [ ] Unprivileged identity test ထည့်ရန်
- [ ] Absolute path escape test ထည့်ရန်
- [ ] Path traversal test ထည့်ရန်
- [ ] Symlink escape test ထည့်ရန်
- [ ] Mount boundary test ထည့်ရန်
- [ ] Workspace ပြင်ပ read/write test ထည့်ရန်
- [ ] Host secret visibility test ထည့်ရန်
- [ ] Docker socket visibility test ထည့်ရန်
- [ ] Network deny test — TCP, UDP, DNS, IPv4, IPv6
- [ ] Proxy bypass test ထည့်ရန်
- [ ] CPU exhaustion test ထည့်ရန်
- [ ] Memory/swap exhaustion test ထည့်ရန်
- [ ] Fork bomb/PID exhaustion test ထည့်ရန်
- [ ] File descriptor exhaustion test ထည့်ရန်
- [ ] Disk/temp-file quota test ထည့်ရန်
- [ ] Stdout/stderr flood test ထည့်ရန်
- [ ] Compile/startup/execution/shutdown timeout test ထည့်ရန်
- [x] Child-only seccomp-BPF primitive test ထည့်ရန် — filter တပ်ထားသော thread တွင် `getpid` ကို deny လုပ်ကြောင်း စစ်ဆေးရန်
- [ ] Runtime-specific seccomp deny test ကို dedicated supervisor နှင့် disposable Linux deployment တွင် စမ်းရန်
- [ ] Capability and privilege-escalation test ထည့်ရန်
- [ ] Child crash/kill နောက် API process survival test ထည့်ရန်
- [ ] Concurrent execution isolation test ထည့်ရန်
- [ ] Cross-principal job/result access test ထည့်ရန်
- [ ] Request parser နှင့် runtime adapter fuzz test ထည့်ရန်
- [ ] Dependency audit, SBOM, image scan, secret scan ထည့်ရန်
- [ ] Security claim → test → evidence matrix ပြုလုပ်ရန်
- [x] Add guarded namespace and kernel-seccomp smoke scripts that never submit code to the API.
- [x] Add a read-only cgroup v2 controller/delegated-root preflight to the Phase 1 smoke runner.
- [ ] Run the smoke scripts in the supported disposable Linux VM/CI environment and attach versioned evidence.
- [x] Phase 2 control-by-control evidence matrix ကို child supervisor controls နှင့် ချိတ်ရန်
- [ ] Independent security review ပြုလုပ်ရန်
- [ ] Known limitations နှင့် disclosure policy ထုတ်ပြန်ရန်

### Exit gate

- Critical isolation test များ အားလုံး pass ဖြစ်ပြီး
- Known critical/high findings မကျန်ရှိဘဲ
- Security reviewer sign-off ရရှိပြီး

## Phase 5 — Reliable Job API and Durable Storage

### Goal Restart ပြီးနောက် jobs, artifacts, audit information မပျောက်စေရန်။
- [ ] Queue နှင့် execution worker ကို ခွဲရန်
- [ ] Durable job records ထည့်ရန်
- [ ] Explicit state transition validation ထည့်ရန်
- [ ] Job cancellation API ထည့်ရန်
- [ ] Supervisor cancellation path ချိတ်ရန်
- [ ] Idempotency key ထည့်ရန်
- [ ] Safe retry behavior သတ်မှတ်ရန်
- [ ] Queue backpressure ထည့်ရန်
- [ ] Orphan-job recovery ထည့်ရန်
- [ ] Worker heartbeat ထည့်ရန်
- [ ] Graceful draining ထည့်ရန်
- [ ] PostgreSQL သို့မဟုတ် equivalent durable database ထည့်ရန်
- [ ] Object storage ထည့်ရန်
- [ ] Artifact metadata, checksum, expiry ထည့်ရန်
- [ ] Execution history pagination ထည့်ရန်
- [ ] Retention and deletion worker ထည့်ရန်
- [ ] Backup and restore procedure ရေးရန်
- [ ] Restore drill တကယ် run ရန်
- [ ] Database migration rollback test ထည့်ရန်

### Exit gate

- Service restart နောက် job/result/artifact မပျောက်ရန်
- Cancel, retry, timeout, recovery flow အားလုံး test pass ဖြစ်ရန်
- Backup restore test အောင်မြင်ရန်

## Phase 6 — Authentication, Authorization, Audit, and Quota

### Goal User/job/organization data များကို မှန်ကန်စွာ ခွဲခြားထိန်းချုပ်ရန်။
- [ ] User, organization, project, service account model သတ်မှတ်ရန်
- [ ] API key authentication ထည့်ရန်
- [ ] JWT/OIDC ကို pilot မတိုင်မီ ထည့်ရန်
- [ ] User, developer, operator, administrator roles သတ်မှတ်ရန်
- [ ] Role-based authorization ထည့်ရန်
- [ ] Per-user/project/organization quota ထည့်ရန်
- [ ] Distributed rate limit လိုအပ်ချက် သတ်မှတ်ရန်
- [ ] Job, workspace, file, artifact ownership checks ထည့်ရန်
- [ ] Audit event schema သတ်မှတ်ရန်
- [ ] Login, execution, download, policy change, deletion event များ audit လုပ်ရန်
- [ ] Audit log append-only policy သတ်မှတ်ရန်
- [ ] Logs, artifacts, stdout/stderr ထဲ secret redaction ထည့်ရန်
- [ ] Tenant-isolation tests ထည့်ရန်
- [ ] Data retention and deletion enforcement ထည့်ရန်

### Exit gate

- User A က User B ၏ job/file/artifact/log မမြင်နိုင်ရန်
- Audit event များ ပြန်စစ်နိုင်ရန်
- Quota နှင့် authorization test pass ဖြစ်ရန်

## Phase 7 — Observability and Operations

### Goal Production pilot ကို စောင့်ကြည့်၊ ပြန်လည်ပြုပြင်၊ incident ဖြေရှင်းနိုင်ရန်။
- [ ] Structured JSON logs ထည့်ရန်
- [ ] Request, execution, job, tenant, trace correlation ID ထည့်ရန်
- [ ] OpenTelemetry traces ထည့်ရန်
- [ ] Metrics ထည့်ရန်
- [ ] Queue latency
- [ ] Cold start
- [ ] Runtime duration
- [ ] Timeout count
- [ ] Isolation failure
- [ ] Resource kill
- [ ] Artifact upload/download
- [ ] Error rate
- [ ] Metrics endpoint ကို private/authenticated လုပ်ရန်
- [ ] Dashboards ထည့်ရန်
- [ ] Alert rules ထည့်ရန်
- [x] Startup configuration validation ထည့်ရန် — security mode is rejected before the API runtime starts when the supervisor is unavailable.
- [ ] Safe defaults သတ်မှတ်ရန်
- [x] Step-by-step local/API/Docker/security-smoke usage guide ရေးရန် — [`docs/quickstart.md`](docs/quickstart.md)
- [ ] TLS/reverse proxy deployment guide ရေးရန်
- [ ] Release artifacts pin/sign/scan လုပ်ရန်
- [ ] Incident response runbook ရေးရန်
- [ ] Upgrade/rollback runbook ရေးရန်
- [ ] Capacity planning guide ရေးရန်
- [ ] SLO သတ်မှတ်ရန် — availability, completion, latency, isolation failure

### Exit gate

- Operator တစ်ယောက်က dashboard နှင့် logs ဖြင့် failure ကို ရှာနိုင်ရန်
- Incident, rollback, restore procedure များ လက်တွေ့စမ်းပြီးဖြစ်ရန်

## Phase 8 — Private AI Data Analyst MVP Web Application

### Goal Login မှ result/export အထိ user workflow တစ်ခုလုံး အဆုံးမှအဆုံး ပြီးစီးရန်။
- [ ] Web frontend project ဖန်တီးရန်
- [ ] Versioned CortexEnv API client ထည့်ရန်
- [ ] Login/logout/session management ထည့်ရန်
- [ ] Organization/project selection ထည့်ရန်
- [ ] Workspace creation ထည့်ရန်
- [ ] CSV/JSON/Parquet upload ထည့်ရန်
- [ ] File size/type validation ထည့်ရန်
- [ ] Python analysis workspace ထည့်ရန်
- [ ] Natural-language AI chat interface ထည့်ရန်
- [ ] File attachment နှင့် tool/execution activity ပြရန်
- [ ] Run, stop, cancel, retry, rerun controls ထည့်ရန်
- [ ] Live stdout/stderr console ထည့်ရန်
- [ ] WebSocket reconnect နှင့် polling fallback ထည့်ရန်
- [ ] Table result rendering ထည့်ရန်
- [ ] Chart/image result rendering ထည့်ရန်
- [ ] HTML/PDF report artifact rendering ထည့်ရန်
- [ ] Artifact download ထည့်ရန်
- [ ] Execution history, search, filter, pagination ထည့်ရန်
- [ ] Timeout/resource/isolation error ကို user နားလည်အောင် ပြရန်
- [ ] User/team usage and quota dashboard ထည့်ရန်
- [ ] Data deletion/export control ထည့်ရန်
- [ ] English/Burmese localization ထည့်ရန်
- [ ] Accessibility နှင့် responsive layout စစ်ရန်
- [ ] Frontend unit/integration/E2E tests ထည့်ရန်

### Exit gate

- Test user တစ်ယောက်သည် login → upload → ask → execute → view → download workflow ကို အောင်မြင်စွာ ပြီးစီးနိုင်ရန်

## Phase 9 — AI Agent Guardrails and Data-Analysis Quality

### Goal AI agent က ပြင်ပ data, tools, secrets များကို မထိန်းချုပ်နိုင်ဘဲ ပြန်လည်ပြင်ဆင်နိုင်သော analysis workflow တည်ဆောက်ရန်။
- [ ] Agent step/time/token/execution budgets ထည့်ရန်
- [ ] AI-generated code ကို untrusted input အဖြစ် သတ်မှတ်ရန်
- [ ] Execution output မှ prompt injection boundary ထည့်ရန်
- [ ] Tool allowlist ထည့်ရန်
- [ ] File read/write policy ထည့်ရန်
- [ ] External network request ကို default ပိတ်ရန်
- [ ] Secret access ကို default ပိတ်ရန်
- [ ] Destructive action approval gate ထည့်ရန်
- [ ] Error → repair → rerun → final result flow ထည့်ရန်
- [ ] Partial result behavior သတ်မှတ်ရန်
- [ ] Model/provider config ကို secret မ hard-code ဘဲ ထည့်ရန်
- [ ] AI result reproducibility metadata ထည့်ရန်
- [ ] Prompt, model, runtime, template, policy version မှတ်တမ်းတင်ရန်
- [ ] Data-analysis correctness sample suite ထည့်ရန်
- [ ] Hallucinated result warning ထည့်ရန်
- [ ] User approval မရှိဘဲ result ကို final အဖြစ် မတင်ရန်

### Exit gate

- Common analysis tasks အတွက် success rate သတ်မှတ်ချက် ပြည့်မီရန်
- Prompt injection နှင့် unsafe tool-call tests pass ဖြစ်ရန်

## Phase 10 — Trusted Pilot and Release Gate

### Goal Controlled users နှင့် production-like private environment တွင် pilot လုပ်ရန်။
- [ ] Pilot environment သီးခြားတည်ဆောက်ရန်
- [ ] 5–10 trusted users ရွေးရန်
- [ ] Sample datasets နှင့် real-but-approved datasets ပြင်ဆင်ရန်
- [ ] Onboarding guide ရေးရန်
- [ ] Privacy, retention, acceptable-use notice ထုတ်ပြန်ရန်
- [ ] Support and issue-triage process သတ်မှတ်ရန်
- [ ] Pilot metrics စုရန်
- [ ] Task success rate
- [ ] Completion rate
- [ ] Median/p95 latency
- [ ] Timeout rate
- [ ] Failure recovery rate
- [ ] Cost per successful task
- [ ] User satisfaction
- [ ] Security regression suite ကို pilot release မတိုင်မီ run ရန်
- [ ] Backup/restore ကို pilot data ဖြင့် verify လုပ်ရန်
- [ ] Pilot blockers ပြင်ရန်
- [ ] Operating procedures update လုပ်ရန်
- [ ] External beta မလုပ်မီ security release gate pass ဖြစ်ကြောင်း အတည်ပြုရန်

### End-user readiness gate
- [ ] Security mode နှင့် deployment assumptions ရှင်းလင်းပြီး
- [ ] Required isolation controls မရှိပါက fail closed ဖြစ်ပြီး
- [ ] Security integration tests pass ဖြစ်ပြီး
- [ ] API v1, schema, SDK contract versioned ဖြစ်ပြီး
- [ ] Jobs, files, artifacts, audit events restart ပြီးနောက် survive ဖြစ်ပြီး
- [ ] Backup/restore tested ဖြစ်ပြီး
- [ ] Monitoring, alerts, runbooks ပြီးပြီး
- [ ] GUI တွင် login, upload, execution, output, history, export ပြည့်စုံပြီး
- [ ] Error/reconnect behavior tested ဖြစ်ပြီး
- [ ] Privacy, retention, deletion, acceptable-use policy ထုတ်ပြန်ပြီး
- [ ] Installation/upgrade documentation ပြီးပြီး
- [ ] Reference product trusted pilot အောင်မြင်ပြီး
- [ ] Limitations ကို README/product documentation တွင် ရိုးသားစွာ ဖော်ပြပြီး

# Deferred Backlog — MVP ပြီးမှသာ စဉ်းစားရန်

## Runtime expansion
- [ ] JavaScript/TypeScript
- [ ] Shell/Bash
- [ ] R/Julia
- [ ] C/C++/Rust/Go build profiles
- [ ] Notebook execution
- [ ] Separate build environment and run environment

## Deployment expansion
- [ ] Multi-node worker pool
- [ ] Redis and distributed queue
- [ ] Kubernetes/Helm
- [ ] Autoscaling
- [ ] Terraform examples
- [ ] Multi-region/data residency

## Developer integrations
- [ ] Python SDK
- [ ] TypeScript SDK
- [ ] MCP server
- [ ] VS Code extension
- [ ] Jupyter integration
- [ ] GitHub Actions/GitLab CI
- [ ] n8n/Airflow/Temporal integrations

## Desktop/offline
- [ ] Tauri desktop app
- [ ] Local engine lifecycle
- [ ] Offline package/model cache
- [ ] Windows/macOS/Linux packages
- [ ] Local/remote synchronization

## Advanced domain profiles
- [ ] Web framework templates
- [ ] GPU/ML profiles
- [ ] Cybersecurity lab
- [ ] IoT/device gateway
- [ ] Robotics simulation
- [ ] Agriculture analytics
- [ ] Finance simulation
- [ ] Live hardware or live trading — separate approved product only

# Recommended first release definition **CortexEnv v0.1 Private Analyst**
- Single-node private deployment
- Python + WASM only
- CSV/JSON/Parquet input
- pandas/Polars/NumPy/Matplotlib profile
- Default-deny network
- Per-job child supervisor
- cgroup + namespace + seccomp isolation
- Durable jobs and artifacts
- Login, upload, execute, live output, report download
- Audit log and usage quota
- Trusted pilot only
- No unrestricted public hostile-code claim
