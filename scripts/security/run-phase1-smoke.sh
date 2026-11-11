#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$ROOT"

if ! command -v cargo >/dev/null 2>&1 && [[ -f "${HOME}/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "${HOME}/.cargo/env"
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: cargo is required for the sandbox-core unit checks"
  exit 2
fi

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "SKIP: Phase 1 OS smoke tests require Linux"
  exit 77
fi

printf '%s\n' '== sandbox-core safety/unit checks =='
cargo test -p sandbox-core linux::supervisor::tests -- --nocapture
cargo test -p sandbox-core linux::cgroup::tests -- --nocapture

printf '%s\n' '== cgroup v2 delegated-root preflight =='
set +e
bash scripts/security/cgroup-v2-smoke.sh
cgroup_status=$?
set -e
if [[ "$cgroup_status" -ne 0 && "$cgroup_status" -ne 77 ]]; then
  exit "$cgroup_status"
fi

printf '%s\n' '== seccomp kernel primitive probe =='
set +e
python3 scripts/security/seccomp-kernel-smoke.py
seccomp_status=$?
set -e
if [[ "$seccomp_status" -ne 0 && "$seccomp_status" -ne 77 ]]; then
  exit "$seccomp_status"
fi

printf '%s\n' '== namespace visibility probe =='
if [[ "${CORTEXENV_RUN_ISOLATION_SMOKE:-}" == "1" ]]; then
  set +e
  bash scripts/security/phase1-isolation-smoke.sh
  namespace_status=$?
  set -e
else
  echo "SKIP: set CORTEXENV_RUN_ISOLATION_SMOKE=1 to opt in to namespace creation"
  namespace_status=77
fi
if [[ "$namespace_status" -ne 0 && "$namespace_status" -ne 77 ]]; then
  exit "$namespace_status"
fi

if [[ "$cgroup_status" -eq 77 || "$seccomp_status" -eq 77 || "$namespace_status" -eq 77 ]]; then
  echo "PASS: safe checks passed; one or more kernel controls were unavailable and were reported as SKIP"
  exit 77
fi

echo "PASS: Phase 1 namespace and seccomp smoke tests"
