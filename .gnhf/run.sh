#!/usr/bin/env bash
# Launch a gnhf loop. Pick which one with the first argument.
# See .gnhf/README.md for the full workflow.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROMPTS="$ROOT/.gnhf/prompts"
TOKEN_CAP="${GNHF_MAX_TOKENS:-5000000}"

usage() {
  {
    echo "usage: $0 <loop-name>"
    echo
    echo "available loops:"
    for p in "$PROMPTS"/*-loop.md; do
      [ -f "$p" ] || continue
      name="$(basename "$p" -loop.md)"
      echo "  $name"
    done
    echo
    echo "token cap: \$GNHF_MAX_TOKENS (default $TOKEN_CAP)"
  } >&2
  exit 2
}

[ $# -eq 1 ] || usage

NAME="$1"
PROMPT="$PROMPTS/$NAME-loop.md"

if [ ! -f "$PROMPT" ]; then
  echo "no such loop: $NAME ($PROMPT not found)" >&2
  usage
fi

if ! command -v gnhf >/dev/null 2>&1; then
  echo "gnhf not found. install: npm install -g gnhf" >&2
  exit 1
fi

if [ -n "$(git -C "$ROOT" status --porcelain)" ]; then
  echo "working tree not clean — commit or stash before launching gnhf" >&2
  exit 1
fi

cd "$ROOT"

exec gnhf --max-tokens "$TOKEN_CAP" < "$PROMPT"
