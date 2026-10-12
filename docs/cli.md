# Command line

```
astral-warden [command] [flags]
```

| Command | Does |
|---|---|
| `run` (default) | Monitor. Tray app on Windows, service loop on Linux. |
| `status` | Print one reading and exit. |
| `install` | Install for autostart (logon task on Windows; systemd, OpenRC, or runit service on Linux). |
| `uninstall` | Remove the autostart entry. The config is kept. |
| `config` | Print the default config. |

| Flag | Does |
|---|---|
| `--config <path>` | Use this config file. It must exist. |
| `--watch` | With `status`: print a live line each poll interval, including the process's memory. |
| `--wait` | With `run`: if another instance holds the lock, wait for it instead of exiting (used by the logon task). |
| `--silent` | With `run`: no startup notification (used by the logon task). |
| `--background` | With `install`/`uninstall`: no dialogs, and the running instance isn't restarted (used by the tray's **Start at logon**). |
| `--secret` | ?? |
| `--version`, `--help` | |
