# Updates

Windows and Omarchy installs update themselves. It is on by default. To opt out, set this in the config:

```toml
auto_update = false
```

Package-managed installs are left to their package manager:

| Install | Updated by |
|---|---|
| Windows zip, or `install` to Program Files | itself |
| Omarchy archive (`/usr/local/bin/astral-warden`) | itself |
| Scoop | `scoop update astral-warden` |
| AUR (`/usr/bin/astral-warden`) | pacman / your AUR helper |
| other Linux archives | not updated; download the new release |

## What happens

1. 30 s after the service starts, it checks the latest GitHub release once. There is no repeating check; the next one happens at the next start.
2. If the release is newer, it downloads the archive for this build and `SHA256SUMS` over HTTPS (with `curl`, built into Windows 10+ and Omarchy).
3. It verifies the archive's SHA-256, extracts the binary, and runs it with `--version` to confirm it starts and reports the expected version. Any failure leaves the running version untouched and is logged.
4. It waits until every pin reads OK and no power cap is engaged. During a warning, recovery, sensor loss, or shutdown it never swaps.
5. It moves the running binary aside as `astral-warden.old`, puts the new one in its place, and restarts into it: in place (`exec`) on Linux, a new process on Windows. Monitoring pauses for well under a second.
6. The new version shows a notification: **Astral Warden updated · Now running X (was Y).** On Omarchy it also refreshes the bar widget files for each user who has it.

If the new version fails to start, it puts the previous binary back, restarts it, and records the failed version in `astral-warden.skip` so it isn't tried again. A later release is tried as usual.

Downloads go to `/var/lib/astral-warden/update` on Linux and `%TEMP%\astral-warden-update` on Windows, and are removed after the swap.
