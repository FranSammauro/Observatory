#ifndef OBSERVER_COLLECTOR_DISK_H
#define OBSERVER_COLLECTOR_DISK_H

#include "agent.h"

/*
 * Collector de I/O de disco desde /proc/diskstats. Reporta tasas
 * (bytes/s, operaciones/s) calculadas por delta entre dos lecturas
 * consecutivas dividido por el tiempo real transcurrido.
 *
 * Solo se reportan discos completos; las particiones (sda1, nvme0n1p2)
 * y dispositivos virtuales (loop, ram) se descartan para mantener la
 * cardinalidad acotada.
 */

#define OBS_MAX_DISKS 16

typedef struct {
    char device[64];
    unsigned long long read_sectors;
    unsigned long long write_sectors;
    unsigned long long read_ops;
    unsigned long long write_ops;
    unsigned long long io_time_ms;
} disk_snapshot_entry_t;

typedef struct {
    disk_snapshot_entry_t disks[OBS_MAX_DISKS];
    size_t count;
} disk_snapshot_t;

typedef struct {
    char device[64];
    double read_bytes_per_sec;
    double write_bytes_per_sec;
    double read_ops_per_sec;
    double write_ops_per_sec;
} disk_rate_entry_t;

typedef struct {
    disk_rate_entry_t disks[OBS_MAX_DISKS];
    size_t count;
    bool valid;
} disk_metrics_t;

typedef struct {
    disk_snapshot_t previous;
    bool has_previous;
} disk_collector_t;

void disk_collector_init(disk_collector_t *collector);

/*
 * elapsed_secs es el tiempo real transcurrido desde la lectura anterior
 * (reloj monotonico), usado como divisor para convertir los deltas de
 * contadores a tasas por segundo.
 */
obs_status_t disk_collect(disk_collector_t *collector, double elapsed_secs,
                           disk_metrics_t *out);

/* Expuesto para tests. */
bool disk_is_whole_device(const char *device_name);
obs_status_t disk_parse_line(const char *line, disk_snapshot_entry_t *out);

#endif /* OBSERVER_COLLECTOR_DISK_H */
