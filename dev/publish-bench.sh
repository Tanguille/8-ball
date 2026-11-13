#!/usr/bin/env bash
# Publish the tables of a benchmark run to one page on the bench-results branch.
# Usage: dev/publish-bench.sh <artifact> <page.md> "<where the models ran>"
# Runs in CI after the benchmark job, on main only. It needs GH_TOKEN and the GITHUB_* variables.
# The branch must exist: git switch --orphan bench-results && git commit --allow-empty -m "chore: start" && git push origin bench-results
set -euo pipefail
artifact=$1 page=$2 where=$3
dir=$(mktemp -d)
gh run download "$GITHUB_RUN_ID" --repo "$GITHUB_REPOSITORY" --name "$artifact" --dir "$dir"
# A rate-limited or half-failed run must not replace good numbers: skip a page when a model has errors on more than 1%
# of its cases in total.
if ! jq -e '[.methods[] | ([.[] | .errors] | add) * 100 <= ([.[] | .cases] | add)] | all' "$dir/summary.json" >/dev/null; then
  echo "::warning::$page not updated: a model had errors on more than 1% of its cases."
  exit 0
fi
{
  printf '# Benchmark: %s\n\n%s\n\n' "${page%.md}" "$where"
  printf 'Run on %s, 8-ball commit %s, [run](%s/%s/actions/runs/%s).\n\n' \
    "$(date -u +%F)" "$GITHUB_SHA" "$GITHUB_SERVER_URL" "$GITHUB_REPOSITORY" "$GITHUB_RUN_ID"
  cat "$dir/report.md"
} >"$dir/page.md"
api="repos/$GITHUB_REPOSITORY/contents/$page"
sha=$(gh api "$api?ref=bench-results" --jq .sha 2>/dev/null || true)
gh api -X PUT "$api" -f branch=bench-results -f message="bench: $page at ${GITHUB_SHA::7}" \
  -f content="$(base64 -w0 "$dir/page.md")" ${sha:+-f sha="$sha"} >/dev/null
