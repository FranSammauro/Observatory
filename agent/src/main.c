#define _POSIX_C_SOURCE 200809L

#include "agent.h"
#include "config.h"
#include "logging.h"
#include "protocol.h"
#include "transport.h"
#include "retry.h"
#include "identity.h"
#include "collectors/cpu.h"
#include "collectors/memory.h"
#include "collectors/disk.h"
#include "collectors/network.h"
#include "collectors/filesystem.h"
#include "collectors/uptime.h"
#include "collectors/process.h"
#include "collectors/temperature.h"

#include <stdio.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#define COMPONENT "main"

/*
 * Fase 2: agrega transporte HTTP real (transport.c), heartbeat como
 * canal independiente de las metricas (informe seccion 24), retry con
 * backoff+jitter ante fallos de envio (seccion 26), y el resto de
 * collectors (disk, network, filesystem, uptime, process, temperature).
 *
 * Scheduling: se usa CLOCK_MONOTONIC (nunca wall clock, ver informe
 * seccion 57) para decidir cuando toca la proxima muestra de metricas
 * y el proximo heartbeat, de forma independiente entre si.
 *
 * Estrategia de retry (seccion 27, "sin buffer local" para V1): si un
 * envio falla, NO se bloquea el loop reintentando la misma muestra -
 * se aplica backoff antes del proximo intento, y en ese proximo intento
 * se envia una muestra fresca (el dato viejo ya no es tan relevante).
 * Esto evita que el agent se cuelgue reintentando indefinidamente una
 * unica muestra mientras el resto del sistema sigue funcionando.
 */

typedef struct {
    struct timespec next_due;
    unsigned int consecutive_failures;
    retry_policy_t retry;
} channel_schedule_t;

