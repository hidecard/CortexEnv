#!/usr/bin/env bash
set -euo pipefail

if [[ "${CORTEXENV_RUN_ISOLATION_SMOKE:-}" != "1" ]]; then
  echo "SKIP: set CORTEXENV_RUN_ISOLATION_SMOKE=1 in a disposable Linux VM to run this test"
  exit 0
fi
if [[ "$(uname -s)" != "Linux" ]]; then
  echo "SKIP: cgroup v2 preflight requires Linux"
  exit 77
fi

root="${CORTEXENV_CGROUP_ROOT:-/sys/fs/cgroup}"
if [[ ! -f "$root/cgroup.controllers" ]]; then
  echo "SKIP: no cgroup v2 controller file at $root/cgroup.controllers"
  exit 77
fi

controllers=$(<"$root/cgroup.controllers")
for controller in cpu memory pids; do
  case " $controllers " in
    *" $controller "*) ;;
    *) echo "SKIP: required cgroup controller '$controller' is unavailable"; exit 77 ;;
  esac
done

for file in memory.max memory.swap.max pids.max cpu.max; do
  if [[ ! -e "$root/$file" ]]; then
    echo "SKIP: required cgroup v2 file is missing: $root/$file"
    exit 77
  fi
done

if [[ ! -w "$root/cgroup.procs" ]]; then
  echo "SKIP: cgroup root is visible but not writable; use a delegated subtree for enforcement tests"
  exit 77
fi

printf 'PASS: delegated cgroup v2 preflight root=%s controllers=%s\n' "$root" "$controllers"
printf 'limits: memory.max=%s memory.swap.max=%s pids.max=%s cpu.max=%s\n' \
  "$(<"$root/memory.max")" "$(<"$root/memory.swap.max")" \
  "$(<"$root/pids.max")" "$(<"$root/cpu.max")"
