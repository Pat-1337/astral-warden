# Install

The same binary installs itself, writes a default config if none exists, and starts at boot or logon.

## Windows 11

Two builds, same program:

| Build | File | For |
|---|---|---|
| Installer | `astral-warden-<version>-windows-x86_64-setup.exe` | most people: installs, starts at logon, updates itself, uninstalls from Settings |
| Portable | `astral-warden-<version>-windows-x86_64-portable.zip` | running from a folder of your choice, nothing installed |

### Installer

Double-click `astral-warden-<version>-windows-x86_64-setup.exe` and accept the UAC prompt. It:

| Step | Result |
|---|---|
| Copy binary | `C:\Program Files\AstralWarden\astral-warden.exe`; the downloaded file can be deleted |
| Default config | `C:\ProgramData\astral-warden\config.toml` |
| Scheduled task | `AstralWarden`, runs at every logon with the highest privileges, no time limit, restarts on failure |
| Apps list | **Astral Warden** in Settings → Apps → Installed apps |
| Start | starts right away |

With fast user switching, a second session waits (`--wait`) and takes over when the first logs off.

Uninstall from **Settings → Apps → Installed apps → Astral Warden → Uninstall**, or run `astral-warden uninstall`. It stops the running instance and removes everything: the program folder, config, log, state files, the logon task, registry entries, and update downloads.

### Portable

Unzip `astral-warden-<version>-windows-x86_64-portable.zip` anywhere and run `astral-warden.exe`. Everything it writes stays next to it:

| File | Holds |
|---|---|
| `astral-warden.toml` | config |
| `astral-warden.log` | log |
| `astral-warden.power-cap`, `astral-warden.last-shutdown` | power cap state, last protective shutdown |

Tick **Start at logon** in the tray menu to start this copy at every logon (UAC prompt). `astral-warden uninstall` removes the logon task, registry entries, and the files above; then delete the folder. Scoop installs this build.

### Tray

The tray icon shows the state:

| Icon | State |
|---|---|
| green | OK |
| amber | warning, recovering, or sensor unreadable |
| red | shutdown issued |

Hover over the icon for the hottest pin and the board power. Right-click it for:

| Section | Shows |
|---|---|
| Header | version and detected board, with the sensor source |
| Readings | state, current and voltage for each pin, total current and power, balance, board power against its limit, power cap state, shutdown arming, memory use (working set and private), uptime |
| Start at logon | checkbox that registers or removes the logon task, with a UAC prompt if not elevated; monitoring continues meanwhile |
| Auto update | checkbox for `auto_update` in the config, with a UAC prompt if not elevated; greyed out for Scoop |
| Check for updates | checks now; a toast says whether an update was found, and a found update installs on its own |
| Actions | Open config, Open log, Exit |

Optional fallback: if the direct sensor read fails, readings come from [HWiNFO](https://www.hwinfo.com/) when it is running with **Shared Memory Support** enabled. `status` shows which source is in use (`via NVAPI, HWiNFO as fallback` or `via HWiNFO`).

With a WireView Pro II, close Thermal Grizzly's own WireView app first. Only one program can hold its COM port.

Administrator rights are needed to change the power limit, and the logon task grants them. Run unelevated, it still monitors and can shut down, but it cannot cap power.

## Linux

Requirements:
- the NVIDIA driver (proprietary or open kernel modules), which exposes the `NVIDIA i2c adapter` buses. A WireView Pro II works without it.
- the `i2c-dev` module, which `install` loads and registers in `/etc/modules-load.d/`
- for a WireView Pro II, a udev rule that keeps ModemManager off its serial port, which `install` writes to `/etc/udev/rules.d/`
- root, for `/dev/i2c-*`, `/dev/ttyACM*`, NVML power limits, and poweroff

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

Uninstall: `sudo astral-warden uninstall`. It removes the binary, service, module and udev files, config (`/etc/astral-warden`), state (`/var/lib/astral-warden`, `/run/astral-warden`), update leftovers, and the Omarchy bar widget.

When it starts from a double-click, a toast confirms it is running and that the card's pins are readable. Starts at logon are silent.

After a protective shutdown, the next start shows when and why it happened.

## Check it works

```
astral-warden status
```

This prints the detected board, each pin's current and voltage, total power, imbalance, the NVML power limit, and the verdict. On Linux, run it with `sudo`.

`astral-warden status --watch` prints a live line each poll interval: total current and power, the hottest pin, NVML board power, and the process's own memory. Run it without the tray, it shows whether memory stays flat.

## Log

Nothing is written to disk until the first warning, critical event, or error. From then on, every event for that session goes to:

| OS | Log |
|---|---|
| Windows installer | `C:\ProgramData\astral-warden\astral-warden.log` (rotated at 1 MB) |
| Windows portable | `astral-warden.log` next to the exe (rotated at 1 MB) |
| Linux | the service log: journald, `/var/log/astral-warden.log` on OpenRC, or runit's logger |

The config is read once at start. Restart after editing it.
