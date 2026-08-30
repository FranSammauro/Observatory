#ifndef OBSERVER_COLLECTOR_UPTIME_H
#define OBSERVER_COLLECTOR_UPTIME_H

#include "agent.h"

/*
 * Uptime del sistema desde /proc/uptime. El valor es monotonamente
 * creciente; el collector detecta reboots al comparar muestras
 * consecutivas y registrar caidas de uptime.
 */

typedef struct {
    unsigned long long uptime_secs;
} uptime_metrics_t;

obs_status_t uptime_collect(uptime_metrics_t *out);
obs_status_t uptime_parse(const char *content, uptime_metrics_t *out);

#endif /* OBSERVER_COLLECTOR_UPTIME_H */
