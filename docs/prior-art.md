# Prior art

The repos below were surveyed on 2026-10-12 to pin down the protocol. Each value in [how-it-works](how-it-works.md) was cross-checked in at least two of them.

| Repo | Lang | What |
|---|---|---|
| [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) | C# | `NvidiaGpu.cs`: Astral per-pin V/A/W via NVAPI I2C, gated by subsystem ID |
| [humza-khalid/12vhpwr-guard](https://github.com/humza-khalid/12vhpwr-guard) | Python | Windows guard: NVAPI I2C plus HWiNFO fallback, NVML clock floor and power cap, then shutdown |
| [ksokolowski/astral-hwmon](https://github.com/ksokolowski/astral-hwmon) | C | Linux hwmon driver and userspace guard; register map and measurements |
| [Timic3/astral-power-monitoring](https://github.com/Timic3/astral-power-monitoring) | Rust | The original NVAPI `I2CReadEx` reader |
| [tgbender/astral-power-monitor](https://github.com/tgbender/astral-power-monitor) | Rust | Windows tray app with history and protection |
| [jan-provaznik/sus](https://github.com/jan-provaznik/sus) | Go | Linux i2c-dev monitor |
| [GhostKellz/nvcontrol](https://github.com/GhostKellz/nvcontrol) | Rust | Linux NVIDIA panel; `asus_power_detector.rs` reads via `I2C_SMBUS` |
| [EthDevOps/astral-power-monitor](https://github.com/EthDevOps/astral-power-monitor) | Python | Linux daemon, byte-wise SMBus reads |
| [senorcris/gpu-power-monitor](https://github.com/senorcris/gpu-power-monitor) | Python | Linux TUI over i2c-dev |
| [Loong0x00/nvidia-tools](https://github.com/Loong0x00/nvidia-tools) | Python | Reverse engineering of the GPU Tweak III ↔ IT8915 path |
| [airtaxi/PinStats](https://github.com/airtaxi/PinStats) | C# | Tray monitor built on LHM |
| [ilya-zlobintsev/LACT](https://github.com/ilya-zlobintsev/LACT) | Rust | Linux GPU tool; Astral support requested in issue #906 |
| [rust-nvml/nvml-wrapper](https://github.com/rust-nvml/nvml-wrapper) | Rust | NVML bindings: power-limit API reference |
| [arcnmx/nvapi-rs](https://github.com/arcnmx/nvapi-rs) | Rust | NVAPI bindings, including private `I2CReadEx` |
| [falahati/NvAPIWrapper](https://github.com/falahati/NvAPIWrapper) | C# | .NET NVAPI wrapper |
| [emaspa/wireview-hwmon](https://github.com/emaspa/wireview-hwmon) | C | Linux hwmon driver and daemon for the WireView Pro II; serial protocol and frame layout |
| [McNugget6750/12VHPWR-Saver](https://github.com/McNugget6750/12VHPWR-Saver) | Python | Connector temperature monitor (external sensor) |
| [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB) (GitLab) | C++ | `pci_ids.h`: the board ID list |

The default thresholds follow 12vhpwr-guard and astral-hwmon's guard design: the 9.5 A contact rating, healthy imbalance under ~16%, and slow-cook failures at 11–15 A.

What gpu-warden adds:
- a single self-contained binary for Windows and Linux
- systemd, OpenRC, and runit support
- coexistence with MSI Afterburner (it saves and restores the user's limit and re-applies the cap if overridden)
