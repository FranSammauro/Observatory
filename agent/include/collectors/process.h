#ifndef OBSERVER_COLLECTOR_PROCESS_H
#define OBSERVER_COLLECTOR_PROCESS_H

#include "agent.h"

/*
 * Conteo agregado de procesos por estado desde /proc/<pid>/stat. Se
 * reportan conteos totales por categoria (running, sleeping, stopped,
 * zombie) en lugar de metricas por PID para evitar cardinalidad no
 * acotada.
 */

typedef struct {
    unsigned int total;
    unsigned int running;
    unsigned int sleeping;
    unsigned int stopped;
    unsigned int zombie;
} process_metrics_t;

obs_status_t process_collect(process_metrics_t *out);

/* Expuesto para tests: normaliza el caracter de estado de /proc/<pid>/stat. */
char process_normalize_state(char raw_state);

#endif /* OBSERVER_COLLECTOR_PROCESS_H */
