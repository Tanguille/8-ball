#!/usr/bin/env bash
# Publish the tables of a benchmark run to one page on the bench-results branch.
# Usage: dev/publish-bench.sh <artifact> <page.md> "<where the models ran>"
# Runs in CI after the benchmark job, on main only. It needs GH_TOKEN and the GITHUB_* variables.
# The branch must exist: git switch --orphan bench-results && git commit --allow-empty -m "chore: start" && git push origin bench-results
set -euo pipefail
artifact=$1 page=$2 where=$3
dir=$(mktemp -d)
gh run download "$GITHUB_RUN_ID" --repo "$GITHUB_REPOSITORY" --name "$artifact" --dir "$dir"
# A model with errors on more than 1% of its cases in total stays off the page, so one flaky model does not hide the
# others. A cascade goes with either model it uses. The page is skipped when d3, the anchor of every run, is excluded
# or fewer than 2 methods remain (the 27b run has one hosted model: if it is bad, the page keeps its old numbers).
drop=$(jq -r '
  def parts: if contains(", then ") then split(", then ") | [.[0], (.[1] | sub(" for unsure$"; ""))] else [.] end;
  [.methods | to_entries[] | select(([.value[] | .errors] | add) * 100 > ([.value[] | .cases] | add)) | .key] as $bad
  | .methods | keys[] | select(. as $name | any(parts[]; . as $part | $bad | index($part)))' "$dir/summary.json")
kept=$(jq --arg drop "$drop" '[.methods | keys[] | select(. as $name | ($drop | split("\n") | index($name)) | not)] | length' "$dir/summary.json")
if printf '%s\n' "$drop" | grep -qx d3 || [ "$kept" -lt 2 ]; then
  echo "::warning::$page not updated: d3 or all hosted models had errors on more than 1% of their cases."
  exit 0
fi
{
  printf '# Benchmark: %s\n\n%s\n\n' "${page%.md}" "$where"
  printf 'Run on %s, 8-ball commit %s, [run](%s/%s/actions/runs/%s).\n\n' \
    "$(date -u +%F)" "$GITHUB_SHA" "$GITHUB_SERVER_URL" "$GITHUB_REPOSITORY" "$GITHUB_RUN_ID"
  if [ -n "$drop" ]; then
    printf 'Excluded, with errors on more than 1%% of their cases: %s.\n\n' "$(printf '%s\n' "$drop" | awk 'NR > 1 { printf ", " } { printf "%s", $0 }')"
  fi
  # Leave the rows of excluded methods out of every table (the label is the first cell).
  awk -v drop="$drop" 'BEGIN { n = split(drop, names, "\n") }
    { for (i = 1; i <= n; i++) if (names[i] != "" && (index($0, "| " names[i] " (") == 1 || index($0, "| " names[i] " |") == 1)) next; print }' "$dir/report.md"
} >"$dir/page.md"
api="repos/$GITHUB_REPOSITORY/contents/$page"
sha=$(gh api "$api?ref=bench-results" --jq .sha 2>/dev/null || true)
gh api -X PUT "$api" -f branch=bench-results -f message="bench: $page at ${GITHUB_SHA::7}" \
  -f content="$(base64 -w0 "$dir/page.md")" ${sha:+-f sha="$sha"} >/dev/null
