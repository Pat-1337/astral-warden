# Omarchy

The `-omarchy` release is the Linux build with the `omarchy` cargo feature. It adds a bar widget for the Omarchy shell. The other releases don't include any of this code.

## Install

```sh
tar xzf gpu-warden-<version>-linux-x86_64-omarchy.tar.gz
cd gpu-warden-*/
sudo ./gpu-warden install
```

This installs the systemd service as on any Linux system, then adds the bar widget for the user who ran `sudo`. To add or remove only the widget, run these as your desktop user, without `sudo`:

```sh
gpu-warden omarchy install
gpu-warden omarchy uninstall
```

`install` writes the plugin to `~/.config/omarchy/plugins/pat-1337.gpu-warden/` and enables it in the right section of the bar. Move it with `omarchy bar move pat-1337.gpu-warden --section <left|center|right>`.

## Widget

| State | Bar |
|---|---|
| OK | bolt and hottest pin current, in the bar colour |
| Warning, recovering, sensor unreadable, shutdown | the same, in the theme's urgent colour |
| Service not running | dimmed bolt |

Click the icon, left or right, to open the panel. It works like the other Omarchy panels: Esc closes it and Tab moves to the next panel.

| Part | What it shows or does |
|---|---|
| Header | board, level, hottest pin current |
| Readings | every pin, total power, balance, board power, power cap, shutdown arming |
| Auto update | turns `auto_update` on or off (asks for your password) |
| Start at boot | enables or disables the service (asks for your password) |
| Live view | `gpu-warden omarchy watch` in a floating terminal |
| Updates | checks for updates now; a notification says what it found |
| Restart | restarts the service (asks for your password) |
| Config | opens `/etc/gpu-warden/config.toml` with `sudoedit` |
| Log | follows the service log with `journalctl` |
| About | opens the project page |

Notifications go through `notify-send`, which the Omarchy shell shows.

## Updates

The Omarchy build updates itself and refreshes the bar widget for every user who has it. See [updates](updates.md).

## How it works

The service runs as root and writes `/run/gpu-warden/status.json` at most once a second, and right away when the level changes. It writes to a temporary file and renames it, so readers never see a partial file. The file is removed when the service stops.

The widget reloads that file on a timer (`refreshIntervalSec`, default 1) and treats it as stale after 5 s. Neither the widget nor `gpu-warden omarchy watch` needs root.

## Without the hardware

```sh
cargo build --release --features omarchy
printf 'poll_ms = 100\nshutdown = false\nalarm = false\nsimulate = "pins.txt"\n' > dev.toml
echo "8 8 8 8 8 8" > pins.txt
sudo ./target/release/gpu-warden run --config dev.toml &
./target/release/gpu-warden omarchy install
echo "8 8 8 8 8 10.5" > pins.txt
```
