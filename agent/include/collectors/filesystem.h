#ifndef OBSERVER_COLLECTOR_FILESYSTEM_H
#define OBSERVER_COLLECTOR_FILESYSTEM_H

#include "agent.h"

/*
 * Uso de filesystem (informe seccion 11). Se usa statvfs() sobre cada
 * mountpoint real listado en /proc/mounts (se filtran filesystems
 * virtuales/pseudo como proc, sysfs, tmpfs, cgroup, overlay, etc. -
 * ver la lista en filesystem.c - para no reportar cardinalidad inutil).
 */

typedef struct {
    char device[OBS_MAX_PATH];
    char mountpoint[OBS_MAX_PATH];
    char fs_type[64];

    unsigned long long total_bytes;
    unsigned long long available_bytes;
    double utilization; /* ratio 0..1 */
} filesystem_entry_t;

typedef struct {
    filesystem_entry_t entries[OBS_MAX_FILESYSTEMS];
    size_t count;
} filesystem_metrics_t;

obs_status_t filesystem_collect(filesystem_metrics_t *out);

/* true si `fs_type` es un filesystem "real" que nos interesa reportar
 * (no pseudo/virtual). Expuesto para tests. */
bool filesystem_is_real_fs_type(const char *fs_type);

/* Parsea una linea de /proc/mounts en device/mountpoint/fs_type.
 * Expuesto para tests. */
obs_status_t filesystem_parse_mounts_line(const char *line,
                                           char *device, size_t device_size,
                                           char *mountpoint, size_t mountpoint_size,
                                           char *fs_type, size_t fs_type_size);

#endif /* OBSERVER_COLLECTOR_FILESYSTEM_H */
