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
 * Serializacion de los payloads de ingestion (informe tecnico, seccion
 * 21 y 24).
 *
 * Deliberadamente NO usamos una libreria JSON externa: el payload es de
 * forma conocida, asi que un escritor manual mantiene el binario chico y
 * evita una dependencia mas (informe seccion 6.1).
 *
 * Dos payloads distintos, en linea con el heartbeat como canal
 * independiente de las metricas (informe seccion 24):
 *   - obs_sample_t     -> POST /api/v1/metrics
 *   - heartbeat         -> POST /api/v1/agents/heartbeat (mas liviano)
 */

typedef struct {
    const char *agent_id;
    unsigned long long timestamp_unix;

    cpu_metrics_t cpu;                 /* cpu.valid indica si hay dato disponible */
    memory_metrics_t memory;
    disk_metrics_t disk;               /* disk.valid indica si hay dato disponible */
    network_metrics_t network;         /* network.valid indica si hay dato disponible */
    filesystem_metrics_t filesystem;
    uptime_metrics_t uptime;
    process_metrics_t process;
    temperature_metrics_t temperature; /* temperature.available puede ser false */
} obs_sample_t;

/*
 * Escribe el payload JSON del sample en buffer (tamano buffer_size).
 * Devuelve OBS_OK, o OBS_ERR_OVERFLOW si no entra en el buffer.
 */
obs_status_t protocol_serialize_sample(const obs_sample_t *sample,
                                        char *buffer,
                                        size_t buffer_size);

/*
 * Escribe el payload JSON del heartbeat (mucho mas liviano que el
 * sample completo - solo identidad + timestamp).
 */
obs_status_t protocol_serialize_heartbeat(const char *agent_id,
                                           unsigned long long timestamp_unix,
                                           char *buffer,
                                           size_t buffer_size);

#endif /* OBSERVER_PROTOCOL_H */
