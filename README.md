<p align="center"><img src="assets/icon.svg" width="96" alt=""></p>

<h1 align="center">Astral Warden</h1>

<p align="center">Per-pin 12V-2x6 watchdog for ASUS ROG Astral RTX 5090 / 5080<br>Windows 11 · Linux</p>

## Levels

| | Level | Trigger (default) | Action |
|:-:|---|---|---|
| <img src="assets/icon.svg" width="32" alt="green"> | OK | every pin under 9.5 A, balance within 30% | check every 0.5 s |
| <img src="assets/icon-warn.svg" width="32" alt="amber"> | Warning | a pin at 9.5 A+ or 30%+ off balance for 3 s | notify, cap power to 80%, shut down if not recovered in 10 s |
| <img src="assets/icon-crit.svg" width="32" alt="red"> | Critical | a pin at 13 A+ or 60%+ off balance for 2 s | shut down |

## Install

**Windows 11**: download the zip from [Releases](../../releases), run `astral-warden.exe`, then tick **Start at logon** in the tray menu. Or with Scoop:

```
scoop install https://github.com/Pat-1337/astral-warden/releases/latest/download/astral-warden.json
```

**Linux** (systemd, OpenRC, runit), from the extracted release folder:

```
sudo ./astral-warden install
```

## Use

One reading:

```
astral-warden status
```

Live readings:

```
astral-warden status --watch
```

| | Config |
|---|---|
| Windows | `C:\ProgramData\astral-warden\config.toml` |
| Linux | `/etc/astral-warden/config.toml` |

More in [docs](docs/).
