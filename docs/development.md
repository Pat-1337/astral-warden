# Development

```sh
cargo build --release
cargo clippy --release -- -D warnings
cargo fmt --check
cargo clippy --release --features omarchy -- -D warnings
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

`build.rs` turns `assets/icon.ico` into a `.res` file that is linked into the `.exe`, with no `rc.exe` needed. The `.res` also includes version info (shown in Task Manager as "Astral Warden") and an application manifest (Per-Monitor-V2 DPI, Common Controls v6, `asInvoker`).

`assets/icon.ico` holds PNG frames from 16 to 256 px, rendered from the same geometry as `assets/icon.svg`. The status variants `icon-warn.svg` and `icon-crit.svg` only swap the colours.

`.cargo/config.toml` links the MSVC C runtime statically, so the release `.exe` doesn't need the Visual C++ Redistributable (`VCRUNTIME140.dll`).

## Layout

| File | Role |
|---|---|
| `src/hardware.rs` | Supported IDs, IT8915 decode, `Bus` trait |
| `src/guard.rs` | State machine (pure, no I/O) |
| `src/warden.rs` | Glues sensor, guard, power cap, and notifications |
| `src/power.rs` | NVML power limit (loaded at runtime) |
| `src/display.rs` | Number and name formatting for everything shown to users |
| `src/platform/windows/` | NVAPI I2C, tray, task install, shutdown |
| `src/platform/linux.rs` | sysfs detection, i2c-dev, init install, poweroff |
| `src/omarchy.rs` | `omarchy` feature: status file, widget install, `omarchy watch` |
| `omarchy/` | Omarchy shell bar widget, embedded at build time |
| `init/` | Service templates, embedded at build time |
| `build.rs` | Windows icon, version info, and manifest resources |
| `assets/` | Icon (SVG per level, multi-size ICO) |

## Releases

Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds:

- `astral-warden-<tag>-windows-x86_64.zip` (MSVC)
- `astral-warden-<tag>-linux-x86_64-{systemd,openrc,runit}.tar.gz` (glibc 2.17+)
- `astral-warden-<tag>-linux-x86_64-omarchy.tar.gz` (built with `--features omarchy`, systemd)
- `SHA256SUMS`
- `astral-warden.json` (Scoop manifest)

If the AUR secrets are set, it also updates the AUR package. See [packaging](packaging.md).

Linux is not built against musl: a static musl binary can't `dlopen` the glibc-built NVML library, which would silently disable the power cap.

Scoop and AUR packaging are covered in [packaging](packaging.md).
