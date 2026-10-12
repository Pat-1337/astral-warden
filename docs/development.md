# Development

```sh
cargo build --release
cargo clippy --release -- -D warnings
cargo fmt --check
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
rustup target add x86_64-unknown-linux-musl x86_64-pc-windows-gnu
cargo zigbuild --release --target x86_64-unknown-linux-musl
cargo zigbuild --release --target x86_64-pc-windows-gnu
```

When both Homebrew and rustup Rust are installed, put rustup's toolchain first in `PATH`:

```sh
export PATH="$(dirname "$(rustup which rustc)"):$PATH"
```

## Layout

| File | Role |
|---|---|
| `src/hardware.rs` | Supported IDs, IT8915 decode, `Bus` trait |
| `src/guard.rs` | State machine (pure, no I/O) |
| `src/warden.rs` | Glues sensor, guard, power cap, and notifications |
| `src/power.rs` | NVML power limit (loaded at runtime) |
| `src/platform/windows/` | NVAPI I2C, tray, task install, shutdown |
| `src/platform/linux.rs` | sysfs detection, i2c-dev, init install, poweroff |
| `init/` | Service templates, embedded at build time |

## Releases

Pushing a `v*` tag runs `.github/workflows/release.yml`, which builds:

- `astral-warden-<tag>-windows-x86_64.zip` (MSVC)
- `astral-warden-<tag>-linux-x86_64-{systemd,openrc,runit}.tar.gz` (static musl)
- `SHA256SUMS`

AUR and Homebrew-tap packaging can use the musl tarball as-is.
