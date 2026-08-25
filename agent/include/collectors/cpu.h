#ifndef OBSERVER_COLLECTOR_CPU_H
#define OBSERVER_COLLECTOR_CPU_H

#include "agent.h"

/*
 * Collector de CPU.
 *
 * Importante (informe tecnico, seccion 9): el porcentaje de CPU NO es una
 * lectura instantanea. /proc/stat expone contadores acumulativos desde el
 * boot, por lo que la utilizacion se calcula como delta entre dos
 * snapshots consecutivos:
 *
 *   total_delta = total(t1) - total(t0)
 *   idle_delta  = idle(t1)  - idle(t0)
 *   utilization = 1 - idle_delta / total_delta
 *
 * cpu_collector_t mantiene el snapshot anterior para poder calcular ese
 * delta en cada llamada a cpu_collect().
 */

typedef struct {
    unsigned long long user;
    unsigned long long nice;
    unsigned long long system;
    unsigned long long idle;
    unsigned long long iowait;
    unsigned long long irq;
    unsigned long long softirq;
    unsigned long long steal;
} cpu_snapshot_t;

typedef struct {
    cpu_snapshot_t previous;
    bool has_previous;
} cpu_collector_t;

typedef struct {
    double utilization;      /* ratio 0..1, ver informe seccion 19 (unidades) */
    double user_ratio;
    double system_ratio;
    double iowait_ratio;
    bool valid;               /* false en la primera lectura: aun no hay delta */
} cpu_metrics_t;

void cpu_collector_init(cpu_collector_t *collector);

/*
 * Lee /proc/stat, calcula el delta contra la lectura anterior y actualiza
 * el snapshot interno. En la primera llamada out->valid sera false porque
 * todavia no existe un delta.
 */
obs_status_t cpu_collect(cpu_collector_t *collector, cpu_metrics_t *out);

/* Expuesto para tests: parsea una linea "cpu  ..." de /proc/stat. */
obs_status_t cpu_parse_line(const char *line, cpu_snapshot_t *out);

#endif /* OBSERVER_COLLECTOR_CPU_H */
