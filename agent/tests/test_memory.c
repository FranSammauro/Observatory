#include "collectors/memory.h"

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

static const char *SAMPLE_MEMINFO =
    "MemTotal:       16384000 kB\n"
    "MemFree:         2048000 kB\n"
    "MemAvailable:    8192000 kB\n"
    "Buffers:          512000 kB\n"
    "Cached:          4096000 kB\n"
    "SwapTotal:       2097152 kB\n"
    "SwapFree:        2097152 kB\n";

static void test_parse_with_mem_available(void)
{
    memory_metrics_t metrics;
    obs_status_t status = memory_parse_meminfo(SAMPLE_MEMINFO, &metrics);

    CHECK(status == OBS_OK, "parse should succeed");
    CHECK(metrics.mem_total_kb == 16384000, "mem_total_kb");
    CHECK(metrics.mem_available_kb == 8192000, "mem_available_kb");

    double expected_util = (double)(16384000 - 8192000) / 16384000.0;
    CHECK(fabs(metrics.mem_utilization - expected_util) < 0.0001, "mem_utilization");

    /* swap totalmente libre -> utilization 0 */
    CHECK(fabs(metrics.swap_utilization - 0.0) < 0.0001, "swap_utilization when fully free");
}

static void test_parse_without_mem_available_falls_back_to_free(void)
{
    const char *content =
        "MemTotal:       1000000 kB\n"
        "MemFree:         200000 kB\n"
        "SwapTotal:             0 kB\n"
        "SwapFree:               0 kB\n";

    memory_metrics_t metrics;
    obs_status_t status = memory_parse_meminfo(content, &metrics);

    CHECK(status == OBS_OK, "parse should succeed without MemAvailable");
    CHECK(metrics.mem_available_kb == 200000, "falls back to MemFree");
    CHECK(fabs(metrics.swap_utilization - 0.0) < 0.0001, "swap_utilization with zero swap_total");
}

static void test_parse_missing_mem_total_fails(void)
{
    const char *content = "SomeOtherField:  123 kB\n";

    memory_metrics_t metrics;
    obs_status_t status = memory_parse_meminfo(content, &metrics);

    CHECK(status == OBS_ERR_PARSE, "missing MemTotal should fail");
}

static void test_parse_null_args(void)
{
    memory_metrics_t metrics;
    CHECK(memory_parse_meminfo(NULL, &metrics) == OBS_ERR_INVALID_ARG, "null content rejected");
    CHECK(memory_parse_meminfo("MemTotal: 1 kB\n", NULL) == OBS_ERR_INVALID_ARG, "null out rejected");
}

int main(void)
{
    test_parse_with_mem_available();
    test_parse_without_mem_available_falls_back_to_free();
    test_parse_missing_mem_total_fails();
    test_parse_null_args();

    if (g_failures == 0) {
        printf("test_memory: all tests passed\n");
        return 0;
    }

    fprintf(stderr, "test_memory: %d failure(s)\n", g_failures);
    return 1;
}
