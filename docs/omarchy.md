# Omarchy

The `-omarchy` release is the Linux build with the `omarchy` cargo feature. It adds a bar widget for the Omarchy shell. The other releases don't include any of this code.

## Install

```sh
tar xzf astral-warden-<version>-linux-x86_64-omarchy.tar.gz
cd astral-warden-*/
sudo ./astral-warden install
```

This installs the systemd service as on any Linux system, then adds the bar widget for the user who ran `sudo`. To add or remove only the widget, run these as your desktop user, without `sudo`:

```sh
astral-warden omarchy install
astral-warden omarchy uninstall
```

`install` writes the plugin to `~/.config/omarchy/plugins/pat-1337.astral-warden/` and enables it in the right section of the bar. Move it with `omarchy bar move pat-1337.astral-warden --section <left|center|right>`.

## Widget

| State | Bar |
|---|---|
| OK | bolt and hottest pin current, in the bar colour |
| Warning, recovering, sensor unreadable, shutdown | the same, in the theme's urgent colour |
| Service not running | dimmed bolt |

Hover for every pin, total power, balance, board power, power cap, and shutdown arming. Click to open `astral-warden omarchy watch` in a floating terminal.

Notifications go through `notify-send`, which the Omarchy shell shows.

## How it works

The service runs as root and writes `/run/astral-warden/status.json` at most once a second, and right away when the level changes. It writes to a temporary file and renames it, so readers never see a partial file. The file is removed when the service stops.

The widget reloads that file on a timer (`refreshIntervalSec`, default 1) and treats it as stale after 5 s. Neither the widget nor `astral-warden omarchy watch` needs root.

## Without the hardware

```sh
cargo build --release --features omarchy
printf 'poll_ms = 100\nshutdown = false\nalarm = false\nsimulate = "pins.txt"\n' > dev.toml
echo "8 8 8 8 8 8" > pins.txt
sudo ./target/release/astral-warden run --config dev.toml &
./target/release/astral-warden omarchy install
echo "8 8 8 8 8 10.5" > pins.txt
```
