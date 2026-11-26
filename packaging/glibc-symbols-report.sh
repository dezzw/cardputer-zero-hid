#!/usr/bin/env bash
# List GLIBC_2.35+ dynamic symbol version tags for an ELF (for CI visibility).
set -euo pipefail

BIN=${1:-}
if [[ -z "$BIN" || ! -f "$BIN" ]]; then
  echo "usage: glibc-symbols-report.sh <elf-binary>" >&2
  exit 2
fi

OBJDUMP="${OBJDUMP:-objdump}"
if ! "$OBJDUMP" -T "$BIN" >/dev/null 2>&1; then
  for candidate in aarch64-linux-gnu-objdump objdump; do
    if command -v "$candidate" >/dev/null 2>&1 && "$candidate" -T "$BIN" >/dev/null 2>&1; then
      OBJDUMP=$candidate
      break
    fi
  done
fi

mapfile -t rows < <(
  "$OBJDUMP" -T "$BIN" 2>/dev/null \
    | awk '/GLIBC_2\.(3[5-9]|[4-9][0-9])/ {
        for (i = 1; i <= NF; i++) {
          if ($i ~ /^GLIBC_2\.(3[5-9]|[4-9][0-9])/) { print $0; break }
        }
      }' \
    | sort -u
)

if [[ ${#rows[@]} -eq 0 ]]; then
  echo "(no symbols tagged GLIBC_2.35 or newer)"
else
  printf '%s\n' "${rows[@]}"
fi