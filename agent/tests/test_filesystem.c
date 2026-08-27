#include "collectors/filesystem.h"

#include <stdio.h>
#include <string.h>

static int g_failures = 0;

#define CHECK(cond, msg) do { \
    if (!(cond)) { \
        fprintf(stderr, "FAIL: %s (%s:%d)\n", msg, __FILE__, __LINE__); \
        g_failures++; \
    } \
} while (0)

static void test_is_real_fs_type(void)
{
    CHECK(filesystem_is_real_fs_type("ext4") == true, "ext4 is real");
    CHECK(filesystem_is_real_fs_type("xfs") == true, "xfs is real");
    CHECK(filesystem_is_real_fs_type("btrfs") == true, "btrfs is real");
    CHECK(filesystem_is_real_fs_type("proc") == false, "proc is pseudo fs");
    CHECK(filesystem_is_real_fs_type("sysfs") == false, "sysfs is pseudo fs");
    CHECK(filesystem_is_real_fs_type("tmpfs") == false, "tmpfs is pseudo fs");
    CHECK(filesystem_is_real_fs_type("overlay") == false, "overlay is pseudo fs");
    CHECK(filesystem_is_real_fs_type("cgroup2") == false, "cgroup2 is pseudo fs");
    CHECK(filesystem_is_real_fs_type(NULL) == false, "NULL is not real");
}

static void test_parse_mounts_line(void)
{
    const char *line = "/dev/sda2 / ext4 rw,relatime,errors=remount-ro 0 0\n";

    char device[OBS_MAX_PATH], mountpoint[OBS_MAX_PATH], fs_type[64];
    obs_status_t status = filesystem_parse_mounts_line(
        line, device, sizeof(device), mountpoint, sizeof(mountpoint), fs_type, sizeof(fs_type));

    CHECK(status == OBS_OK, "parse should succeed");
    CHECK(strcmp(device, "/dev/sda2") == 0, "device");
    CHECK(strcmp(mountpoint, "/") == 0, "mountpoint");
    CHECK(strcmp(fs_type, "ext4") == 0, "fs_type");
}

static void test_parse_pseudo_fs_line(void)
{
    const char *line = "proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n";

    char device[OBS_MAX_PATH], mountpoint[OBS_MAX_PATH], fs_type[64];
    obs_status_t status = filesystem_parse_mounts_line(
        line, device, sizeof(device), mountpoint, sizeof(mountpoint), fs_type, sizeof(fs_type));

    CHECK(status == OBS_OK, "parse itself should still succeed");
    CHECK(strcmp(fs_type, "proc") == 0, "fs_type parsed as proc");
    CHECK(filesystem_is_real_fs_type(fs_type) == false, "but proc is filtered out downstream");
}

static void test_collect_runs_against_real_system(void)
{
    /* Integracion liviana: corre contra el /proc/mounts real del
     * contenedor. No podemos aserts sobre el contenido exacto, pero si
     * sobre el contrato (no debe fallar, y todo lo que devuelva debe
     * tener un fs_type "real"). */
    filesystem_metrics_t metrics;
    obs_status_t status = filesystem_collect(&metrics);

    CHECK(status == OBS_OK, "filesystem_collect should not error");
    for (size_t i = 0; i < metrics.count; i++) {
        CHECK(filesystem_is_real_fs_type(metrics.entries[i].fs_type),
               "every reported entry must be a real fs type");
    }
}

int main(void)
{
    test_is_real_fs_type();
    test_parse_mounts_line();
    test_parse_pseudo_fs_line();
    test_collect_runs_against_real_system();

    if (g_failures == 0) {
        printf("test_filesystem: all tests passed\n");
        return 0;
    }
    fprintf(stderr, "test_filesystem: %d failure(s)\n", g_failures);
    return 1;
}
