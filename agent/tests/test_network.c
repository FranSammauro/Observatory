#include "collectors/network.h"

#include <stdio.h>
#include <string.h>

static int g_failures = 0;

#define CHECK(cond, msg) do { \
    if (!(cond)) { \
        fprintf(stderr, "FAIL: %s (%s:%d)\n", msg, __FILE__, __LINE__); \
        g_failures++; \
    } \
} while (0)

static void test_parse_line(void)
{
    /* formato real de /proc/net/dev */
    const char *line =
        "  eth0: 123456   200    0    0    0     0          0         0 "
        "654321   150    0    0    0     0       0          0\n";

    network_snapshot_entry_t entry;
    obs_status_t status = network_parse_line(line, &entry);

    CHECK(status == OBS_OK, "parse should succeed");
    CHECK(strcmp(entry.interface, "eth0") == 0, "interface name trimmed");
    CHECK(entry.rx_bytes == 123456, "rx_bytes");
    CHECK(entry.rx_packets == 200, "rx_packets");
    CHECK(entry.tx_bytes == 654321, "tx_bytes");
    CHECK(entry.tx_packets == 150, "tx_packets");
}

static void test_parse_header_line_fails(void)
{
    network_snapshot_entry_t entry;
    /* primera linea de header real de /proc/net/dev, sin ':' antes de numeros */
    obs_status_t status = network_parse_line(
        "Inter-|   Receive                                                |  Transmit\n",
        &entry);
    CHECK(status == OBS_ERR_PARSE, "header line without full fields should fail");
}

static void test_parse_malformed_line(void)
{
    network_snapshot_entry_t entry;
    obs_status_t status = network_parse_line("eth0: not enough fields\n", &entry);
    CHECK(status == OBS_ERR_PARSE, "malformed line should fail");
}

static void test_first_collection_is_invalid(void)
{
    network_collector_t collector;
    network_collector_init(&collector);

    network_metrics_t metrics;
    obs_status_t status = network_collect(&collector, 1.0, &metrics);

    CHECK(status == OBS_OK, "first collection should not error");
    CHECK(metrics.valid == false, "first collection has no delta yet");
}

int main(void)
{
    test_parse_line();
    test_parse_header_line_fails();
    test_parse_malformed_line();
    test_first_collection_is_invalid();

    if (g_failures == 0) {
        printf("test_network: all tests passed\n");
        return 0;
    }
    fprintf(stderr, "test_network: %d failure(s)\n", g_failures);
    return 1;
}
