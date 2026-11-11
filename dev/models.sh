#!/usr/bin/env bash
# Start/stop the local model stack for live tests. Usage: dev/models.sh up|bench|down|status|logs [service]
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
EIGHTBALL_DEV_UID="$(id -u)"
EIGHTBALL_DEV_GID="$(id -g)"
# Rootless Podman maps only container root to the host user, so any other id cannot write the cache.
if [ "$(docker info --format '{{.Host.Security.Rootless}}' 2>/dev/null)" = true ]; then
  EIGHTBALL_DEV_UID=0
  EIGHTBALL_DEV_GID=0
fi
export EIGHTBALL_DEV_UID EIGHTBALL_DEV_GID
mkdir -p "$HOME/.cache/8ball-models"
case "${1:-status}" in
  up) docker compose up -d --wait ;;
  bench) docker compose --profile bench up -d --wait big ;;
  down) docker compose --profile bench down ;;
  status)
    docker compose ps
    for p in 9931 9932 9933 9934; do
      printf '%s ' "$p"
      curl -fsS "http://127.0.0.1:$p/health" || printf 'DOWN'
      echo
    done
    ;;
  logs) docker compose logs --tail=100 ${2:+"$2"} ;;
  *)
    echo "usage: $0 up|bench|down|status|logs [service]" >&2
    exit 2
    ;;
esac
