# Development

```sh
cargo build --release
cargo clippy --release -- -D warnings
cargo fmt --check
cargo clippy --release --features omarchy -- -D warnings
```

## Pre-commit

`.pre-commit-config.yaml` runs `cargo fmt --check` and clippy with `-D warnings` for the host, the Windows target, and the `omarchy` feature on every commit that touches Rust files:

```sh
rustup target add x86_64-pc-windows-gnu
pre-commit install
pre-commit run --all-files
```

## Dry run without hardware

Works on any OS, macOS included:

```sh
printf 'poll_ms = 100\nrecover_secs = 2\nsimulate = "pins.txt"\n' > dev.toml
echo "8 8 8 8 8 8" > pins.txt
cargo run -- run --config dev.toml &
echo "8 8 8 8 8 10" > pins.txt
```

On macOS, shutdown is logged and not executed.

## Cross-compiling from macOS

```sh
cargo install cargo-zigbuild
rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-gnu
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.17
cargo zigbuild --release --target x86_64-pc-windows-gnu
```

When both Homebrew and rustup Rust are installed, put rustup's toolchain first in `PATH`:

```sh
export PATH="$(dirname "$(rustup which rustc)"):$PATH"
```

## Windows resources

`build.rs` turns `assets/icon.ico` into a `.res` file that is linked into the `.exe`, with no `rc.exe` needed. The `.res` also includes version info (shown in Task Manager as "GPU Warden") and an application manifest (Per-Monitor-V2 DPI, Common Controls v6, `asInvoker`).

`assets/icon.ico` holds PNG frames from 16 to 256 px, rendered from the same geometry as `assets/icon.svg`. The status variants `icon-warn.svg` and `icon-crit.svg` only swap the colours.

`.cargo/config.toml` links the MSVC C runtime statically, so the release `.exe` doesn't need the Visual C++ Redistributable (`VCRUNTIME140.dll`).

## Layout

The crate is a library, `gpu_warden`, plus the `gpu-warden` app built on it. The library is also built as `gpu_warden.dll` and `libgpu_warden.so` with a C interface; see [SDK](sdk.md).

### Library

| File | Role |
|---|---|
| `src/lib.rs` | Library root |
| `src/ffi.rs` | C interface, matching `include/gpu_warden.h` |
| `src/sensor.rs` | `Reading` and the `Sensor` trait |
| `src/hardware.rs` | Supported IDs, IT8915 decode, `Bus` trait |
| `src/wireview.rs` | WireView Pro II serial protocol |
| `src/device/` | Finding and opening a sensor: sysfs and i2c-dev on Linux, NVAPI and HWiNFO on Windows, WireView on both |
| `src/guard.rs` | State machine (pure, no I/O) |
| `src/warden.rs` | Glues sensor, guard, power cap, and notifications |
| `src/power.rs` | NVML power limit (loaded at runtime), and the Windows power helper that sets it for unelevated instances |
| `src/config.rs` | Settings and their paths |
| `src/display.rs` | Number and name formatting for everything shown to users |
| `src/win/` | Windows string, registry, and named pipe helpers |

### App

| File | Role |
|---|---|
| `src/main.rs` | Command line |
| `src/platform/windows/` | Tray, notifications, shutdown |
| `src/platform/windows/install.rs` | Installer and portable install, logon task, power helper task, full uninstall |
| `src/platform/windows/setup.rs` | Setup dialog: install folder and who it runs for |
| `src/platform/windows/registry.rs` | Apps-list entry, autostart flag, install folder and scope, and the traces uninstall clears |
| `src/platform/linux/` | Notifications, init install, poweroff |
| `src/update.rs` | Self-update for Windows and Omarchy builds (`cfg(updater)`, set by `build.rs`) |
| `src/sha256.rs` | SHA-256 for checking downloaded releases |
| `src/omarchy.rs` | `omarchy` feature: status file, widget install, `omarchy watch` |
| `include/` | C header for the shared library |
| `omarchy/` | Omarchy shell bar widget, embedded at build time |
| `init/` | Service templates, embedded at build time |
| `build.rs` | Windows icon, version info, and manifest resources |
| `assets/` | Icon (SVG per level, multi-size ICO) |

## Releases

Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds:

- `gpu-warden-<tag>-windows-x86_64-setup.exe` (installer build, MSVC)
- `gpu-warden-<tag>-windows-x86_64-portable.zip` (`--features portable`)
- `gpu-warden-<tag>-linux-x86_64-{systemd,openrc,runit}.tar.gz` (glibc 2.17+)
- `gpu-warden-<tag>-linux-x86_64-omarchy.tar.gz` (built with `--features omarchy`, systemd)
- `gpu-warden-<tag>-{windows,linux}-x86_64-sdk` (shared library and C header)
- `SHA256SUMS`
- `gpu-warden.json` (Scoop manifest)

If the AUR secrets are set, it also updates the AUR package. See [packaging](packaging.md).

Linux is not built against musl: a static musl binary can't `dlopen` the glibc-built NVML library, which would silently disable the power cap.

Scoop and AUR packaging are covered in [packaging](packaging.md).
