#!/usr/bin/env bash
# Exercise postinst restart helper against a stub systemctl (no real systemd).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STUB_BIN="$(mktemp -d)"
trap 'rm -rf "$STUB_BIN"' EXIT

mkdir -p "${STUB_BIN}/bin"
cat >"${STUB_BIN}/bin/systemctl" <<'STUB'
#!/bin/sh
case "$1" in
  daemon-reload) exit 0 ;;
  enable) exit 0 ;;
  show)
    if [ "$2" = "-p" ] && [ "$3" = "ExecMainStatus" ] && [ "$4" = "--value" ]; then
      printf '%s\n' "${SYSTEMCTL_EXEC_MAIN_STATUS:-3}"
    fi
    exit 0
    ;;
  restart)
    exit "${SYSTEMCTL_RESTART_EXIT:-1}"
    ;;
  *)
    echo "unexpected systemctl: $*" >&2
    exit 99
    ;;
esac
STUB
chmod +x "${STUB_BIN}/bin/systemctl"

PATH="${STUB_BIN}/bin:$PATH"
export PATH

# Exit 3 from ExecMainStatus must let postinst configure succeed (coexistence).
SYSTEMCTL_RESTART_EXIT=1
SYSTEMCTL_EXEC_MAIN_STATUS=3
sh "${ROOT}/packaging/debian/postinst" configure

# Real failure (exit 1, not status 3) must fail configure.
SYSTEMCTL_RESTART_EXIT=1
SYSTEMCTL_EXEC_MAIN_STATUS=1
if sh "${ROOT}/packaging/debian/postinst" configure 2>/dev/null; then
  echo "error: postinst should fail when restart fails with ExecMainStatus=1" >&2
  exit 1
fi

# Clean success path.
SYSTEMCTL_RESTART_EXIT=0
SYSTEMCTL_EXEC_MAIN_STATUS=0
sh "${ROOT}/packaging/debian/postinst" configure

echo "postinst restart tests ok"