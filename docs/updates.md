# Updates

Windows and Omarchy installs update themselves. It is on by default. To opt out, set this in the config:

```toml
auto_update = false
```

Package-managed installs are left to their package manager:

| Install | Updated by |
|---|---|
| Windows installer (`…-setup.exe`) | itself |
| Windows portable zip | itself, in place |
| Omarchy archive (`/usr/local/bin/gpu-warden`) | itself |
| Scoop | `scoop update gpu-warden` |
| AUR (`/usr/bin/gpu-warden`) | pacman / your AUR helper |
| other Linux archives | not updated; download the new release |

Turn it on or off from the Windows tray (**Auto update**), or with `gpu-warden auto-update <on|off>` (`sudo` on Linux).

## Check now

**Check for updates** in the Windows tray, or **Updates** in the Omarchy panel (`gpu-warden omarchy update`), checks right away, even with `auto_update = false`. A toast answers: up to date, update found (it then installs as below), or why the check failed. On Scoop it says to run `scoop update gpu-warden`.

## What happens

1. 30 s after the service starts, it checks the latest GitHub release once. There is no repeating check; the next one happens at the next start.
2. If the release is newer, it downloads the file for this build (`…-setup.exe`, `…-portable.zip`, or `…-omarchy.tar.gz`) and `SHA256SUMS` over HTTPS (with `curl`, built into Windows 10+ and Omarchy).
3. It verifies the SHA-256, extracts the binary if needed, and runs it with `--version` to confirm it starts and reports the expected version. Any failure leaves the running version untouched and is logged.
4. It waits until every pin reads OK and no power cap is engaged. During a warning, recovery, sensor loss, or shutdown it never swaps.
5. It moves the running binary aside as `gpu-warden.old`, puts the new one in its place, and restarts into it: in place (`exec`) on Linux, a new process on Windows. Monitoring pauses for well under a second.
6. The new version shows a notification: **GPU Warden updated · Now running X (was Y).** On Omarchy it also refreshes the bar widget files for each user who has it.

If the new version fails to start, it puts the previous binary back, restarts it, and records the failed version in `gpu-warden.skip` so it isn't tried again. A later release is tried as usual.

## Standard Windows users

A standard (non-administrator) user cannot write to `C:\Program Files\GPUWarden`. When such a user's instance has an update ready, it asks the power helper (`GPUWardenPower`, running as SYSTEM) to install it:

1. The instance sends only the release tag, such as `v1.2.3`, over `\\.\pipe\gpu-warden-power`. The helper accepts it only if it is a well-formed tag newer than its own version.
2. The helper downloads and verifies that release itself, the same way as steps 2 and 3 above. It never takes a file from the user.
3. The helper swaps the binary, restarts itself into the new version, and the user's instance restarts into it too.

A rollback after a failed start also goes through the helper. The portable build and Scoop have no helper, so a standard user's portable copy only updates if that user can write to its folder.

Downloads go to `/var/lib/gpu-warden/update` on Linux and `%TEMP%\gpu-warden-update` on Windows, and are removed after the swap.
