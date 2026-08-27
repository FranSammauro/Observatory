#ifndef OBSERVER_COLLECTOR_NETWORK_H
#define OBSERVER_COLLECTOR_NETWORK_H

#include "agent.h"

/*
 * Trafico de red por interfaz (informe seccion 13), via /proc/net/dev.
 * Igual que CPU y disk, se reporta como *rate* calculada por delta
 * entre snapshots, usando el tiempo real transcurrido (reloj
 * monotonico) para el divisor.
 *
 * No se reportan interfaces loopback (lo) ni interfaces down, para
 * mantener la cardinalidad razonable.
 */

typedef struct {
    char interface[32];
    unsigned long long rx_bytes;
    unsigned long long tx_bytes;
    unsigned long long rx_packets;
    unsigned long long tx_packets;
    unsigned long long rx_errors;
    unsigned long long tx_errors;
} network_snapshot_entry_t;

typedef struct {
    network_snapshot_entry_t interfaces[OBS_MAX_INTERFACES];
    size_t count;
} network_snapshot_t;

typedef struct {
    char interface[32];
    double rx_bytes_per_sec;
    double tx_bytes_per_sec;
    double rx_packets_per_sec;
    double tx_packets_per_sec;
    unsigned long long rx_errors_total;
    unsigned long long tx_errors_total;
} network_rate_entry_t;

typedef struct {
    network_rate_entry_t interfaces[OBS_MAX_INTERFACES];
    size_t count;
    bool valid;
} network_metrics_t;

typedef struct {
    network_snapshot_t previous;
    bool has_previous;
} network_collector_t;

void network_collector_init(network_collector_t *collector);
obs_status_t network_collect(network_collector_t *collector, double elapsed_secs,
                              network_metrics_t *out);

/* Parsea una linea de /proc/net/dev ("iface: rx... tx..."). Expuesto
 * para tests. */
obs_status_t network_parse_line(const char *line, network_snapshot_entry_t *out);

#endif /* OBSERVER_COLLECTOR_NETWORK_H */
