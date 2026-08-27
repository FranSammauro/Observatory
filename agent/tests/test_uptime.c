#include "collectors/uptime.h"

#include <stdio.h>

static int g_failures = 0;

#define CHECK(cond, msg) do { \
    if (!(cond)) { \
        fprintf(stderr, "FAIL: %s (%s:%d)\n", msg, __FILE__, __LINE__); \
        g_failures++; \
    } \
} while (0)

static void test_parse_valid(void)
{
    uptime_metrics_t metrics;
    obs_status_t status = uptime_parse("123456.78 987654.32\n", &metrics);

    CHECK(status == OBS_OK, "parse should succeed");
    CHECK(metrics.uptime_secs == 123456, "uptime truncated to whole seconds");
}

static void test_parse_malformed(void)
{
    uptime_metrics_t metrics;
    obs_status_t status = uptime_parse("not a number\n", &metrics);
    CHECK(status == OBS_ERR_PARSE, "malformed content should fail");
}

static void test_parse_null_args(void)
{
    uptime_metrics_t metrics;
    CHECK(uptime_parse(NULL, &metrics) == OBS_ERR_INVALID_ARG, "null content rejected");
    CHECK(uptime_parse("1.0 2.0", NULL) == OBS_ERR_INVALID_ARG, "null out rejected");
}

static void test_collect_against_real_system(void)
{
    uptime_metrics_t metrics;
    obs_status_t status = uptime_collect(&metrics);
    CHECK(status == OBS_OK, "uptime_collect should succeed on a real Linux system");
}

int main(void)
{
    test_parse_valid();
    test_parse_malformed();
    test_parse_null_args();
    test_collect_against_real_system();

    if (g_failures == 0) {
        printf("test_uptime: all tests passed\n");
        return 0;
    }
    fprintf(stderr, "test_uptime: %d failure(s)\n", g_failures);
    return 1;
}
