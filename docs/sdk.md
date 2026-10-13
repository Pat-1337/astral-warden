# SDK

GPU Warden's core ships as a shared library with a C interface, so other programs can read the 12V-2x6 pins directly.

| Platform | Archive | Files |
|---|---|---|
| Windows | `gpu-warden-<version>-windows-x86_64-sdk.zip` | `lib/gpu_warden.dll`, `lib/gpu_warden.lib`, `include/gpu_warden.h` |
| Linux | `gpu-warden-<version>-linux-x86_64-sdk.tar.gz` | `lib/libgpu_warden.so`, `include/gpu_warden.h` |

The library finds the same hardware as the app: ASUS ROG Astral and Matrix cards, and the Thermal Grizzly WireView Pro II. On Linux, reading a card needs root; on Windows, changing the power limit needs administrator rights.

## Example

```c
#include <stdio.h>
#include "gpu_warden.h"

int main(void) {
    GwDevice *device = gw_open(NULL);
    if (!device) {
        fprintf(stderr, "%s\n", gw_last_error());
        return 1;
    }

    GwReading reading;
    if (gw_read(device, &reading)) {
        uint32_t pin;
        int level = gw_assess(&reading, NULL, &pin);
        printf("%s: pin %u, level %d\n", gw_name(device), pin + 1, level);
    }

    gw_close(device);
    return 0;
}
```

Build it with `cc demo.c -Iinclude -Llib -lgpu_warden` on Linux, or link `gpu_warden.lib` with MSVC on Windows.

## Functions

| Function | What it does |
|---|---|
| `gw_version()` | library version |
| `gw_last_error()` | why the last failed call on this thread failed |
| `gw_open(port)` | finds a supported card or WireView; `port` names a WireView serial port, or `NULL` to search |
| `gw_open_simulated(file)` | reads six amp values from a text file instead of hardware |
| `gw_close(device)` | closes the device and restores any power limit it lowered |
| `gw_name(device)`, `gw_detail(device)` | the board name, and how it is read |
| `gw_read(device, out)` | one sample: volts and amps of all six pins |
| `gw_assess(reading, limits, pin)` | `GW_OK`, `GW_WARN` or `GW_CRIT`, and the pin at fault; `limits` may be `NULL` for the defaults |
| `gw_imbalance(reading, pin)` | the largest deviation from the average, as a fraction, and its pin |
| `gw_board_watts(device)`, `gw_limit_watts(device)` | board power and power limit from NVML, or NaN when unavailable |
| `gw_cap(device, percent)` | lowers the power limit to `percent` of the default |
| `gw_release(device)` | puts the previous power limit back |

`gw_read` allocates nothing, so it is cheap to call in a tight loop. Returned strings belong to the library and stay valid until the device is closed, or until the next failed call for `gw_last_error`.

The default limits are the app's: 9.5 A warn and 13 A critical per pin, 30% and 60% imbalance, judged above 20 A total.
