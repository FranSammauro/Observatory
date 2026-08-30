#ifndef OBSERVER_COLLECTOR_CPU_H
#define OBSERVER_COLLECTOR_CPU_H

#include "agent.h"

/*
 * Collector de CPU basado en /proc/stat.
 *
 * La utilizacion no es una lectura instantanea: /proc/stat expone
 * contadores acumulativos desde el boot. La metrica se calcula como
 * la razon entre el delta del tiempo no ocioso y el delta del tiempo
 * total entre dos lecturas consecutivas.
 *
 * En la primera llamada a cpu_collect() out->valid sera false porque
 * todavia no existe un punto de comparacion.
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
    double utilization;   /* ratio en [0, 1] */
    double user_ratio;
    double system_ratio;
    double iowait_ratio;
    bool valid;
} cpu_metrics_t;

void cpu_collector_init(cpu_collector_t *collector);
obs_status_t cpu_collect(cpu_collector_t *collector, cpu_metrics_t *out);

/* Expuesto para tests: parsea la linea "cpu ..." de /proc/stat. */
obs_status_t cpu_parse_line(const char *line, cpu_snapshot_t *out);

#endif /* OBSERVER_COLLECTOR_CPU_H */
