#ifndef OBSERVER_COLLECTOR_MEMORY_H
#define OBSERVER_COLLECTOR_MEMORY_H

#include "agent.h"

/*
 * Collector de memoria (informe tecnico, seccion 10).
 *
 * Se prioriza MemAvailable (cuando el kernel lo expone) por sobre un
 * calculo naive de MemFree, ya que MemAvailable ya tiene en cuenta
 * cache/buffers reclamables. Esto coincide con la recomendacion de las
 * OpenTelemetry Semantic Conventions para metricas de sistema en Linux.
 */

typedef struct {
    unsigned long long mem_total_kb;
    unsigned long long mem_available_kb;
    unsigned long long swap_total_kb;
    unsigned long long swap_free_kb;

    double mem_utilization;    /* ratio 0..1 */
    double swap_utilization;   /* ratio 0..1 */
} memory_metrics_t;

/* Lee /proc/meminfo y calcula las metricas de memoria/swap. */
obs_status_t memory_collect(memory_metrics_t *out);

/* Expuesto para tests: parsea el contenido completo de /proc/meminfo. */
obs_status_t memory_parse_meminfo(const char *content, memory_metrics_t *out);

#endif /* OBSERVER_COLLECTOR_MEMORY_H */
