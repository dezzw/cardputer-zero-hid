#!/usr/bin/env bash
# Fail if an ELF binary needs GLIBC newer than MAX_VERSION (default 2.41).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN=${1:-}
MAX=${2:-2.41}

if [[ -z "$BIN" ]]; then
  echo "usage: verify-glibc-max.sh <elf-binary> [max-version]" >&2
  exit 2
fi

required="$("$ROOT/packaging/glibc-required-version.sh" "$BIN")"
echo "Highest GLIBC symbol version: $required (max allowed: $MAX)"

if dpkg --compare-versions "$required" gt "$MAX"; then
  echo "error: binary requires GLIBC $required > $MAX" >&2
  exit 1
fi

echo "GLIBC requirement OK"