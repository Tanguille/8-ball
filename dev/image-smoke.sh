#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
image="${1:?usage: image-smoke.sh IMAGE}"

docker run --rm "$image" --version
container=$(docker run -d --read-only --cap-drop ALL --security-opt no-new-privileges \
  --tmpfs /data:rw,mode=1777 \
  -p 127.0.0.1::8080 \
  -v "$PWD/examples/hermes/config.toml:/config.toml:ro,Z" \
  -e EIGHTBALL_API_KEY=smoke-test \
  "$image" serve --config /config.toml --db /data/8ball.db)
trap 'docker logs "$container" >&2; docker rm -f "$container" >/dev/null' EXIT
address=$(docker port "$container" 8080/tcp)
test -n "$address"
curl --retry 10 --retry-delay 1 --retry-all-errors --max-time 5 -fsS "http://$address/healthz"
test "$(curl --max-time 5 -s -o /dev/null -w '%{http_code}' "http://$address/v1/doctor")" = 401
# Empty triage exercises authentication and the HTTP contract without a model server.
response=$(curl --max-time 5 -fsS "http://$address/v1/triage" \
  -H 'Authorization: Bearer smoke-test' -H 'Content-Type: application/json' \
  -d '{"spec":"gitops-investigate","items":[]}')
jq -e '.kept == [] and .unsure == [] and .rejected == [] and .prefiltered == [] and .dropped == 0' <<< "$response"
