# cardputer-zero-hid

Turn an [M5Stack Cardputer Zero](https://docs.m5stack.com/en/CardputerZero) into a USB composite **keyboard + mouse** HID gadget for a connected host, with keyboard / mouse / combo modes and an on-device UI.

**M0 (this milestone)** is scaffold-only: the crate builds, packages as a CardputerZero-style `.deb`, and CI cross-compiles for the device — but **no USB HID, configfs, or on-screen UI yet**.

## Device prerequisites (for future HID milestones)

These steps come from the Cardputer Zero USB topology and mirror [intGus/jiggler](https://github.com/intGus/jiggler) (MIT, reference commit [`a59e756`](https://github.com/intGus/jiggler/commit/a59e756)):

1. **USB switch — Slave (device) mode** — slide the switch so the **green LED is off** (device / USB gadget side).
2. **Right-side Type-C** — use a **data-capable** cable on the **right** Type-C port when presenting HID to a host (not charge-only).

On Linux the gadget UDC is expected at **`3f980000.usb`** (needs real-device verification).

Hardware context: **320×170** display, **46-key** matrix keyboard, Raspberry Pi **CM0** running **Debian aarch64**.

## CPU / target triple

| Source | Claim |
| --- | --- |
| [M5Stack Cardputer Zero docs](https://docs.m5stack.com/en/CardputerZero) | Quad-core **Cortex-A53**, **ARMv8-A (aarch64)** on CM0 |
| [Raspberry Pi CM0 documentation](https://www.raspberrypi.com/products/compute-module/) | CM0 family (BCM2837-class, 64-bit capable) |

**Cross target:** `aarch64-unknown-linux-gnu` — the device ships **Debian arm64 with glibc** (same class as [jiggler’s `cp0-aarch64-linux-gnu` toolchain](https://github.com/intGus/jiggler/blob/a59e756/cmake/toolchains/cp0-aarch64-linux-gnu.cmake)). We use the GNU ABI to match distro `libc6` and `.deb` dependencies; **musl** would produce a mostly static binary but is a poor default here because the platform is glibc-first and future UI stacks will likely link shared system libraries.

## Build (host)

Requirements: Rust **stable** (see `rust-toolchain.toml`), `rustfmt`, `clippy`.

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

### Cross-compile release binary (device)

The Cardputer Zero OS image is **Debian trixie (glibc 2.41)**. CI builds the release binary inside a **`debian:trixie`** container with `gcc-aarch64-linux-gnu` so linked symbols stay **≤ GLIBC 2.41** (Ubuntu “latest” runners can link newer glibc and break on-device).

```bash
# Match CI (recommended for release artifacts)
docker run --rm -v "$PWD:/src" -w /src debian:trixie bash -lc '
  apt-get update && apt-get install -y curl gcc-aarch64-linux-gnu
  curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  . "$HOME/.cargo/env"
  rustup target add aarch64-unknown-linux-gnu
  export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
  cargo build --release --target aarch64-unknown-linux-gnu
'
```

Host-only cross-build (dev; verify glibc with `packaging/verify-glibc-max.sh`):

```bash
sudo apt-get install gcc-aarch64-linux-gnu binutils-aarch64-linux-gnu
rustup target add aarch64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
packaging/verify-glibc-max.sh target/aarch64-unknown-linux-gnu/release/cardputer-zero-hid 2.41
```

Release profile is tuned for size (`opt-level = "z"`, `lto`, `strip`, `panic = "abort"`).

### Build `.deb` (arm64 package layout)

After a successful cross-build:

```bash
export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
packaging/build-deb.sh target/aarch64-unknown-linux-gnu/release/cardputer-zero-hid
```

`Depends: libc6` is taken from the binary’s highest `GLIBC_*` symbol (see `packaging/glibc-required-version.sh`).

Output: `dist/Cardputer Zero HID_<version>_m5stack1_arm64.deb`.

Install layout follows **jiggler** conventions:

- `/usr/bin/cardputer-zero-hid`
- `/usr/share/APPLaunch/applications/*.desktop`
- `/etc/cardputer-zero-hid-app.conf` (conffile; placeholder in M0)
- `app-builder.json` store metadata (not published in M0)

M0 **`postinst`** only reloads systemd (no HID boot service yet). Jiggler’s `postinst` enables `jiggler-hid-gadget.service` — we will adopt that pattern when HID lands.

## Install on device

```bash
sudo dpkg -i "dist/Cardputer Zero HID_"*"_arm64.deb"
```

Launch from APPLaunch when UI exists; M0 binary prints a scaffold banner to stderr and exits successfully.

## Project layout

| Module | Role (planned) |
| --- | --- |
| `gadget` | configfs / libcomposite setup |
| `hid` | HID report endpoints |
| `input` | Matrix keyboard |
| `ui` | 320×170 on-device UI |
| `app` | Main loop / mode state |

## Credits

- **[intGus/jiggler](https://github.com/intGus/jiggler)** (MIT) — `app-builder.json`, Debian/APPLaunch layout, and `postinst` patterns adapted from commit [`a59e756`](https://github.com/intGus/jiggler/commit/a59e756).

## License

MIT — see [LICENSE](LICENSE).