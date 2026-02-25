#!/usr/bin/env bash
# Run go-readability and readability-rs benchmarks and print results side by side.
#
# Usage:
#   ./benches/compare.sh [--quick]
#
# Options:
#   --quick   Use -benchtime=1s / --sample-size=10 for a faster run.

set -euo pipefail
cd "$(dirname "$0")/.."

QUICK=0
for arg in "$@"; do [[ "$arg" == "--quick" ]] && QUICK=1; done

if [[ $QUICK -eq 1 ]]; then
  GO_TIME="-benchtime=1s"
  RS_ARGS="--bench extraction -- --sample-size 10"
else
  GO_TIME="-benchtime=5s"
  RS_ARGS="--bench extraction"
fi

# ── run Go benchmarks ─────────────────────────────────────────────────────────
echo "==> Running Go benchmarks…"
GO_OUT=$(cd benches/go && go test -bench=. -benchmem $GO_TIME -run='^$' 2>/dev/null)

# ── run Rust benchmarks ───────────────────────────────────────────────────────
echo "==> Running Rust benchmarks…"
RS_OUT=$(cargo bench $RS_ARGS 2>/dev/null)

# ── helpers ───────────────────────────────────────────────────────────────────

# Extract throughput (MiB/s) from Go output for a given benchmark function name.
# Go format: BenchmarkFoo-16    1234   5678 ns/op   99.99 MB/s   ...
go_thrpt() {
  local fn="$1"
  echo "$GO_OUT" \
    | grep -E "^Benchmark${fn}-" \
    | awk '{print $5}' \
    | head -1
}

# Extract throughput (MiB/s) from Rust/criterion output for a given group/bench name.
# Criterion format (indented):  thrpt:  [X MiB/s  Y MiB/s  Z MiB/s]
# Fields: thrpt:  [low  unit  mid  unit  high  unit]  → $4 is the middle estimate.
rs_thrpt() {
  local name="$1"
  echo "$RS_OUT" \
    | grep -A2 "^${name}/parse" \
    | grep "thrpt:" \
    | awk '{print $4}' \
    | head -1
}

# Extract median time from Rust criterion output.
# Criterion format:  foo/parse   time:   [X unit  Y unit  Z unit]
# Fields (after leading name):  time:  [low  unit  mid  unit  high  unit]
# We want the middle estimate: fields 5+6 of the full line.
rs_time() {
  local name="$1"
  echo "$RS_OUT" \
    | grep -E "^${name}/parse[[:space:]]" \
    | awk '{print $5, $6}' \
    | head -1
}

# Extract time from Go output (ns/op → human readable).
go_time() {
  local fn="$1"
  local ns
  ns=$(echo "$GO_OUT" \
    | grep -E "^Benchmark${fn}-" \
    | awk '{print $3}' \
    | head -1)
  if [[ -z "$ns" ]]; then echo "n/a"; return; fi
  # Convert ns to µs or ms
  local us
  us=$(awk "BEGIN {printf \"%.1f\", $ns/1000}")
  if awk "BEGIN {exit !($us >= 1000)}"; then
    awk "BEGIN {printf \"%.2f ms\", $us/1000}"
  else
    echo "${us} µs"
  fi
}

# Extract all-fixtures time from Go (ns/op for the whole suite per iteration).
go_suite_time() {
  local ns
  ns=$(echo "$GO_OUT" \
    | grep -E "^BenchmarkAllFixtures-" \
    | awk '{print $3}' \
    | head -1)
  if [[ -z "$ns" ]]; then echo "n/a"; return; fi
  awk "BEGIN {printf \"%.0f ms\", $ns/1000000}"
}

rs_suite_time() {
  echo "$RS_OUT" \
    | grep -E "^all_fixtures/133_pages[[:space:]]" \
    | awk '{print $5, $6}' \
    | head -1
}

# ── print table ───────────────────────────────────────────────────────────────

printf "\n%-20s  %12s  %12s  %12s  %12s\n" \
  "Page (~size)" "Go time" "Go thrpt" "Rust time" "Rust thrpt"
printf "%-20s  %12s  %12s  %12s  %12s\n" \
  "--------------------" "------------" "------------" "------------" "------------"

pages=(
  "ars-1:Ars1:ars-1:~56 KB"
  "wapo-1:Wapo1:wapo-1:~180 KB"
  "wikipedia:Wikipedia:wikipedia:~244 KB"
  "nytimes-3:Nytimes3:nytimes-3:~489 KB"
  "yahoo-2:Yahoo2:yahoo-2:~1.6 MB"
)

for entry in "${pages[@]}"; do
  IFS=: read -r label go_fn rs_name size <<< "$entry"
  gt=$(go_time "$go_fn")
  gtp=$(go_thrpt "$go_fn")
  rt=$(rs_time "$rs_name")
  rtp=$(rs_thrpt "$rs_name")
  printf "%-20s  %12s  %10s MB/s  %12s  %10s MiB/s\n" \
    "${label} (${size})" "${gt:-n/a}" "${gtp:-n/a}" "${rt:-n/a}" "${rtp:-n/a}"
done

printf "%-20s  %12s  %12s  %12s  %12s\n" \
  "--------------------" "------------" "------------" "------------" "------------"

# All-fixtures row
printf "%-20s  %12s  %12s  %12s  %12s\n" \
  "all fixtures" \
  "$(go_suite_time)" \
  "" \
  "$(rs_suite_time)" \
  ""

echo ""
