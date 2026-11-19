#!/usr/bin/env bash
# Publish the tables of a benchmark run to one page on the bench-results branch.
# Usage: dev/publish-bench.sh <artifact> <page.md> "<where the models ran>"
# Runs in CI after the benchmark job, on main only. It needs GH_TOKEN and the GITHUB_* variables.
# The branch must exist (see docs/benchmark.md).
set -euo pipefail
artifact=$1 page=$2 where=$3
dir=$(mktemp -d)
gh run download "$GITHUB_RUN_ID" --repo "$GITHUB_REPOSITORY" --name "$artifact" --dir "$dir"
# The bench writes page.md without the methods that it excluded (errors on more than 1% of their cases). The page is
# skipped when d3, the anchor of every run, is excluded or fewer than 2 methods remain (the 27b run has one hosted
# model: if it is bad, the page keeps its old numbers).
if jq -e 'any(.excluded[]; . == "d3") or (.methods | length) - (.excluded | length) < 2' "$dir/summary.json" >/dev/null; then
  echo "::warning::$page not updated: d3 or all hosted models had errors on more than 1% of their cases."
  exit 0
fi
{
  printf '# Benchmark: %s\n\n%s\n\n' "${page%.md}" "$where"
  printf 'Run on %s, 8-ball commit %s, [run](%s/%s/actions/runs/%s).\n\n' \
    "$(date -u +%F)" "$GITHUB_SHA" "$GITHUB_SERVER_URL" "$GITHUB_REPOSITORY" "$GITHUB_RUN_ID"
  cat "$dir/page.md"
} >"$dir/published.md"
api="repos/$GITHUB_REPOSITORY/contents/$page"
sha=$(gh api "$api?ref=bench-results" --jq .sha 2>/dev/null || true)
gh api -X PUT "$api" -f branch=bench-results -f message="bench: $page at ${GITHUB_SHA::7}" \
  -f content="$(base64 -w0 "$dir/published.md")" ${sha:+-f sha="$sha"} >/dev/null
