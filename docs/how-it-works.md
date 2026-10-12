# How it works

## Sensor

Astral boards have an **ITE IT8915** ("Power Detector+") on the GPU's own I2C bus. It measures the voltage and current of each of the six 12 V pins.

| | |
|---|---|
| I2C address | `0x2B` (7-bit) |
| Chip ID | register `0x21` = `0x89` (checked, but only a mismatch warning) |
| Pin block | register `0x80`, 24 bytes |
| Layout | 6 × `[u16 BE millivolts, u16 BE milliamps]`, **reverse pin order**: pin 1 is at bytes 20–23, pin 6 at bytes 0–3 |

A frame is rejected unless every pin has 6–16 V and 0–30 A. That catches all-zero and `0xFFFF` garbage reads. A failed read is retried twice, 20 ms apart.

| OS | Access path |
|---|---|
| Windows | `nvapi64.dll` → `NvAPI_I2CReadEx` (`NV_I2C_INFO_V3`, port 1, 100 kHz, not DDC) |
| Linux | `/dev/i2c-N` for the adapter named `NVIDIA i2c adapter 1` under the GPU's PCI device. It uses the `I2C_SMBUS` ioctl with `I2C_SMBUS_I2C_BLOCK_DATA`, because a plain repeated-start read returns zeros on this adapter. |

Nothing is ever written to the chip.

There is one more sanity check: if the pins sum to under 1 A while NVML reports more than 150 W of board power, the sample counts as a failed read. That happens on BTF boards powered through the motherboard slot, where current bypasses the sensor.

## Detection

Before any I2C traffic, the GPU's PCI vendor, device, and subsystem IDs must match the [supported list](hardware.md). Address `0x2B` on other boards may be a different chip.

## Decision

Each sample is graded:

- **pin**: the hottest pin against `pin_amps`
- **imbalance**: the largest |pin − mean| / mean against `imbalance`, judged only when the total is at least `imbalance_min_total_amps`

```
            warn × warn_samples                 clean × warn_samples
  Normal ─────────────────────────▶ Warning ─────────────────────────▶ Recovering
    ▲                               │  cap power                       │
    │                               │  recover_secs elapsed            │ clean for restore_after_secs
    │                               ▼                                  ▼
    │                            SHUTDOWN ◀── warn again ──────────── restore limit ─▶ Normal
    │                               ▲
    └── any state: crit × crit_samples ─┘

  sensor fails × sensor_fail_samples: Normal/Recovering → Blind (cap power, notify)
  first good sample:                  Blind → Recovering
```

- If a warning re-triggers while still recovering, the cap was not enough, so the PC shuts down.
- During a warning, failed reads don't stop the shutdown timer.
- With `shutdown = false`, every SHUTDOWN becomes a sustained warning instead.

## Actions

| Action | Windows | Linux |
|---|---|---|
| Notify | tray balloon / toast | `notify-send` per session, plus log |
| Cap | NVML `nvmlDeviceSetPowerManagementLimit` | same |
| Shutdown | `InitiateSystemShutdownExW` (force apps closed, reason "hardware: environment"), falling back to `shutdown /s /f /t 0` | `systemctl poweroff` → `shutdown -P now` → `poweroff` → `reboot(2)` |

The power cap is released when the process exits (Exit, service stop, or logoff), so a stopped watchdog never leaves the card throttled.

## Footprint

- One thread. Windows uses a message-loop timer; Linux uses a sleep loop.
- Each sample is one 24-byte I2C transaction.
- The binary is about 0.7 MB, with no runtime dependencies beyond the NVIDIA driver.
- Only one instance runs: a `Global\AstralWarden` mutex on Windows, an `flock` on `/run/lock/astral-warden.lock` on Linux.
