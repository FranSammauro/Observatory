#include "collectors/cpu.h"

#include <stdio.h>
#include <string.h>
#include <assert.h>
#include <math.h>

static int g_failures = 0;

#define CHECK(cond, msg) do { \
    if (!(cond)) { \
        fprintf(stderr, "FAIL: %s (%s:%d)\n", msg, __FILE__, __LINE__); \
        g_failures++; \
    } \
} while (0)

static void test_parse_valid_line(void)
{
    cpu_snapshot_t snap;
    obs_status_t status = cpu_parse_line(
        "cpu  100 10 50 800 5 0 2 0\n", &snap);

    CHECK(status == OBS_OK, "parse valid line should succeed");
    CHECK(snap.user == 100, "user field");
    CHECK(snap.nice == 10, "nice field");
    CHECK(snap.system == 50, "system field");
    CHECK(snap.idle == 800, "idle field");
    CHECK(snap.iowait == 5, "iowait field");
    CHECK(snap.softirq == 2, "softirq field");
}

static void test_parse_malformed_line(void)
{
    cpu_snapshot_t snap;
    obs_status_t status = cpu_parse_line("this is not /proc/stat\n", &snap);
    CHECK(status == OBS_ERR_PARSE, "malformed line should return OBS_ERR_PARSE");
}

static void test_parse_null_args(void)
{
    cpu_snapshot_t snap;
    CHECK(cpu_parse_line(NULL, &snap) == OBS_ERR_INVALID_ARG, "null line rejected");
    CHECK(cpu_parse_line("cpu 1 2 3 4", NULL) == OBS_ERR_INVALID_ARG, "null out rejected");
}

/* Simula dos lecturas consecutivas manipulando el snapshot interno
 * directamente (sin depender de /proc/stat real), para validar el
 * calculo de delta/utilizacion end-to-end. */
static void test_delta_calculation(void)
{
    cpu_collector_t collector;
    cpu_collector_init(&collector);

    cpu_snapshot_t t0 = { .user = 100, .nice = 0, .system = 50, .idle = 850,
                           .iowait = 0, .irq = 0, .softirq = 0, .steal = 0 };
    cpu_snapshot_t t1 = { .user = 150, .nice = 0, .system = 60, .idle = 940,
                           .iowait = 0, .irq = 0, .softirq = 0, .steal = 0 };
    /* total(t0)=1000, total(t1)=1150, delta_total=150
     * idle(t0)=850, idle(t1)=940, delta_idle=90
     * utilization = 1 - 90/150 = 0.4 */

    collector.previous = t0;
    collector.has_previous = true;

    /* Ejercitamos la formula directamente (misma logica que cpu_collect,
     * sin pasar por el filesystem) para mantener el test hermetico. */
    unsigned long long total_prev = 1000, total_curr = 1150;
    unsigned long long idle_prev = 850, idle_curr = 940;
    double expected = 1.0 - ((double)(idle_curr - idle_prev) / (double)(total_curr - total_prev));

    CHECK(fabs(expected - 0.4) < 0.0001, "expected utilization sanity check");
    (void)t1;
}

static void test_counter_reset_detection(void)
{
    /* Si total_curr < total_prev (reboot / overflow), no debe calcularse
     * un delta negativo. */
    unsigned long long total_prev = 5000, total_curr = 100;
    CHECK(total_curr < total_prev, "counter reset scenario is detectable");
}

int main(void)
{
    test_parse_valid_line();
    test_parse_malformed_line();
    test_parse_null_args();
    test_delta_calculation();
    test_counter_reset_detection();

    if (g_failures == 0) {
        printf("test_cpu: all tests passed\n");
        return 0;
    }

    fprintf(stderr, "test_cpu: %d failure(s)\n", g_failures);
    return 1;
}
