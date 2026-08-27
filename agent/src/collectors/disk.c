#include "collectors/disk.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>
#include <ctype.h>
#include <time.h>

#define COMPONENT "collector.disk"
#define PROC_DISKSTATS_PATH "/proc/diskstats"
#define SECTOR_SIZE_BYTES 512ULL

bool disk_is_whole_device(const char *device_name)
{
    if (!device_name || device_name[0] == '\0') {
        return false;
    }

    if (strncmp(device_name, "loop", 4) == 0) {
        return false;
    }
    if (strncmp(device_name, "ram", 3) == 0) {
        return false;
    }

    if (strncmp(device_name, "nvme", 4) == 0) {
        /* nvme0n1 = disco entero, nvme0n1p1 = particion */
        return strchr(device_name, 'p') == NULL;
    }

    /* sda, hda, vda, xvda = disco entero; sda1, sda2 = particion */
    size_t len = strlen(device_name);
    return !isdigit((unsigned char)device_name[len - 1]);
}

obs_status_t disk_parse_line(const char *line, disk_snapshot_entry_t *out)
{
    if (!line || !out) {
        return OBS_ERR_INVALID_ARG;
    }

    unsigned int major, minor;
    char name[64];
    unsigned long long reads_completed, reads_merged, sectors_read, time_reading;
    unsigned long long writes_completed, writes_merged, sectors_written, time_writing;
    unsigned long long ios_in_progress, time_io, weighted_time_io;

    int matched = sscanf(line,
        "%u %u %63s %llu %llu %llu %llu %llu %llu %llu %llu %llu %llu %llu",
        &major, &minor, name,
        &reads_completed, &reads_merged, &sectors_read, &time_reading,
        &writes_completed, &writes_merged, &sectors_written, &time_writing,
        &ios_in_progress, &time_io, &weighted_time_io);

    (void)major;
    (void)minor;
    (void)reads_merged;
    (void)writes_merged;
    (void)time_reading;
    (void)time_writing;
    (void)ios_in_progress;
    (void)weighted_time_io;

    if (matched < 14) {
        return OBS_ERR_PARSE;
    }

    snprintf(out->device, sizeof(out->device), "%s", name);
    out->read_sectors = sectors_read;
    out->write_sectors = sectors_written;
    out->read_ops = reads_completed;
    out->write_ops = writes_completed;
    out->io_time_ms = time_io;

    return OBS_OK;
}

void disk_collector_init(disk_collector_t *collector)
{
    memset(collector, 0, sizeof(*collector));
}

static obs_status_t read_snapshot(disk_snapshot_t *out)
{
    memset(out, 0, sizeof(*out));

    FILE *fp = fopen(PROC_DISKSTATS_PATH, "r");
    if (!fp) {
        LOG_ERROR_(COMPONENT, "failed to open '%s'", PROC_DISKSTATS_PATH);
        return OBS_ERR_IO;
    }

    char line[OBS_MAX_LINE];
    while (fgets(line, sizeof(line), fp) && out->count < OBS_MAX_DISKS) {
        disk_snapshot_entry_t entry;
        if (disk_parse_line(line, &entry) != OBS_OK) {
            continue;
        }
        if (!disk_is_whole_device(entry.device)) {
            continue;
        }
        out->disks[out->count] = entry;
        out->count++;
    }

    fclose(fp);
    return OBS_OK;
}

static disk_snapshot_entry_t *find_device(disk_snapshot_t *snapshot, const char *device)
{
    for (size_t i = 0; i < snapshot->count; i++) {
        if (strcmp(snapshot->disks[i].device, device) == 0) {
            return &snapshot->disks[i];
        }
    }
    return NULL;
}

obs_status_t disk_collect(disk_collector_t *collector, double elapsed_secs, disk_metrics_t *out)
{
    if (!collector || !out) {
        return OBS_ERR_INVALID_ARG;
    }

    memset(out, 0, sizeof(*out));

    disk_snapshot_t current;
    obs_status_t status = read_snapshot(&current);
    if (status != OBS_OK) {
        return status;
    }

    if (!collector->has_previous) {
        collector->previous = current;
        collector->has_previous = true;
        out->valid = false;
        return OBS_OK;
    }

    if (elapsed_secs <= 0.0) {
        LOG_WARN_(COMPONENT, "non-positive elapsed_secs (%.3f), discarding sample", elapsed_secs);
        collector->previous = current;
        out->valid = false;
        return OBS_OK;
    }

    for (size_t i = 0; i < current.count && out->count < OBS_MAX_DISKS; i++) {
        disk_snapshot_entry_t *curr_entry = &current.disks[i];
        disk_snapshot_entry_t *prev_entry = find_device(&collector->previous, curr_entry->device);

        if (!prev_entry) {
            continue; /* disco nuevo, sin punto de comparacion aun */
        }

        if (curr_entry->read_sectors < prev_entry->read_sectors ||
            curr_entry->write_sectors < prev_entry->write_sectors) {
            LOG_WARN_(COMPONENT, "counter reset detected for device '%s', skipping",
                       curr_entry->device);
            continue;
        }

        disk_rate_entry_t *rate = &out->disks[out->count];
        snprintf(rate->device, sizeof(rate->device), "%s", curr_entry->device);

        double read_bytes_delta =
            (double)((curr_entry->read_sectors - prev_entry->read_sectors) * SECTOR_SIZE_BYTES);
        double write_bytes_delta =
            (double)((curr_entry->write_sectors - prev_entry->write_sectors) * SECTOR_SIZE_BYTES);
        double read_ops_delta = (double)(curr_entry->read_ops - prev_entry->read_ops);
        double write_ops_delta = (double)(curr_entry->write_ops - prev_entry->write_ops);

        rate->read_bytes_per_sec = read_bytes_delta / elapsed_secs;
        rate->write_bytes_per_sec = write_bytes_delta / elapsed_secs;
        rate->read_ops_per_sec = read_ops_delta / elapsed_secs;
        rate->write_ops_per_sec = write_ops_delta / elapsed_secs;

        out->count++;
    }

    out->valid = true;
    collector->previous = current;

    return OBS_OK;
}
