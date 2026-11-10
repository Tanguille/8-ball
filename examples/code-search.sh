#!/usr/bin/env bash
# Search code by behaviour: ripgrep finds candidate blocks, 8-ball keeps the ones that match.
# Usage: examples/code-search.sh "<what the code does>" <rg pattern> [path]
set -euo pipefail
question=$1 pattern=$2 path=${3:-.}

# rg exits 1 when nothing matches: that is an empty result, not an error.
matches=$(rg -n --json -C 12 "$pattern" "$path") || [ $? -eq 1 ]

# Join each match and its 12 context lines into one block per file region.
blocks=$(jq -s -c '
  [ .[] | select(.type == "match" or .type == "context") | .data
    | {file: .path.text, line: .line_number, text: .lines.text} ]
  | group_by(.file)
  | map(sort_by(.line) | reduce .[] as $l ([];
      if length > 0 and $l.line <= .[-1].end + 1
      then .[-1].end = $l.line | .[-1].code += $l.text
      else . + [{file: $l.file, start: $l.line, end: $l.line, code: $l.text}] end))
  | flatten' <<<"$matches")

count=$(jq length <<<"$blocks")
if ((count == 0)); then
  echo "No code matches $pattern in $path." >&2
  exit 0
fi
if ((count > 64)); then
  echo "$count blocks; triage takes at most 64. Use narrower keywords or a path." >&2
  exit 1
fi

# triage returns indices into its input, so map them back to the blocks.
echo "$blocks" | jq -c 'map(.code)' \
  | 8ball triage "Does this code $question?" \
  | jq -c --argjson b "$blocks" '
      ((.kept[] | {verdict: "match", index, c: .decision.confidence}),
       (.unsure[] | {verdict: "check", index, c: .decision.confidence}))
      | {verdict, file: $b[.index].file, lines: "\($b[.index].start)-\($b[.index].end)", confidence: (.c * 100 | round / 100)}'
