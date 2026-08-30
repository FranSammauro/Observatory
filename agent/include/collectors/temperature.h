#ifndef OBSERVER_COLLECTOR_TEMPERATURE_H
#define OBSERVER_COLLECTOR_TEMPERATURE_H

#include "agent.h"

/*
 * Temperatura del sistema desde /sys/class/thermal/thermal_zone*.
 * Metrica opcional: si no hay sensores disponibles, available queda en
 * false y no se considera un error de coleccion.
 */

typedef struct {
    bool available;
    double celsius;
} temperature_metrics_t;

obs_status_t temperature_collect(temperature_metrics_t *out);

#endif /* OBSERVER_COLLECTOR_TEMPERATURE_H */
