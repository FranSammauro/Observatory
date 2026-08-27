#include "collectors/disk.h"

#include <stdio.h>
#include <string.h>
#include <math.h>

static int g_failures = 0;

#define CHECK(cond, msg) do { \
    if (!(cond)) { \
        fprintf(stderr, "FAIL: %s (%s:%d)\n", msg, __FILE__, __LINE__); \
        g_failures++; \
    } \
} while (0)

static void test_is_whole_device(void)
{
    CHECK(disk_is_whole_device("sda") == true, "sda is whole device");
    CHECK(disk_is_whole_device("sdb") == true, "sdb is whole device");
    CHECK(disk_is_whole_device("sda1") == false, "sda1 is a partition");
    CHECK(disk_is_whole_device("sda15") == false, "sda15 is a partition");
    CHECK(disk_is_whole_device("nvme0n1") == true, "nvme0n1 is whole device");
    CHECK(disk_is_whole_device("nvme0n1p1") == false, "nvme0n1p1 is a partition");
    CHECK(disk_is_whole_device("loop0") == false, "loop0 excluded");
    CHECK(disk_is_whole_device("ram0") == false, "ram0 excluded");
    CHECK(disk_is_whole_device("") == false, "empty string excluded");
}

static void test_parse_line(void)
{
    /* formato real de /proc/diskstats (14 campos tras major/minor/name) */
    const char *line =
        "   8       0 sda 1000 50 80000 1200 500 20 40000 900 0 1500 2100\n";

    disk_snapshot_entry_t entry;
    obs_status_t status = disk_parse_line(line, &entry);

    CHECK(status == OBS_OK, "parse should succeed");
    CHECK(strcmp(entry.device, "sda") == 0, "device name");
    CHECK(entry.read_ops == 1000, "read_ops");
    CHECK(entry.read_sectors == 80000, "read_sectors");
    CHECK(entry.write_ops == 500, "write_ops");
    CHECK(entry.write_sectors == 40000, "write_sectors");
    CHECK(entry.io_time_ms == 1500, "io_time_ms");
}

static void test_parse_malformed_line(void)
{
    disk_snapshot_entry_t entry;
    obs_status_t status = disk_parse_line("garbage line\n", &entry);
    CHECK(status == OBS_ERR_PARSE, "malformed line should fail");
}

static void test_first_collection_is_invalid(void)
{
    /* No podemos garantizar contenido real de /proc/diskstats en CI,
     * pero si podemos verificar el contrato: la primera lectura nunca
     * es valid=true (no hay delta todavia). */
    disk_collector_t collector;
    disk_collector_init(&collector);

    disk_metrics_t metrics;
    obs_status_t status = disk_collect(&collector, 1.0, &metrics);

    CHECK(status == OBS_OK, "first collection should not error");
    CHECK(metrics.valid == false, "first collection has no delta yet");
}

int main(void)
{
    test_is_whole_device();
    test_parse_line();
    test_parse_malformed_line();
    test_first_collection_is_invalid();

    if (g_failures == 0) {
        printf("test_disk: all tests passed\n");
        return 0;
    }
    fprintf(stderr, "test_disk: %d failure(s)\n", g_failures);
    return 1;
}
