#include "collectors/network.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>
#include <ctype.h>

#define COMPONENT "collector.network"
#define PROC_NET_DEV_PATH "/proc/net/dev"

static char *trim_leading(char *s)
{
    while (isspace((unsigned char)*s)) {
        s++;
    }
    return s;
}

obs_status_t network_parse_line(const char *line, network_snapshot_entry_t *out)
{
    if (!line || !out) {
        return OBS_ERR_INVALID_ARG;
    }

    char buf[OBS_MAX_LINE];
    snprintf(buf, sizeof(buf), "%s", line);

    char *colon = strchr(buf, ':');
    if (!colon) {
        return OBS_ERR_PARSE; /* lineas de header no tienen ':' antes de los datos */
    }

    *colon = '\0';
    char *name = trim_leading(buf);
    if (name[0] == '\0') {
        return OBS_ERR_PARSE;
    }

    unsigned long long rx_bytes, rx_packets, rx_errs, rx_drop, rx_fifo, rx_frame,
                        rx_compressed, rx_multicast;
    unsigned long long tx_bytes, tx_packets, tx_errs, tx_drop, tx_fifo, tx_colls,
                        tx_carrier, tx_compressed;

    int matched = sscanf(colon + 1,
        "%llu %llu %llu %llu %llu %llu %llu %llu "
        "%llu %llu %llu %llu %llu %llu %llu %llu",
        &rx_bytes, &rx_packets, &rx_errs, &rx_drop, &rx_fifo, &rx_frame,
        &rx_compressed, &rx_multicast,
        &tx_bytes, &tx_packets, &tx_errs, &tx_drop, &tx_fifo, &tx_colls,
        &tx_carrier, &tx_compressed);

    (void)rx_drop; (void)rx_fifo; (void)rx_frame; (void)rx_compressed; (void)rx_multicast;
    (void)tx_drop; (void)tx_fifo; (void)tx_colls; (void)tx_carrier; (void)tx_compressed;

    if (matched < 16) {
        return OBS_ERR_PARSE;
    }

    snprintf(out->interface, sizeof(out->interface), "%.31s", name);
    out->rx_bytes = rx_bytes;
    out->tx_bytes = tx_bytes;
    out->rx_packets = rx_packets;
    out->tx_packets = tx_packets;
    out->rx_errors = rx_errs;
    out->tx_errors = tx_errs;

    return OBS_OK;
}

static bool should_skip_interface(const char *name)
{
    return strcmp(name, "lo") == 0;
}

void network_collector_init(network_collector_t *collector)
{
    memset(collector, 0, sizeof(*collector));
}

static obs_status_t read_snapshot(network_snapshot_t *out)
{
    memset(out, 0, sizeof(*out));

    FILE *fp = fopen(PROC_NET_DEV_PATH, "r");
    if (!fp) {
        LOG_ERROR_(COMPONENT, "failed to open '%s'", PROC_NET_DEV_PATH);
        return OBS_ERR_IO;
    }

    char line[OBS_MAX_LINE];
    int line_no = 0;

    while (fgets(line, sizeof(line), fp) && out->count < OBS_MAX_INTERFACES) {
        line_no++;
        if (line_no <= 2) {
            continue; /* dos lineas de header */
        }

        network_snapshot_entry_t entry;
        if (network_parse_line(line, &entry) != OBS_OK) {
            continue;
        }
        if (should_skip_interface(entry.interface)) {
            continue;
        }

        out->interfaces[out->count] = entry;
        out->count++;
    }

    fclose(fp);
    return OBS_OK;
}

static network_snapshot_entry_t *find_interface(network_snapshot_t *snapshot, const char *name)
{
    for (size_t i = 0; i < snapshot->count; i++) {
        if (strcmp(snapshot->interfaces[i].interface, name) == 0) {
            return &snapshot->interfaces[i];
        }
    }
    return NULL;
}

obs_status_t network_collect(network_collector_t *collector, double elapsed_secs,
                              network_metrics_t *out)
{
    if (!collector || !out) {
        return OBS_ERR_INVALID_ARG;
    }

    memset(out, 0, sizeof(*out));

    network_snapshot_t current;
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

    for (size_t i = 0; i < current.count && out->count < OBS_MAX_INTERFACES; i++) {
        network_snapshot_entry_t *curr_iface = &current.interfaces[i];
        network_snapshot_entry_t *prev_iface = find_interface(&collector->previous, curr_iface->interface);

        if (!prev_iface) {
            continue;
        }

        if (curr_iface->rx_bytes < prev_iface->rx_bytes ||
            curr_iface->tx_bytes < prev_iface->tx_bytes) {
            LOG_WARN_(COMPONENT, "counter reset detected for interface '%s', skipping",
                       curr_iface->interface);
            continue;
        }

        network_rate_entry_t *rate = &out->interfaces[out->count];
        snprintf(rate->interface, sizeof(rate->interface), "%s", curr_iface->interface);

        rate->rx_bytes_per_sec =
            (double)(curr_iface->rx_bytes - prev_iface->rx_bytes) / elapsed_secs;
        rate->tx_bytes_per_sec =
            (double)(curr_iface->tx_bytes - prev_iface->tx_bytes) / elapsed_secs;
        rate->rx_packets_per_sec =
            (double)(curr_iface->rx_packets - prev_iface->rx_packets) / elapsed_secs;
        rate->tx_packets_per_sec =
            (double)(curr_iface->tx_packets - prev_iface->tx_packets) / elapsed_secs;
        rate->rx_errors_total = curr_iface->rx_errors;
        rate->tx_errors_total = curr_iface->tx_errors;

        out->count++;
    }

    out->valid = true;
    collector->previous = current;

    return OBS_OK;
}
