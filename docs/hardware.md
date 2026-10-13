# Supported hardware

| Sensor | Read through |
|---|---|
| ASUS ROG Astral and Matrix cards below | the card's own I2C bus |
| Thermal Grizzly WireView Pro II, including the Noctua Edition | USB serial |

The card sensor is tried first. The WireView is used when no supported card is found, or when `wireview_port` is set.

## ASUS cards

Detection requires NVIDIA vendor `10de`, ASUS subsystem vendor `1043`, and both IDs below.

### GPUs

| Device | GPU |
|---|---|
| `2b85` | RTX 5090 |
| `2b87` | RTX 5090 D |
| `2b8c` | RTX 5090 D v2 |
| `2c02` | RTX 5080 |

### Boards (subsystem device)

| ID | Board | Sensor confirmed by |
|---|---|---|
| `89e3` | ROG Astral OC 5090 | LibreHardwareMonitor, 12vhpwr-guard, astral-hwmon |
| `89e4` | ROG Astral 5090 | listed in OpenRGB pci_ids, unconfirmed |
| `89ea` | ROG Astral D OC 5090 | LibreHardwareMonitor, 12vhpwr-guard, astral-hwmon |
| `89ec` | ROG Astral LC 5090 | LibreHardwareMonitor, 12vhpwr-guard, astral-hwmon |
| `89ed` | ROG Astral LC OC 5090 | unconfirmed |
| `8a2e` | ROG Astral OC White 5090 | 12vhpwr-guard, astral-hwmon |
| `8a3c`, `8a5a` | ROG Astral BTF OC 5090 | 12vhpwr-guard (pins read ~0 when slot-powered) |
| `8a61` | ROG Matrix Platinum 5090 (30th Anniversary) | LibreHardwareMonitor, 12vhpwr-guard, astral-hwmon |
| `89de` | ROG Astral OC 5080 | all |
| `89df` | ROG Astral 5080 | 12vhpwr-guard |
| `8a2b` | ROG Astral OC White 5080 | 12vhpwr-guard, astral-hwmon |
| `8a2c` | ROG Astral White 5080 | unconfirmed |
| `8a45` | ROG Astral OC Hatsune Miku 5080 | 12vhpwr-guard, astral-hwmon |

On Linux, unconfirmed boards still have to pass the frame plausibility check before monitoring starts. On Windows, a board whose direct read fails falls back to HWiNFO.

If `status` reports `10de:xxxx 1043:yyyy is not a supported board` for an Astral card, open an issue with that line and the `status` output.

Other cards have no per-pin sensing on board. Use a WireView Pro II with them.

## Thermal Grizzly WireView Pro II

It sits between the 12V-2x6 cable and the card, so it works with any card from any brand. Connect its USB cable to a motherboard USB 2.0 header.

| | |
|---|---|
| USB ID | `0483:5740` (serial port) |
| Products | `5` WireView Pro II, `6` Noctua Edition |
| Linux | found under `/dev/ttyACM*`, or read from the `wireview` hwmon device when wireviewd from [wireview-hwmon](https://github.com/emaspa/wireview-hwmon) already owns the port |
| Windows | found through the COM port Windows assigned to it |

The power cap needs to know which card the WireView feeds. With exactly one NVIDIA card in the PC, that card is capped. With none or several, the cap is off, but notifications and shutdown still work.

The WireView II (products 7 and 8) speaks a different protocol and is not supported yet.

## Not supported

| Hardware | Why |
|---|---|
| MSI GPU Safeguard PSUs | the PSU measures each pin, but the reading only reaches MSI Center over an undocumented link |
| Cooler Master GPU Shield | LED and buzzer only, no readout |
| ASUS ROG Thor III PSU, ROG Equalizer cable | total power only, nothing per pin |
