#include "collectors/process.h"

#include <stdio.h>

static int g_failures = 0;

#define CHECK(cond, msg) do { \
    if (!(cond)) { \
        fprintf(stderr, "FAIL: %s (%s:%d)\n", msg, __FILE__, __LINE__); \
        g_failures++; \
    } \
} while (0)

static void test_normalize_state(void)
{
    CHECK(process_normalize_state('R') == 'R', "R -> running");
    CHECK(process_normalize_state('S') == 'S', "S -> sleeping");
    CHECK(process_normalize_state('D') == 'S', "D (uninterruptible) -> sleeping");
    CHECK(process_normalize_state('T') == 'T', "T -> stopped");
    CHECK(process_normalize_state('t') == 'T', "t (tracing stop) -> stopped");
    CHECK(process_normalize_state('Z') == 'Z', "Z -> zombie");
    CHECK(process_normalize_state('I') == '?', "I (idle kernel thread) -> unbucketed");
    CHECK(process_normalize_state('X') == '?', "unknown state -> unbucketed");
}

static void test_collect_against_real_system(void)
{
    /* Corre contra /proc real del contenedor - siempre hay al menos
     * este mismo proceso corriendo. */
    process_metrics_t metrics;
    obs_status_t status = process_collect(&metrics);

    CHECK(status == OBS_OK, "process_collect should succeed");
    CHECK(metrics.total > 0, "should see at least one process");
    CHECK(metrics.total >= metrics.running + metrics.sleeping + metrics.stopped + metrics.zombie,
           "bucketed counts should not exceed total");
}

int main(void)
{
    test_normalize_state();
    test_collect_against_real_system();

    if (g_failures == 0) {
        printf("test_process: all tests passed\n");
        return 0;
    }
    fprintf(stderr, "test_process: %d failure(s)\n", g_failures);
    return 1;
}
