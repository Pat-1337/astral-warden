#ifndef GPU_WARDEN_H
#define GPU_WARDEN_H

#include <stdbool.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define GW_PINS 6

typedef struct GwDevice GwDevice;

typedef struct {
    float volts[GW_PINS];
    float amps[GW_PINS];
} GwReading;

typedef struct {
    float pin_warn;
    float pin_crit;
    float imbalance_warn;
    float imbalance_crit;
    float imbalance_min_total_amps;
} GwLimits;

enum {
    GW_OK = 0,
    GW_WARN = 1,
    GW_CRIT = 2,
};

const char *gw_version(void);
const char *gw_last_error(void);

GwDevice *gw_open(const char *wireview_port);
GwDevice *gw_open_simulated(const char *file);
void gw_close(GwDevice *device);

const char *gw_name(const GwDevice *device);
const char *gw_detail(const GwDevice *device);

bool gw_read(GwDevice *device, GwReading *out);
int32_t gw_assess(const GwReading *reading, const GwLimits *limits, uint32_t *pin);
float gw_imbalance(const GwReading *reading, uint32_t *pin);

float gw_board_watts(const GwDevice *device);
float gw_limit_watts(const GwDevice *device);
bool gw_cap(GwDevice *device, uint32_t percent);
bool gw_release(GwDevice *device);

#ifdef __cplusplus
}
#endif

#endif
