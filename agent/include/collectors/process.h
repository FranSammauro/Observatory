#ifndef OBSERVER_COLLECTOR_PROCESS_H
#define OBSERVER_COLLECTOR_PROCESS_H

#include "agent.h"

/*
 * Conteo agregado de procesos (informe seccion 14). Deliberadamente NO
 * se reporta una serie por PID (cardinalidad no acotada, ver seccion
 * 35) - solo conteos agregados por estado.
 */

typedef struct {
    unsigned int total;
    unsigned int running;
    unsigned int sleeping;
    unsigned int stopped;
    unsigned int zombie;
} process_metrics_t;

obs_status_t process_collect(process_metrics_t *out);

/* Mapea el caracter de estado de /proc/<pid>/stat (R, S, D, T, Z, ...)
 * a una de las categorias agregadas. Expuesto para tests. */
char process_normalize_state(char raw_state);

#endif /* OBSERVER_COLLECTOR_PROCESS_H */
