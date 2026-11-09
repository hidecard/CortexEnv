#!/usr/bin/env bash
set -euo pipefail

container_name="${1:-cortexenv-smoke}"
port="${2:-18080}"
host="${SMOKE_HOST:-127.0.0.1}"
cleanup() {
  docker rm -f "$container_name" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker run --detach --name "$container_name" --publish "${port}:8080" \
  --cap-drop=ALL --security-opt=no-new-privileges \
  cortexenv:ci >/dev/null

for _ in $(seq 1 30); do
  if curl --fail --silent "http://${host}:${port}/health" >/dev/null; then
    echo "CortexEnv Docker health check passed"
    exit 0
  fi
  sleep 2
done

echo "CortexEnv Docker health check failed" >&2
docker logs "$container_name" >&2 || true
exit 1
