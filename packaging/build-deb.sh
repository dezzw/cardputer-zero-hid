#!/usr/bin/env bash
# Build an arm64 .deb following CardputerZero / Jiggler packaging layout.
# SPDX-License-Identifier: MIT
# Packaging conventions adapted from intGus/jiggler (MIT), commit a59e756.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')"
DISPLAY_NAME="Cardputer Zero HID"
DEB_REVISION="${DEB_REVISION:-m5stack1}"
DEB_ARCH="${DEB_ARCH:-arm64}"
PKG_FILE="${DISPLAY_NAME}_${VERSION}_${DEB_REVISION}_${DEB_ARCH}.deb"
BINARY="${1:-target/aarch64-unknown-linux-gnu/release/cardputer-zero-hid}"
GADGET_BIN="${GADGET_BIN:-${BINARY%/*}/cardputer-zero-hid-gadget}"

if [[ ! -f "$BINARY" ]]; then
  echo "error: release binary not found: $BINARY" >&2
  exit 1
fi
if [[ ! -f "$GADGET_BIN" ]]; then
  echo "error: gadget daemon binary not found: $GADGET_BIN" >&2
  exit 1
fi

if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
  SOURCE_DATE_EPOCH="$(git -C "$ROOT" log -1 --format=%ct 2>/dev/null || true)"
fi
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
  echo "error: SOURCE_DATE_EPOCH unset and git commit time unavailable" >&2
  exit 1
fi
export SOURCE_DATE_EPOCH
export TZ=UTC

GLIBC_MAIN="$("$ROOT/packaging/glibc-required-version.sh" "$BINARY")"
GLIBC_GADGET="$("$ROOT/packaging/glibc-required-version.sh" "$GADGET_BIN")"
GLIBC_REQUIRED="$(printf '%s\n' "$GLIBC_MAIN" "$GLIBC_GADGET" | sort -t. -k1,1n -k2,2n -V | tail -1)"

STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING"' EXIT

PKG_ROOT="${STAGING}/pkg"
mkdir -p "${PKG_ROOT}/usr/bin"
install -m 0755 "$BINARY" "${PKG_ROOT}/usr/bin/cardputer-zero-hid"
install -m 0755 "$GADGET_BIN" "${PKG_ROOT}/usr/bin/cardputer-zero-hid-gadget"

mkdir -p "${PKG_ROOT}/lib/systemd/system"
install -m 0644 "${ROOT}/packaging/systemd/cardputer-zero-hid-gadget.service" \
  "${PKG_ROOT}/lib/systemd/system/"

mkdir -p "${PKG_ROOT}/etc"
install -m 0644 "${ROOT}/config/cardputer-zero-hid-app.conf" "${PKG_ROOT}/etc/"

mkdir -p "${PKG_ROOT}/usr/share/APPLaunch/applications"
install -m 0644 "${ROOT}/packaging/debian/cardputer-zero-hid.desktop" \
  "${PKG_ROOT}/usr/share/APPLaunch/applications/"

mkdir -p "${PKG_ROOT}/usr/share/cardputer-zero-hid/images"
install -m 0644 "${ROOT}/assets/images/cardputer-zero-hid.png" \
  "${PKG_ROOT}/usr/share/cardputer-zero-hid/images/"

mkdir -p "${PKG_ROOT}/usr/share/APPLaunch/share/images"
install -m 0644 "${ROOT}/assets/images/cardputer-zero-hid.png" \
  "${PKG_ROOT}/usr/share/APPLaunch/share/images/"

mkdir -p "${PKG_ROOT}/usr/share/doc/cardputer-zero-hid"
install -m 0644 "${ROOT}/README.md" "${PKG_ROOT}/usr/share/doc/cardputer-zero-hid/"
install -m 0644 "${ROOT}/LICENSE" "${PKG_ROOT}/usr/share/doc/cardputer-zero-hid/copyright"

DEBIAN="${PKG_ROOT}/DEBIAN"
mkdir -p "$DEBIAN"
install -m 0755 "${ROOT}/packaging/debian/postinst" "$DEBIAN/"
install -m 0755 "${ROOT}/packaging/debian/prerm" "$DEBIAN/"
install -m 0755 "${ROOT}/packaging/debian/postrm" "$DEBIAN/"
printf '%s\n' '/etc/cardputer-zero-hid-app.conf' >"${DEBIAN}/conffiles"

# Reproducible mtimes for every packaged file.
find "$PKG_ROOT" -depth -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} +

INSTALLED_KB="$(du -sk "${PKG_ROOT}" | cut -f1)"
cat >"${DEBIAN}/control" <<EOF
Package: cardputer-zero-hid
Version: ${VERSION}-${DEB_REVISION}
Architecture: ${DEB_ARCH}
Maintainer: dezzw <dezzw@users.noreply.github.com>
Section: utils
Priority: optional
Installed-Size: ${INSTALLED_KB}
Depends: libc6 (>= ${GLIBC_REQUIRED})
Description: ${DISPLAY_NAME}
 USB HID keyboard+mouse composite gadget for M5Stack Cardputer Zero (M1 gadget service).
EOF
touch -d "@${SOURCE_DATE_EPOCH}" "${DEBIAN}/control" "${DEBIAN}/conffiles"

mkdir -p "${ROOT}/dist"
OUTPUT="${ROOT}/dist/${PKG_FILE}"
rm -f "$OUTPUT"
dpkg-deb --root-owner-group --build --uniform-compression -Zgzip -z9 "${PKG_ROOT}" "$OUTPUT"
echo "$OUTPUT"
echo "libc6 Depends: libc6 (>= ${GLIBC_REQUIRED})"