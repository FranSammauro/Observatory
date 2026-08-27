#ifndef OBSERVER_COLLECTOR_TEMPERATURE_H
#define OBSERVER_COLLECTOR_TEMPERATURE_H

#include "agent.h"

/*
 * Temperatura (informe seccion 16) - metrica explicitamente opcional.
 * Busca sensores bajo /sys/class/thermal/thermal_zone*. Si no hay
 * ninguno disponible, `available=false` NO se considera un error.
 */

typedef struct {
    bool available;
    double celsius; /* solo valido si available == true */
} temperature_metrics_t;

obs_status_t temperature_collect(temperature_metrics_t *out);

#endif /* OBSERVER_COLLECTOR_TEMPERATURE_H */
