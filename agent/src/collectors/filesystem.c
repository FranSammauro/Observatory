#include "collectors/filesystem.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>
#include <sys/statvfs.h>

#define COMPONENT "collector.filesystem"
#define PROC_MOUNTS_PATH "/proc/mounts"

/* Filesystems "reales" que nos interesa reportar. Todo lo que no este
 * en esta lista se asume pseudo/virtual (proc, sysfs, tmpfs, devtmpfs,
 * cgroup, cgroup2, overlay, squashfs de snaps, etc.) y se descarta -
 * ver informe seccion 35 (cardinalidad). */
static const char *REAL_FS_TYPES[] = {
    "ext2", "ext3", "ext4", "xfs", "btrfs", "vfat", "exfat",
    "ntfs", "ntfs3", "zfs", "f2fs", "jfs", "reiserfs", "hfsplus",
    NULL
};

bool filesystem_is_real_fs_type(const char *fs_type)
{
    if (!fs_type) {
        return false;
    }
    for (int i = 0; REAL_FS_TYPES[i] != NULL; i++) {
        if (strcmp(fs_type, REAL_FS_TYPES[i]) == 0) {
            return true;
        }
    }
    return false;
}

obs_status_t filesystem_parse_mounts_line(const char *line,
                                           char *device, size_t device_size,
                                           char *mountpoint, size_t mountpoint_size,
                                           char *fs_type, size_t fs_type_size)
{
    if (!line || !device || !mountpoint || !fs_type) {
        return OBS_ERR_INVALID_ARG;
    }

    /* Formato /proc/mounts: "<device> <mountpoint> <fs_type> <options> 0 0" */
    char dev_buf[OBS_MAX_PATH];
    char mnt_buf[OBS_MAX_PATH];
    char type_buf[64];

    int matched = sscanf(line, "%255s %255s %63s", dev_buf, mnt_buf, type_buf);
    if (matched != 3) {
        return OBS_ERR_PARSE;
    }

    snprintf(device, device_size, "%s", dev_buf);
    snprintf(mountpoint, mountpoint_size, "%s", mnt_buf);
    snprintf(fs_type, fs_type_size, "%s", type_buf);

    return OBS_OK;
}

obs_status_t filesystem_collect(filesystem_metrics_t *out)
{
    if (!out) {
        return OBS_ERR_INVALID_ARG;
    }

    memset(out, 0, sizeof(*out));

    FILE *fp = fopen(PROC_MOUNTS_PATH, "r");
    if (!fp) {
        LOG_ERROR_(COMPONENT, "failed to open '%s'", PROC_MOUNTS_PATH);
        return OBS_ERR_IO;
    }

    char line[OBS_MAX_LINE];

    while (fgets(line, sizeof(line), fp) && out->count < OBS_MAX_FILESYSTEMS) {
        char device[OBS_MAX_PATH], mountpoint[OBS_MAX_PATH], fs_type[64];

        if (filesystem_parse_mounts_line(line, device, sizeof(device),
                                          mountpoint, sizeof(mountpoint),
                                          fs_type, sizeof(fs_type)) != OBS_OK) {
            continue;
        }

        if (!filesystem_is_real_fs_type(fs_type)) {
            continue;
        }

        struct statvfs vfs;
        if (statvfs(mountpoint, &vfs) != 0) {
            LOG_WARN_(COMPONENT, "statvfs('%s') failed, skipping", mountpoint);
            continue;
        }

        if (vfs.f_blocks == 0) {
            continue;
        }

        filesystem_entry_t *entry = &out->entries[out->count];
        snprintf(entry->device, sizeof(entry->device), "%s", device);
        snprintf(entry->mountpoint, sizeof(entry->mountpoint), "%s", mountpoint);
        snprintf(entry->fs_type, sizeof(entry->fs_type), "%s", fs_type);

        unsigned long long total = (unsigned long long)vfs.f_blocks * vfs.f_frsize;
        unsigned long long available = (unsigned long long)vfs.f_bavail * vfs.f_frsize;

        entry->total_bytes = total;
        entry->available_bytes = available;
        entry->utilization = (total > 0)
            ? 1.0 - ((double)available / (double)total)
            : 0.0;

        out->count++;
    }

    fclose(fp);
    return OBS_OK;
}
