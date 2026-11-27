#!/usr/bin/env bash
# List dynamic symbols whose version is GLIBC_2.35 .. GLIBC_2.99 (one line each: "sym GLIBC_x.y").
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
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

if ! "$OBJDUMP" -T "$BIN" >/dev/null 2>&1; then
  echo "error: cannot read dynamic symbols from $BIN with $OBJDUMP" >&2
  exit 1
fi

mapfile -t rows < <(
  "$OBJDUMP" -T "$BIN" 2>/dev/null | awk '
  function ver_ge_235(ver,   n, rest) {
    if (ver !~ /^2\./) return 0
    rest = ver
    sub(/^2\./, "", rest)
    n = rest + 0
    return (n >= 35 && n <= 99)
  }
  {
    if (match($0, /\(GLIBC_2\.[0-9]+\)/)) {
      tag = substr($0, RSTART + 1, RLENGTH - 2)
      ver = tag
      sub(/^GLIBC_/, "", ver)
      if (!ver_ge_235(ver)) next
      sym = $NF
      if (sym == "" || sym ~ /^\(GLIBC_/) next
      print sym, tag
    }
  }' | sort -u
)

if [[ ${#rows[@]} -eq 0 ]]; then
  required="$("$ROOT/packaging/glibc-required-version.sh" "$BIN")"
  if dpkg --compare-versions "$required" ge 2.35; then
    echo "error: binary highest GLIBC is $required but no GLIBC_2.35+ symbols were listed (check $OBJDUMP parsing)" >&2
    exit 1
  fi
  echo "(no symbols tagged GLIBC_2.35 or newer)"
else
  printf '%s\n' "${rows[@]}"
fi