#include "collectors/temperature.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>

#define COMPONENT "collector.temperature"
#define THERMAL_ZONE_PATH_FMT "/sys/class/thermal/thermal_zone%d/temp"
#define MAX_ZONES_TO_PROBE 4

obs_status_t temperature_collect(temperature_metrics_t *out)
{
    if (!out) {
        return OBS_ERR_INVALID_ARG;
    }

    out->available = false;
    out->celsius = 0.0;

    for (int zone = 0; zone < MAX_ZONES_TO_PROBE; zone++) {
        char path[OBS_MAX_PATH];
        snprintf(path, sizeof(path), THERMAL_ZONE_PATH_FMT, zone);

        FILE *fp = fopen(path, "r");
        if (!fp) {
            continue; /* zona no existe - no es un error, simplemente probamos la siguiente */
        }

        long millidegrees = 0;
        int matched = fscanf(fp, "%ld", &millidegrees);
        fclose(fp);

        if (matched == 1) {
            out->available = true;
            out->celsius = (double)millidegrees / 1000.0;
            LOG_DEBUG_(COMPONENT, "found sensor at zone %d: %.1fC", zone, out->celsius);
            return OBS_OK;
        }
    }

    LOG_DEBUG_(COMPONENT, "no thermal sensors available (not an error)");
    return OBS_OK;
}