static double monotonic_now(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

static void schedule_init(channel_schedule_t *sched, uint32_t rng_seed)
{
    sched->consecutive_failures = 0;
    retry_policy_init(&sched->retry, 1, 30, rng_seed);
    clock_gettime(CLOCK_MONOTONIC, &sched->next_due);
}

static bool schedule_is_due(const channel_schedule_t *sched)
{
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    if (now.tv_sec != sched->next_due.tv_sec) {
        return now.tv_sec > sched->next_due.tv_sec;
    }
    return now.tv_nsec >= sched->next_due.tv_nsec;
}

static void schedule_set_next(channel_schedule_t *sched, unsigned int delay_secs)
{
    clock_gettime(CLOCK_MONOTONIC, &sched->next_due);
    sched->next_due.tv_sec += delay_secs;
}

static void schedule_on_success(channel_schedule_t *sched, unsigned int normal_interval_secs)
{
    sched->consecutive_failures = 0;
    schedule_set_next(sched, normal_interval_secs);
}

static void schedule_on_failure(channel_schedule_t *sched)
{
    unsigned int delay = retry_next_delay_secs(&sched->retry, sched->consecutive_failures);
    sched->consecutive_failures++;
    LOG_WARN_(COMPONENT, "send failed (%u consecutive failures), retrying in %us",
               sched->consecutive_failures, delay);
    schedule_set_next(sched, delay);
}

static void collect_sample(obs_sample_t *sample,
                            cpu_collector_t *cpu_collector,
                            disk_collector_t *disk_collector,
                            network_collector_t *network_collector,
                            double elapsed_secs,
                            const char *agent_id)
{
    memset(sample, 0, sizeof(*sample));
    sample->agent_id = agent_id;
    sample->timestamp_unix = (unsigned long long)time(NULL);

    obs_status_t status;

    status = cpu_collect(cpu_collector, &sample->cpu);
    if (status != OBS_OK) {
        LOG_WARN_(COMPONENT, "cpu collection failed: %s", obs_status_str(status));
    }

    status = memory_collect(&sample->memory);
    if (status != OBS_OK) {
        LOG_WARN_(COMPONENT, "memory collection failed: %s", obs_status_str(status));
    }

    status = disk_collect(disk_collector, elapsed_secs, &sample->disk);
    if (status != OBS_OK) {
        LOG_WARN_(COMPONENT, "disk collection failed: %s", obs_status_str(status));
    }

    status = network_collect(network_collector, elapsed_secs, &sample->network);
    if (status != OBS_OK) {
        LOG_WARN_(COMPONENT, "network collection failed: %s", obs_status_str(status));
    }

    status = filesystem_collect(&sample->filesystem);
    if (status != OBS_OK) {
        LOG_WARN_(COMPONENT, "filesystem collection failed: %s", obs_status_str(status));
    }

    status = uptime_collect(&sample->uptime);
    if (status != OBS_OK) {
        LOG_WARN_(COMPONENT, "uptime collection failed: %s", obs_status_str(status));
    }

    status = process_collect(&sample->process);
    if (status != OBS_OK) {
        LOG_WARN_(COMPONENT, "process collection failed: %s", obs_status_str(status));
    }

    status = temperature_collect(&sample->temperature);
    if (status != OBS_OK) {
        LOG_WARN_(COMPONENT, "temperature collection failed: %s", obs_status_str(status));
    }
}

static bool send_json(const transport_config_t *transport_config,
                       const char *collector_url,
                       const char *path,
                       const char *token,
                       const char *json)
{
    transport_result_t result;
    obs_status_t status = transport_post(transport_config, collector_url, path, token, json, &result);

    if (status != OBS_OK) {
        LOG_WARN_(COMPONENT, "%s: transport error: %s", path, obs_status_str(status));
        return false;
    }

    if (!result.got_response || result.status_code < 200 || result.status_code >= 300) {
        LOG_WARN_(COMPONENT, "%s: collector responded with status %d", path, result.status_code);
        return false;
    }

    return true;
}

int main(int argc, char **argv)
{
    obs_config_t config;
    config_set_defaults(&config);

    const char *config_path = (argc > 1) ? argv[1] : "/etc/observer/agent.conf";
    config_load(config_path, &config);

    log_init(config.log_level);

    char agent_id[OBS_MAX_LINE];
    obs_status_t id_status = identity_resolve(config.agent_id_path, agent_id, sizeof(agent_id));
    if (id_status != OBS_OK) {
        LOG_ERROR_(COMPONENT, "could not resolve agent identity: %s", obs_status_str(id_status));
        return 1;
    }

    LOG_INFO_(COMPONENT, "observer-agent %s starting (agent_id=%s, protocol_version=%d)",
               AGENT_VERSION, agent_id, PROTOCOL_VERSION);
    LOG_INFO_(COMPONENT, "collector_url=%s metrics_interval=%us heartbeat_interval=%us",
               config.collector_url, config.metrics_interval_secs, config.heartbeat_interval_secs);

    cpu_collector_t cpu_collector;
    cpu_collector_init(&cpu_collector);
    disk_collector_t disk_collector;
    disk_collector_init(&disk_collector);
    network_collector_t network_collector;
    network_collector_init(&network_collector);

    transport_config_t transport_config = {
        .connect_timeout_secs = config.connect_timeout_secs,
        .write_timeout_secs = config.write_timeout_secs,
        .read_timeout_secs = config.read_timeout_secs,
    };

    channel_schedule_t metrics_schedule;
    channel_schedule_t heartbeat_schedule;
    schedule_init(&metrics_schedule, (uint32_t)time(NULL) ^ 0x1234u);
    schedule_init(&heartbeat_schedule, (uint32_t)time(NULL) ^ 0x5678u);

    double last_metrics_monotonic = monotonic_now();

    for (;;) {
        if (schedule_is_due(&heartbeat_schedule)) {
            char hb_json[256];
            obs_status_t serialize_status = protocol_serialize_heartbeat(
                agent_id, (unsigned long long)time(NULL), hb_json, sizeof(hb_json));

            bool ok = false;
            if (serialize_status == OBS_OK) {
                ok = send_json(&transport_config, config.collector_url,
                                "/api/v1/agents/heartbeat", config.agent_token, hb_json);
            } else {
                LOG_ERROR_(COMPONENT, "failed to serialize heartbeat: %s",
                            obs_status_str(serialize_status));
            }

            if (ok) {
                schedule_on_success(&heartbeat_schedule, config.heartbeat_interval_secs);
            } else {
                schedule_on_failure(&heartbeat_schedule);
            }
        }

        if (schedule_is_due(&metrics_schedule)) {
            double now = monotonic_now();
            double elapsed = now - last_metrics_monotonic;
            last_metrics_monotonic = now;

            obs_sample_t sample;
            collect_sample(&sample, &cpu_collector, &disk_collector, &network_collector,
                            elapsed, agent_id);

            char json_buffer[OBS_MAX_JSON_BUFFER];
            obs_status_t serialize_status =
                protocol_serialize_sample(&sample, json_buffer, sizeof(json_buffer));

            bool ok = false;
            if (serialize_status == OBS_OK) {
                ok = send_json(&transport_config, config.collector_url,
                                "/api/v1/metrics", config.agent_token, json_buffer);
            } else {
                LOG_ERROR_(COMPONENT, "failed to serialize sample: %s",
                            obs_status_str(serialize_status));
            }

            if (ok) {
                schedule_on_success(&metrics_schedule, config.metrics_interval_secs);
            } else {
                schedule_on_failure(&metrics_schedule);
            }
        }

        /* Tick corto: suficiente resolucion para heartbeat (tipicamente
         * 5s) sin ocupar CPU en un busy loop. */
        struct timespec tick = { .tv_sec = 0, .tv_nsec = 200L * 1000L * 1000L };
        nanosleep(&tick, NULL);
    }

    return 0;
}
