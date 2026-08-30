#ifndef OBSERVER_COLLECTOR_FILESYSTEM_H
#define OBSERVER_COLLECTOR_FILESYSTEM_H

#include "agent.h"

/*
 * Collector de uso de filesystem desde /proc/mounts + statvfs(). Solo
 * se reportan filesystems reales (ext4, xfs, btrfs, etc.); los
 * pseudo-filesystems (proc, sysfs, tmpfs, overlay, cgroup) se
 * descartan para mantener la cardinalidad acotada.
 */

typedef struct {
    char device[OBS_MAX_PATH];
    char mountpoint[OBS_MAX_PATH];
    char fs_type[64];

    unsigned long long total_bytes;
    unsigned long long available_bytes;
    double utilization; /* ratio en [0, 1] */
} filesystem_entry_t;

typedef struct {
    filesystem_entry_t entries[OBS_MAX_FILESYSTEMS];
    size_t count;
} filesystem_metrics_t;

obs_status_t filesystem_collect(filesystem_metrics_t *out);

/* Expuesto para tests. */
bool filesystem_is_real_fs_type(const char *fs_type);
obs_status_t filesystem_parse_mounts_line(const char *line,
                                           char *device, size_t device_size,
                                           char *mountpoint, size_t mountpoint_size,
                                           char *fs_type, size_t fs_type_size);

#endif /* OBSERVER_COLLECTOR_FILESYSTEM_H */
