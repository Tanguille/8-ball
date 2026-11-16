#!/usr/bin/env bash
# Test dev/publish-bench.sh against a stub gh: which methods stay on the page, and when the page is skipped.
# Usage: dev/test-publish-bench.sh (needs jq and awk; the shell lint job runs it).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failures=0
cascade='d3, then hosted-qwen for unsure'

# Prints the summary entry of one method with 100 cases.
# Arguments: $1 method name, $2 errors out of 100.
# Outputs: a JSON member on stdout.
method() { printf '"%s":{"spam":{"cases":100,"errors":%s}}' "$1" "$2"; }

# Runs the script on a summary, so that $tmp/page.md exists only if it published.
# Globals: root, tmp.
# Arguments: $1 the summary JSON.
# Outputs: the script output in $tmp/out.
publish() {
  rm -f "$tmp/page.md"
  printf '%s\n' "$1" >"$tmp/summary.json"
  "$root/dev/publish-bench.sh" bench x.md where >"$tmp/out" 2>&1
}

# Tells whether the page has a table row for a method (the label is the first cell).
# Globals: tmp.
# Arguments: $1 method name.
# Outputs: nothing, only the exit status.
row() { grep -qF -- "| $1 (" "$tmp/page.md" || grep -qF -- "| $1 |" "$tmp/page.md"; }

# Negates a command.
# Arguments: the command.
# Outputs: nothing, only the exit status.
not() { ! "$@"; }

# Runs one check and counts a failure.
# Globals: failures.
# Arguments: $1 label, then the command.
# Outputs: the label on stderr if the check fails.
check() {
  local label=$1
  shift
  "$@" || { echo "FAIL: $label" >&2 && failures=$((failures + 1)); }
}

# Writes the stub gh and the report that it hands out, then checks every case.
# Globals: tmp, cascade, failures.
# Outputs: a summary line on stdout, the failed checks on stderr.
# Returns: 1 if a check failed.
main() {
  mkdir "$tmp/bin"
  # `gh run download ... --dir <dir>` copies the fixtures; a PUT with `-f content=<base64>` keeps the page.
  cat >"$tmp/bin/gh" <<'EOF'
#!/usr/bin/env bash
if [ "$1 $2" = "run download" ]; then
  while [ $# -gt 0 ]; do [ "$1" = --dir ] && cp "$FIXTURES"/summary.json "$FIXTURES"/report.md "$2"; shift; done
  exit 0
fi
for arg in "$@"; do case $arg in content=*) echo "${arg#content=}" | base64 -d >"$FIXTURES/page.md" ;; esac; done
EOF
  chmod +x "$tmp/bin/gh"
  local names=(d3 hosted-clef hosted-clef-flash hosted-gemma hosted-qwen "$cascade")
  {
    printf '| Method | Decided |\n| --- | --- |\n'
    for name in "${names[@]}"; do echo "| $name (m) | 90% |"; done
    printf '\n| Method | Task |\n| --- | --- |\n'
    for name in "${names[@]}"; do echo "| $name | spam |"; done
  } >"$tmp/report.md"
  export FIXTURES=$tmp GITHUB_RUN_ID=1 GITHUB_REPOSITORY=o/r GITHUB_SERVER_URL=https://github.com GITHUB_SHA=abc1234
  PATH=$tmp/bin:$PATH
  local base rest
  base="$(method d3 0),$(method hosted-clef 0),$(method hosted-clef-flash 0)"
  rest="$(method hosted-gemma 0),$(method hosted-qwen 0),$(method "$cascade" 0)"

  publish "{\"methods\":{$base,$rest}}"
  check "a clean run publishes every row" row hosted-gemma
  check "a clean run has no Excluded line" not grep -q '^Excluded' "$tmp/page.md"
  publish "{\"methods\":{$base,$(method hosted-gemma 40),$(method hosted-qwen 0),$(method "$cascade" 0)}}"
  check "a bad model is named in the Excluded line" grep -q '^Excluded.*hosted-gemma' "$tmp/page.md"
  check "a bad model has no row" not row hosted-gemma
  check "a good cascade stays" row "$cascade"
  publish "{\"methods\":{$(method d3 0),$(method hosted-clef 40),$(method hosted-clef-flash 0),$rest}}"
  check "a bad hosted-clef leaves its rows out" not row hosted-clef
  check "a bad hosted-clef keeps hosted-clef-flash" row hosted-clef-flash
  publish "{\"methods\":{$base,$(method hosted-gemma 0),$(method hosted-qwen 40),$(method "$cascade" 0)}}"
  check "a bad model takes its cascade with it" not row "$cascade"
  check "the cascade is named in the Excluded line" grep -q "^Excluded.*$cascade" "$tmp/page.md"
  check "d3 stays" row d3
  publish "{\"methods\":{$(method d3 40),$(method hosted-clef 0),$(method "$cascade" 0)}}"
  check "a bad d3 skips the page" test ! -f "$tmp/page.md"
  check "a skipped page warns" grep -q 'not updated' "$tmp/out"
  publish "{\"methods\":{$(method d3 0),$(method hosted-qwen 40),$(method "$cascade" 0)}}"
  check "a bad only hosted model skips the page" test ! -f "$tmp/page.md"

  if [ "$failures" -gt 0 ]; then echo "$failures check(s) failed" >&2 && return 1; fi
  echo "publish-bench: all checks passed"
}
main "$@"
