<p align="center"><img src="assets/icon.svg" width="96" alt=""></p>

<h1 align="center">GPU Warden</h1>

<p align="center">Per-pin 12V-2x6 watchdog for Windows 11 and Linux</p>

<p align="center">
  <a href="https://github.com/patvuk/gpu-warden/actions/workflows/ci.yml"><img src="https://github.com/patvuk/gpu-warden/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <a href="https://github.com/patvuk/gpu-warden/releases/latest"><img src="https://img.shields.io/github/v/release/patvuk/gpu-warden" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/patvuk/gpu-warden" alt="License"></a>
</p>

## Hardware

- ASUS ROG Astral RTX 5090 and 5080
- ASUS ROG Matrix RTX 5090
- Thermal Grizzly WireView Pro II

The full list of boards is in [hardware](docs/hardware.md).

## Levels

| | Level | Trigger (default) | Action |
|:-:|---|---|---|
| <img src="assets/icon.svg" width="32" alt="green"> | OK | every pin under 9.5 A and balance within 30% | check every half second |
| <img src="assets/icon-warn.svg" width="32" alt="amber"> | Warning | a pin at 9.5 A or more, or 30% off balance, for 3 s | notify, cap power to 80%, shut down if not recovered in 10 s |
| <img src="assets/icon-crit.svg" width="32" alt="red"> | Critical | a pin at 13 A or more, or 60% off balance, for 2 s | shut down |

## Install

### Windows 11

Download the setup exe from [Releases](../../releases) and run it. It installs, starts at every logon, updates itself, and uninstalls from Settings under Apps.

To keep everything in one folder instead, use the portable zip, or Scoop:

```
scoop install https://github.com/patvuk/gpu-warden/releases/latest/download/gpu-warden.json
```

### Linux

Works with systemd, OpenRC and runit. From the extracted release folder:

```
sudo ./gpu-warden install
```

### NixOS

The flake has a package and a NixOS module. See [Packaging](docs/packaging.md#nix).

```nix
services.gpu-warden.enable = true;
```

### Omarchy

Use the omarchy archive, which also adds a bar widget. See [Omarchy](docs/omarchy.md).

## Use

One reading:

```
gpu-warden status
```

Live readings:

```
gpu-warden status --watch
```

| | Config |
|---|---|
| Windows | `C:\ProgramData\gpu-warden\config.toml` |
| Windows portable | `gpu-warden.toml` next to the exe |
| Linux | `/etc/gpu-warden/config.toml` |

Windows and Omarchy installs update themselves, see [updates](docs/updates.md). AUR and Scoop installs update through their package manager.

## SDK

The core also ships as `gpu_warden.dll` and `libgpu_warden.so` with a C header, for reading the pins from your own programs. See [SDK](docs/sdk.md).

More in [docs](docs/).
