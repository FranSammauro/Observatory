#ifndef OBSERVER_COLLECTOR_MEMORY_H
#define OBSERVER_COLLECTOR_MEMORY_H

#include "agent.h"

/*
 * Collector de memoria desde /proc/meminfo.
 *
 * Se prioriza MemAvailable sobre MemFree porque ya descuenta cache y
 * buffers reclamables, dando una estimacion mas precisa de la memoria
 * realmente disponible para nuevas asignaciones. En kernels que no
 * exponen MemAvailable se usa MemFree como fallback.
 */

typedef struct {
    unsigned long long mem_total_kb;
    unsigned long long mem_available_kb;
    unsigned long long swap_total_kb;
    unsigned long long swap_free_kb;

    double mem_utilization;   /* ratio en [0, 1] */
    double swap_utilization;  /* ratio en [0, 1] */
} memory_metrics_t;

obs_status_t memory_collect(memory_metrics_t *out);

/* Expuesto para tests: parsea el contenido completo de /proc/meminfo. */
obs_status_t memory_parse_meminfo(const char *content, memory_metrics_t *out);

#endif /* OBSERVER_COLLECTOR_MEMORY_H */
