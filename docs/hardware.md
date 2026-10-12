# Supported hardware

Detection requires NVIDIA vendor `10de`, ASUS subsystem vendor `1043`, and both IDs below.

## GPUs

| Device | GPU |
|---|---|
| `2b85` | RTX 5090 |
| `2b87` | RTX 5090 D |
| `2b8c` | RTX 5090 D v2 |
| `2c02` | RTX 5080 |

## Boards (subsystem device)

| ID | Board | Sensor confirmed by |
|---|---|---|
| `89e3` | ROG Astral OC 5090 | LibreHardwareMonitor, 12vhpwr-guard, astral-hwmon |
| `89e4` | ROG Astral 5090 | listed in OpenRGB pci_ids, unconfirmed |
| `89ea` | ROG Astral D OC 5090 | LibreHardwareMonitor, 12vhpwr-guard, astral-hwmon |
| `89ec` | ROG Astral LC 5090 | LibreHardwareMonitor, 12vhpwr-guard, astral-hwmon |
| `89ed` | ROG Astral LC OC 5090 | unconfirmed |
| `8a2e` | ROG Astral OC White 5090 | 12vhpwr-guard, astral-hwmon |
| `8a3c`, `8a5a` | ROG Astral BTF OC 5090 | 12vhpwr-guard (pins read ~0 when slot-powered) |
| `8a61` | ROG Matrix Platinum 5090 | LibreHardwareMonitor, 12vhpwr-guard, astral-hwmon |
| `89de` | ROG Astral OC 5080 | all |
| `89df` | ROG Astral 5080 | 12vhpwr-guard |
| `8a2b` | ROG Astral OC White 5080 | 12vhpwr-guard, astral-hwmon |
| `8a2c` | ROG Astral White 5080 | unconfirmed |
| `8a45` | ROG Astral OC Hatsune Miku 5080 | 12vhpwr-guard, astral-hwmon |

On Linux, unconfirmed boards still have to pass the frame plausibility check before monitoring starts. On Windows, a board whose direct read fails falls back to HWiNFO.

If `status` reports `10de:xxxx 1043:yyyy is not a supported board` for an Astral card, open an issue with that line and the `status` output.

Non-Astral 5080/5090 cards have no per-pin sensing. An in-line meter such as the Thermal Grizzly WireView Pro II is the alternative for them.
