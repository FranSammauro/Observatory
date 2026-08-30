#ifndef OBSERVER_PROTOCOL_H
#define OBSERVER_PROTOCOL_H

#include "agent.h"
#include "collectors/cpu.h"
#include "collectors/memory.h"
#include "collectors/disk.h"
#include "collectors/network.h"
#include "collectors/filesystem.h"
#include "collectors/uptime.h"
#include "collectors/process.h"
#include "collectors/temperature.h"

/*
 * Serializacion del payload JSON hacia el collector. El escritor es
 * manual para no introducir dependencias externas; el formato del payload
 * es estable y conocido, por lo que no es necesario un encoder generico.
 *
 * Dos payloads distintos: el sample completo (metricas) y el heartbeat,
 * que es un canal independiente de menor tamano.
 */

typedef struct {
    const char *agent_id;
    unsigned long long timestamp_unix;

    cpu_metrics_t cpu;
    memory_metrics_t memory;
    disk_metrics_t disk;
    network_metrics_t network;
    filesystem_metrics_t filesystem;
    uptime_metrics_t uptime;
    process_metrics_t process;
    temperature_metrics_t temperature;
} obs_sample_t;

obs_status_t protocol_serialize_sample(const obs_sample_t *sample,
                                        char *buffer,
                                        size_t buffer_size);

obs_status_t protocol_serialize_heartbeat(const char *agent_id,
                                           unsigned long long timestamp_unix,
                                           char *buffer,
                                           size_t buffer_size);

#endif /* OBSERVER_PROTOCOL_H */
