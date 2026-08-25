#include "collectors/cpu.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>

#define COMPONENT "collector.cpu"
#define PROC_STAT_PATH "/proc/stat"

obs_status_t cpu_parse_line(const char *line, cpu_snapshot_t *out)
{
    if (!line || !out) {
        return OBS_ERR_INVALID_ARG;
    }

    /* Formato esperado:
     * cpu  user nice system idle iowait irq softirq steal [guest] [guest_nice]
     * Los campos guest/guest_nice pueden faltar en kernels viejos; no son
     * necesarios para el calculo de utilizacion (ya estan incluidos en
     * user/nice segun la documentacion de /proc/stat).
     */
    unsigned long long user = 0, nice = 0, system = 0, idle = 0;
    unsigned long long iowait = 0, irq = 0, softirq = 0, steal = 0;

    int matched = sscanf(line,
        "cpu %llu %llu %llu %llu %llu %llu %llu %llu",
        &user, &nice, &system, &idle, &iowait, &irq, &softirq, &steal);

    /* Kernels sin steal/softirq/irq: aceptamos con menos campos, el resto
     * queda en 0 (ya inicializados arriba). Exigimos al menos user..idle. */
    if (matched < 4) {
        return OBS_ERR_PARSE;
    }

    out->user = user;
    out->nice = nice;
    out->system = system;
    out->idle = idle;
    out->iowait = iowait;
    out->irq = irq;
    out->softirq = softirq;
    out->steal = steal;

    return OBS_OK;
}

static obs_status_t read_snapshot_from_file(const char *path, cpu_snapshot_t *out)
{
    FILE *fp = fopen(path, "r");
    if (!fp) {
        LOG_ERROR_(COMPONENT, "failed to open '%s'", path);
        return OBS_ERR_IO;
    }

    char line[OBS_MAX_LINE];
    obs_status_t status = OBS_ERR_PARSE;

    if (fgets(line, sizeof(line), fp)) {
        status = cpu_parse_line(line, out);
    }

    fclose(fp);
    return status;
}

void cpu_collector_init(cpu_collector_t *collector)
{
    memset(collector, 0, sizeof(*collector));
    collector->has_previous = false;
}

static unsigned long long snapshot_total(const cpu_snapshot_t *s)
{
    return s->user + s->nice + s->system + s->idle
         + s->iowait + s->irq + s->softirq + s->steal;
}

static unsigned long long snapshot_idle(const cpu_snapshot_t *s)
{
    return s->idle + s->iowait;
}

obs_status_t cpu_collect(cpu_collector_t *collector, cpu_metrics_t *out)
{
    if (!collector || !out) {
        return OBS_ERR_INVALID_ARG;
    }

    memset(out, 0, sizeof(*out));

    cpu_snapshot_t current;
    obs_status_t status = read_snapshot_from_file(PROC_STAT_PATH, &current);
    if (status != OBS_OK) {
        return status;
    }

    if (!collector->has_previous) {
        collector->previous = current;
        collector->has_previous = true;
        out->valid = false;
        return OBS_OK;
    }

    unsigned long long total_prev = snapshot_total(&collector->previous);
    unsigned long long total_curr = snapshot_total(&current);
    unsigned long long idle_prev = snapshot_idle(&collector->previous);
    unsigned long long idle_curr = snapshot_idle(&current);

    /* Contadores de counter reset (reboot, overflow, kernel raro):
     * si el delta seria negativo, descartamos esta muestra en vez de
     * reportar un valor invalido (ver informe seccion 76, Definition of
     * Done: "Handles counter reset"). */
    if (total_curr < total_prev || idle_curr < idle_prev) {
        LOG_WARN_(COMPONENT, "counter reset detected, discarding sample");
        collector->previous = current;
        out->valid = false;
        return OBS_OK;
    }

    unsigned long long total_delta = total_curr - total_prev;
    unsigned long long idle_delta = idle_curr - idle_prev;

    if (total_delta == 0) {
        /* Llamadas demasiado seguidas: no hay suficiente resolucion. */
        out->valid = false;
        collector->previous = current;
        return OBS_OK;
    }

    out->utilization = 1.0 - ((double)idle_delta / (double)total_delta);
    out->user_ratio = (double)(current.user - collector->previous.user) / (double)total_delta;
    out->system_ratio = (double)(current.system - collector->previous.system) / (double)total_delta;
    out->iowait_ratio = (double)(current.iowait - collector->previous.iowait) / (double)total_delta;
    out->valid = true;

    collector->previous = current;

    return OBS_OK;
}
