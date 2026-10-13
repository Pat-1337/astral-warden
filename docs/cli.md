# Command line

```
astral-warden [command] [flags]
```

| Command | Does |
|---|---|
| `run` (default) | Monitor. Tray app on Windows, service loop on Linux. |
| `status` | Print one reading and exit. |
| `install` | Install for autostart (logon task on Windows; systemd, OpenRC, or runit service on Linux). |
| `uninstall` | Remove everything: binary (installer build), service or logon task, config, log, state, registry entries. |
| `config` | Print the default config. |
| `omarchy install` | `-omarchy` build only: add the bar widget for the current user. See [Omarchy](omarchy.md). |
| `omarchy uninstall` | `-omarchy` build only: remove the bar widget. |
| `auto-update <on\|off>` | Windows and Omarchy builds: set `auto_update` in the config. Needs root / admin. |
| `omarchy update` | `-omarchy` build only: ask the service to check for updates now; the result shows as a notification. |
| `omarchy watch` | `-omarchy` build only: show the service's readings live, without root. |

| Flag | Does |
|---|---|
| `--config <path>` | Use this config file. It must exist. |
| `--watch` | With `status`: print a live line each poll interval, including the process's memory. |
| `--wait` | With `run`: if another instance holds the lock, wait for it instead of exiting (used by the logon task). |
| `--silent` | With `run`: no startup notification (used by the logon task). |
| `--background` | With `install`/`uninstall`: no dialogs, and the running instance isn't restarted (used by the tray's **Start at logon**). |
| `--secret` | ?? |
| `--version`, `--help` | |
