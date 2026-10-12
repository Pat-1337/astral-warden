# Configuration

TOML. A missing file means all defaults. An unknown key or invalid value refuses to start, so a typo can't silently disable protection.

```toml
poll_ms = 500
imbalance_min_total_amps = 20.0
warn_samples = 6
crit_samples = 4
recover_secs = 10
sensor_fail_samples = 6
shutdown = true

[pin_amps]
warn = 9.5
crit = 13.0

[imbalance]
warn = 0.3
crit = 0.6

[power_cap]
enabled = true
percent = 80
restore_after_secs = 120
```

| Key | Default | Meaning |
|---|---|---|
| `poll_ms` | 500 | Sample interval (min 50). |
| `pin_amps.warn` / `.crit` | 9.5 / 13.0 | Per-pin current in amps. 9.5 A is the contact rating; 600 W balanced is about 8.3 A per pin. |
| `imbalance.warn` / `.crit` | 0.3 / 0.6 | The largest deviation of one pin from the mean, as a fraction: 0.3 means ±30%. Healthy cards stay under about 0.16. A dead pin reads -100%. |
| `imbalance_min_total_amps` | 20 | Imbalance is only judged above this total current, because idle readings are noisy. |
| `warn_samples` | 6 | Consecutive samples over a warn threshold before a warning, and consecutive clean samples before it counts as recovered. |
| `crit_samples` | 4 | Consecutive samples over a crit threshold before shutdown. |
| `recover_secs` | 10 | How long a warning may last before shutdown. |
| `sensor_fail_samples` | 6 | Consecutive failed reads before "sensor lost". |
| `shutdown` | true | `false` keeps notifying and capping but never powers off. |
| `power_cap.enabled` | true | Lower the GPU power limit on a warning or sensor loss. |
| `power_cap.percent` | 80 | Cap as a percentage of the board's default limit (50–100), clamped to what the board allows. |
| `power_cap.restore_after_secs` | 120 | Clean time needed before the previous limit is restored. |
| `simulate` | unset | A path to a text file of 6 amp values. It replaces the hardware sensor, for dry runs. |

Restart the service, or Exit and relaunch from the tray, after editing.

## MSI Afterburner and other tuners

astral-warden only touches the **power limit**, and only while a warning is active.

- Clocks, voltage curves, undervolts, and fan curves are never changed.
- Before capping, it saves the limit that is currently set (Afterburner's or yours) and puts it back after `restore_after_secs` of clean readings.
- It only ever lowers the limit. A limit already below the cap is left alone.
- While capped, the limit is checked every sample. If Afterburner (or anything else) raises it again, the cap is re-applied.
- Afterburner's power-limit slider is a percentage of the same default limit, so `percent = 80` matches Afterburner's 80%.
