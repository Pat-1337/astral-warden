<img src="assets/icon.svg" width="96" alt="">

# astral-warden

Per-pin 12V-2x6 watchdog for ASUS ROG Astral RTX 5090 / 5080.

Warn → cap power to 80% → shut down.

## Install

**Windows 11**: unzip a [release](../../releases), run `astral-warden.exe install`.

**Linux** (systemd, OpenRC, runit): `sudo ./astral-warden install`

## Use

```
astral-warden status
```

| | Config |
|---|---|
| Windows | `C:\ProgramData\astral-warden\config.toml` |
| Linux | `/etc/astral-warden/config.toml` |

More in [docs](docs/).
