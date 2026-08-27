#ifndef OBSERVER_COLLECTOR_UPTIME_H
#define OBSERVER_COLLECTOR_UPTIME_H

#include "agent.h"

/*
 * Uptime del sistema (informe seccion 15). Se usa ademas para detectar
 * reboots: si uptime_actual < uptime_anterior, el Collector puede inferir
 * un evento SYSTEM_REBOOT (esa comparacion vive del lado del Collector,
 * no del agent - el agent solo reporta el valor crudo).
 */

typedef struct {
    unsigned long long uptime_secs;
} uptime_metrics_t;

obs_status_t uptime_collect(uptime_metrics_t *out);

/* Expuesto para tests: parsea el contenido de /proc/uptime. */
obs_status_t uptime_parse(const char *content, uptime_metrics_t *out);

#endif /* OBSERVER_COLLECTOR_UPTIME_H */
