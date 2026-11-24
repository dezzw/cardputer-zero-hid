#!/usr/bin/env bash
# Print the highest GLIBC_X.Y version referenced in an ELF dynamic symbol table.
# Example: GLIBC_2.38 -> 2.38
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: glibc-required-version.sh <elf-binary>" >&2
  exit 2
fi

BIN=$1
if [[ ! -f "$BIN" ]]; then
  echo "error: not a file: $BIN" >&2
  exit 1
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

mapfile -t versions < <(
  "$OBJDUMP" -T "$BIN" 2>/dev/null | grep -oE 'GLIBC_[0-9]+(\.[0-9]+)*' | sed 's/^GLIBC_//' | sort -t. -k1,1n -k2,2n -V -u
)

if [[ ${#versions[@]} -eq 0 ]]; then
  echo "error: no GLIBC version symbols found in $BIN" >&2
  exit 1
fi

echo "${versions[$((${#versions[@]} - 1))]}"