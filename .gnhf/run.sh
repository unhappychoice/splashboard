#!/usr/bin/env bash
# Launch overnight gnhf loops in parallel worktrees.
# See .gnhf/README.md for the full workflow.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROMPTS="$ROOT/.gnhf/prompts"
TOKEN_CAP="${GNHF_MAX_TOKENS:-5000000}"

if ! command -v gnhf >/dev/null 2>&1; then
  echo "gnhf not found. install: npm install -g gnhf" >&2
  exit 1
fi

if [ -n "$(git -C "$ROOT" status --porcelain)" ]; then
  echo "working tree not clean — gnhf --worktree requires a clean tree" >&2
  exit 1
fi

cd "$ROOT"

cat "$PROMPTS/coverage-loop.md" | gnhf --worktree --max-tokens "$TOKEN_CAP" \
  > /tmp/gnhf-coverage.log 2>&1 &
COVERAGE_PID=$!

cat "$PROMPTS/perf-loop.md" | gnhf --worktree --max-tokens "$TOKEN_CAP" \
  > /tmp/gnhf-perf.log 2>&1 &
PERF_PID=$!

disown "$COVERAGE_PID" "$PERF_PID" 2>/dev/null || true

cat <<MSG
launched two gnhf loops:
  coverage-loop  pid=$COVERAGE_PID  log=/tmp/gnhf-coverage.log
  perf-loop      pid=$PERF_PID      log=/tmp/gnhf-perf.log

token cap per loop: $TOKEN_CAP (override with GNHF_MAX_TOKENS=...)
worktrees will appear under: $(cd .. && pwd)/splashboard-gnhf-worktrees/

morning checklist:
  ls ../splashboard-gnhf-worktrees/
  tail /tmp/gnhf-coverage.log /tmp/gnhf-perf.log
MSG
