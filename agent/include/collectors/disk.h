#ifndef OBSERVER_COLLECTOR_DISK_H
#define OBSERVER_COLLECTOR_DISK_H

#include "agent.h"

/*
 * Disk I/O (informe seccion 12) - distinto de filesystem capacity
 * (seccion 11). Se leen contadores acumulativos de /proc/diskstats y se
 * reportan como *rate* (bytes/sec, ops/sec) via delta entre dos
 * snapshots, igual que CPU.
 *
 * Solo se reportan dispositivos "reales" (se descartan particiones como
 * sda1, sda2 - se agregan a nivel de disco completo - y dispositivos
 * loop/ram) para mantener la cardinalidad bajo control.
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
    bool valid; /* false en la primera lectura (no hay delta aun) */
} disk_metrics_t;

typedef struct {
    disk_snapshot_t previous;
    bool has_previous;
} disk_collector_t;

void disk_collector_init(disk_collector_t *collector);

/*
 * `elapsed_secs` es el tiempo real transcurrido desde la lectura anterior
 * (reloj monotonico, ver informe seccion 57), usado para convertir el
 * delta de contadores en una tasa por segundo real en vez de asumir que
 * el intervalo configurado se cumplio exactamente.
 */
obs_status_t disk_collect(disk_collector_t *collector, double elapsed_secs, disk_metrics_t *out);

/* true si el nombre de dispositivo es un disco "real" que nos interesa
 * (se excluyen particiones, loopN, ramN, dmN de bajo interes, etc.).
 * Expuesto para tests. */
bool disk_is_whole_device(const char *device_name);

/* Parsea una linea de /proc/diskstats. Expuesto para tests. */
obs_status_t disk_parse_line(const char *line, disk_snapshot_entry_t *out);

#endif /* OBSERVER_COLLECTOR_DISK_H */
