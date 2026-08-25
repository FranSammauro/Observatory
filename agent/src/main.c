#include "agent.h"
#include "config.h"
#include "logging.h"
#include "protocol.h"
#include "collectors/cpu.h"
#include "collectors/memory.h"

#include <stdio.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#define COMPONENT "main"

/*
 * Fase 1: el agent recolecta CPU + memoria localmente y las serializa a
 * JSON en stdout en cada intervalo de metrics_interval_secs, para poder
 * validar collectors y protocolo sin depender aun del Collector remoto.
 *
 * El transporte HTTPS real (transport.c), el heartbeat separado y el
 * resto de collectors (disk, network, filesystem, uptime, procesos) se
 * agregan en la Fase 2.
 */

static const char *resolve_agent_id(const obs_config_t *config, char *out, size_t out_size)
{
    FILE *fp = fopen(config->agent_id_path, "r");
    if (fp) {
        if (fgets(out, (int)out_size, fp)) {
            size_t len = strlen(out);
            while (len > 0 && (out[len - 1] == '\n' || out[len - 1] == '\r')) {
                out[len - 1] = '\0';
                len--;
            }
        }
        fclose(fp);
        if (out[0] != '\0') {
            return out;
        }
    }

    /* Fase 1: fallback simple si no existe /etc/observer/agent-id todavia.
     * La generacion/persistencia real de una identidad unica se completa
     * junto con el registro contra el Collector (Fase 2/3). */
    snprintf(out, out_size, "unidentified-agent");
    LOG_WARN_(COMPONENT,
        "could not read agent id from '%s', using placeholder '%s'",
        config->agent_id_path, out);
    return out;
}

int main(int argc, char **argv)
{
    obs_config_t config;
    config_set_defaults(&config);

    const char *config_path = (argc > 1) ? argv[1] : "/etc/observer/agent.conf";
    config_load(config_path, &config);

    log_init(config.log_level);

    char agent_id[OBS_MAX_LINE];
    resolve_agent_id(&config, agent_id, sizeof(agent_id));

    LOG_INFO_(COMPONENT, "observer-agent %s starting (agent_id=%s, protocol_version=%d)",
               AGENT_VERSION, agent_id, PROTOCOL_VERSION);
    LOG_INFO_(COMPONENT, "metrics_interval=%us heartbeat_interval=%us (heartbeat wiring: fase 2)",
               config.metrics_interval_secs, config.heartbeat_interval_secs);

    cpu_collector_t cpu_collector;
    cpu_collector_init(&cpu_collector);

    for (;;) {
        obs_sample_t sample;
        memset(&sample, 0, sizeof(sample));
        sample.agent_id = agent_id;
        sample.timestamp_unix = (unsigned long long)time(NULL);

        obs_status_t cpu_status = cpu_collect(&cpu_collector, &sample.cpu);
        if (cpu_status != OBS_OK) {
            LOG_WARN_(COMPONENT, "cpu collection failed: %s", obs_status_str(cpu_status));
        }

        obs_status_t mem_status = memory_collect(&sample.memory);
        if (mem_status != OBS_OK) {
            LOG_WARN_(COMPONENT, "memory collection failed: %s", obs_status_str(mem_status));
        }

        char json_buffer[OBS_MAX_JSON_BUFFER];
        obs_status_t serialize_status =
            protocol_serialize_sample(&sample, json_buffer, sizeof(json_buffer));

        if (serialize_status == OBS_OK) {
            /* Fase 1: stdout en vez de transport.c (eso llega en Fase 2). */
            printf("%s\n", json_buffer);
            fflush(stdout);
        } else {
            LOG_ERROR_(COMPONENT, "failed to serialize sample: %s",
                        obs_status_str(serialize_status));
        }

        sleep(config.metrics_interval_secs);
    }

    return 0;
}
