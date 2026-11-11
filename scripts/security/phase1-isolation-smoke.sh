#!/usr/bin/env bash
set -euo pipefail

# This script is intentionally opt-in. It creates namespaces in a child
# process, but it must still be run only inside a disposable VM or isolated CI
# runner. It never submits code to the CortexEnv API.
if [[ "${CORTEXENV_RUN_ISOLATION_SMOKE:-}" != "1" ]]; then
  echo "SKIP: set CORTEXENV_RUN_ISOLATION_SMOKE=1 in a disposable Linux VM to run this test"
  exit 0
fi

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "SKIP: namespace smoke test requires Linux"
  exit 77
fi
if [[ "$(id -u)" == "0" ]]; then
  echo "ERROR: refusing to run namespace smoke test as root"
  exit 2
fi
if ! command -v unshare >/dev/null 2>&1; then
  echo "SKIP: util-linux unshare is not installed"
  exit 77
fi

# All assertions execute below unshare --fork, so a failure cannot alter the
# caller's namespace membership. --map-root-user maps only the child to uid 0
# inside its private user namespace; it does not grant host root privileges.
unshare --user --map-root-user --mount --pid --fork --net --uts --ipc /bin/sh -eu -c '
  test "$(id -u)" = 0
  test -r /proc/self/uid_map
  test -r /proc/self/gid_map
  test "$(readlink /proc/self/ns/user)" != "$(readlink /proc/1/ns/user)"
  test "$(readlink /proc/self/ns/mnt)" != "$(readlink /proc/1/ns/mnt)"
  test "$(readlink /proc/self/ns/net)" != "$(readlink /proc/1/ns/net)"
  test "$(readlink /proc/self/ns/uts)" != "$(readlink /proc/1/ns/uts)"
  test "$(readlink /proc/self/ns/ipc)" != "$(readlink /proc/1/ns/ipc)"
  nspid_count=$(awk "/^NSpid:/ { print NF - 1 }" /proc/self/status)
  test "$nspid_count" -ge 2
  printf "namespace smoke: uid=%s pid-levels=%s user=%s mount=%s net=%s uts=%s ipc=%s\\n" \
    "$(id -u)" "$nspid_count" "$(readlink /proc/self/ns/user)" \
    "$(readlink /proc/self/ns/mnt)" "$(readlink /proc/self/ns/net)" \
    "$(readlink /proc/self/ns/uts)" "$(readlink /proc/self/ns/ipc)"
'

echo "PASS: namespace visibility and unprivileged user mapping"
