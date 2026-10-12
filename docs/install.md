# Install

The same binary installs itself, writes a default config if none exists, and starts at boot or logon.

## Windows 11

1. Download `astral-warden-<version>-windows-x86_64.zip` from Releases and unzip it.
2. Run `astral-warden.exe install`. A UAC prompt appears if the terminal is not elevated.

What `install` does:

| Step | Result |
|---|---|
| Copy binary | `C:\Program Files\AstralWarden\astral-warden.exe` |
| Default config | `C:\ProgramData\astral-warden\config.toml` |
| Scheduled task | `AstralWarden`, runs at every logon with the highest privileges, no time limit, restarts on failure |
| Multi-user | with fast user switching, a second session waits (`--wait`) and takes over when the first logs off |
| Start | starts the task right away |

The tray icon shows the state:

| Icon | State |
|---|---|
| green | OK |
| amber | warning, recovering, or sensor unreadable |
| red | shutdown issued |

Hover over the icon for the hottest pin and the board power. Right-click it for Open config, Open log, and Exit.

Administrator rights are needed to change the power limit, and the logon task grants them. Run unelevated, it still monitors and can shut down, but it cannot cap power.

Uninstall: `astral-warden.exe uninstall`. The config is kept.

## Linux

Requirements:
- the NVIDIA driver (proprietary or open kernel modules), which exposes the `NVIDIA i2c adapter` buses
- the `i2c-dev` module, which `install` loads and registers in `/etc/modules-load.d/`
- root, for `/dev/i2c-*`, NVML power limits, and poweroff

```sh
tar xzf astral-warden-<version>-linux-x86_64-<init>.tar.gz
cd astral-warden-*/
sudo ./astral-warden install
```

The binary needs glibc 2.17 or newer, which it uses to load NVML for the power cap. It is the same in every archive. The `-systemd`, `-openrc`, and `-runit` archives differ only in the reference service file in `init/`. `install` detects the init system on its own:

| Init | Detected by | Service file | Logs |
|---|---|---|---|
| systemd | `/run/systemd/system` | `/etc/systemd/system/astral-warden.service` | `journalctl -u astral-warden` |
| OpenRC | `/run/openrc` or `/sbin/openrc-run` | `/etc/init.d/astral-warden` (supervise-daemon) | `/var/log/astral-warden.log` |
| runit | `/run/runit` or `/etc/runit` | `/etc/sv/astral-warden/run`, linked into the runsvdir | runit log or console |

The binary goes to `/usr/local/bin/astral-warden` and the config to `/etc/astral-warden/config.toml`.

With any other init system, run `astral-warden run` as root at boot. The service files in `init/` are templates: `@BIN@` and `@CONFIG@` are the placeholders.

Desktop notifications go through `notify-send` to every logged-in user's session bus. Critical events also go to `wall`.

Uninstall: `sudo astral-warden uninstall`. The config is kept.

## Check it works

```
astral-warden status
```

This prints the detected board, each pin's current and voltage, total power, imbalance, the NVML power limit, and the verdict. On Linux, run it with `sudo`.
