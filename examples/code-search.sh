#!/usr/bin/env bash
# Search code by behaviour: ripgrep finds candidate blocks, 8-ball keeps the ones that match.
# Usage: examples/code-search.sh "<what the code does>" <rg pattern> [rg flags and paths, default .]
# Example: examples/code-search.sh "retry a failed request" 'retry|backoff' backend service -g '!**/tests/**'
set -euo pipefail
question=$1 pattern=$2
shift 2

# rg exits 1 when nothing matches: that is an empty result, not an error. With stdin on /dev/null, rg searches the
# current directory when it gets no path, not stdin.
matches=$(rg -n --json -C 12 -e "$pattern" "$@" </dev/null) || [ $? -eq 1 ]

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
  echo "No code matches $pattern." >&2
  exit 0
fi

# --no-learn: exploration never turns a skipped block into `unsure`.
result=$(jq -c 'map(.code)' <<<"$blocks" | 8ball triage --no-learn "Does this code $question?")

# triage returns indices into its input, so map them back to the blocks. A file, not --argjson: one argument is
# limited to 128 KiB.
jq -c --slurpfile b <(printf '%s' "$blocks") '
    ((.kept[] | {verdict: "match", index, c: .decision.confidence}),
     (.unsure[] | {verdict: "check", index, c: .decision.confidence}))
    | $b[0][.index] as $block
    | {verdict, file: $block.file, lines: "\($block.start)-\($block.end)", confidence: (.c * 100 | round / 100)}' <<<"$result"
jq -r --argjson n "$count" '"\($n) blocks: \(.kept | length) match, \(.unsure | length) check, \(.rejected | length) no"' <<<"$result" >&2
