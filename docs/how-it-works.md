# How it works

## Sensor

ASUS ROG Astral and Matrix boards have an **ITE IT8915** ("Power Detector+") on the GPU's own I2C bus. It measures the voltage and current of each of the six 12 V pins.

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

### HWiNFO fallback (Windows)

If NVAPI can't read the chip (at startup, or on any single sample), the reading comes from HWiNFO's shared memory instead.

| | |
|---|---|
| Mapping | `Global\HWiNFO_SENS_SM2`, read-only, guarded by `Global\HWiNFO_SM2_MUTEX` (200 ms wait) |
| Header | `HWiS` signature; data older than 30 s counts as a failed read |
| Pins | entries labelled `GPU 12VHPWR Pin1..6 Current`, matched on the original or the renamed label |
| Volts | `GPU 12VHPWR Pin1..6 Voltage` if present, otherwise a nominal 12 V (only affects the watts shown) |

The six entry positions are cached and re-checked against their labels on every read, so a steady-state read touches six entries, not the whole table. HWiNFO is opened lazily and reopened after it restarts. When the direct read works, HWiNFO is never touched.

HWiNFO needs **Settings → Shared Memory Support** turned on. In the free version, that setting turns itself off after 12 hours unless it is re-enabled.

There is one more sanity check: if the pins sum to under 1 A while NVML reports more than 150 W of board power, the sample counts as a failed read. That happens on BTF boards powered through the motherboard slot, where current bypasses the sensor.

### WireView Pro II

The WireView is a USB serial device at 115200 baud. Every sample sends one command byte and reads a fixed 100-byte little-endian frame.

| | |
|---|---|
| Identify | command `0x01` returns vendor `0xEF`, product `5` or `6`, firmware version |
| Sample | command `0x04` returns the frame |
| Pins | 6 entries of 12 bytes from byte 12: `i16` millivolts, 2 padding bytes, `u32` milliamps, `u32` milliwatts |

The protocol has no framing or checksum, so a frame is rejected when its padding is not zero, its fan duty is over 100, or a pin is outside 6 to 16 V or over 30 A. A rejected frame clears the input buffer and is read again once. A failed port is closed and found again on the next sample, so unplugging and replugging the WireView recovers on its own.

On Linux, when the wireview-hwmon driver is loaded, the pins come from its `in0_input` to `in5_input` and `curr1_input` to `curr6_input` instead, because wireviewd owns the port. The driver stops answering 5 s after wireviewd stops, which counts as a failed read.

## Detection

Before any I2C traffic, the GPU's PCI vendor, device, and subsystem IDs must match the [supported list](hardware.md). Address `0x2B` on other boards may be a different chip.

Before a serial port is used for samples, it has to answer the identify command as a WireView Pro II. Other STM32 boards share its USB ID.

## Decision

Each sample is graded:

- **pin**: the hottest pin against `pin_amps`
- **imbalance**: the largest |pin − mean| / mean against `imbalance`, judged only when the total is at least `imbalance_min_total_amps`

```
            warn × warn_samples                 calm × warn_samples
  Normal ─────────────────────────▶ Warning ─────────────────────────▶ Recovering
    ▲                               │  cap power                       │
    │                               │  recover_secs elapsed            │ calm for restore_after_secs
    │                               ▼                                  ▼
    │                            SHUTDOWN ◀── warn again ──────────── Normal, restore limit
    │                               ▲
    └── any state: crit × crit_samples ─┘

  sensor fails × sensor_fail_samples: Normal/Recovering → Blind (cap power, notify)
  first good sample:                  Blind → Recovering (a warning here goes to Warning, not SHUTDOWN)
```

- "Calm" means every value is below 95% of its warn threshold. That hysteresis stops a pin hovering at the threshold from bouncing between Warning and Recovering.
- If a warning re-triggers while recovering from a warning, the cap was not enough, so the PC shuts down.
- During a warning, failed reads don't stop the shutdown timer.
- On shutdown, the cap is engaged and kept even while the process is stopped, and the shutdown request is retried every 15 s until it succeeds.
- With `shutdown = false`, the first critical condition sends a one-off critical notification and the warning continues.

## Actions

| Action | Windows | Linux |
|---|---|---|
| Notify | toast with the connector icon in the level's colour | `notify-send` per session with a dialog icon |
| Alarm | system sound every 2 s during a warning, *Critical Stop* on critical (`alarm = false` to mute) | `canberra-gtk-play` per session, plus the console bell |
| Last shutdown | at the next start: a toast and a message box with the time and reason | a notification and a log line |
| Cap | NVML `nvmlDeviceSetPowerManagementLimit` | same |
| Shutdown | `InitiateSystemShutdownExW` (force apps closed, reason "hardware: environment"), falling back to `shutdown /s /f /t 0` | `systemctl poweroff` → `shutdown -P now` → `poweroff` → `reboot(2)` |

The power cap is released when the process exits normally (Exit, service stop, or logoff), so a stopped watchdog doesn't leave the card throttled. Shutdown is the exception: the cap stays.

The limit saved before capping is also written to a state file (`/run/gpu-warden.power-cap`, or `C:\ProgramData\gpu-warden\power-cap.state`). If the process is killed while capped, the next start finds the cap still in place and restores the previous limit. A reboot clears both the cap and, on Linux, the file. On Windows, a stale file is ignored because the limit no longer matches.

When there are several GPUs, NVML is matched by PCI IDs and bus number.

## Footprint

- One thread. Windows uses a message-loop timer; Linux uses a sleep loop.
- Per tick: one sensor read and the state machine. The tray tooltip is refreshed at most every 5 s, and immediately when the state colour changes. The NVML board-power check (only used when the pins read near 0 A) runs at most every 5 s.
- No disk writes during normal operation. See [Log](install.md#log).
- Each sample is one 24-byte I2C transaction, or one 100-byte serial read for a WireView.
- The binary is about 0.7 MB, with no runtime dependencies beyond the NVIDIA driver (and glibc on Linux). A WireView needs no driver at all.
- Only one instance runs: a `Global\GPUWarden` mutex on Windows, an `flock` on `/run/lock/gpu-warden.lock` on Linux.
