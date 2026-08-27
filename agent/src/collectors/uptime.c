#include "collectors/uptime.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>

#define COMPONENT "collector.uptime"
#define PROC_UPTIME_PATH "/proc/uptime"

obs_status_t uptime_parse(const char *content, uptime_metrics_t *out)
{
    if (!content || !out) {
        return OBS_ERR_INVALID_ARG;
    }

    /* Formato: "<uptime_segundos> <idle_segundos_acumulados_todos_los_cpu>" */
    double uptime_secs = 0.0;
    if (sscanf(content, "%lf", &uptime_secs) != 1 || uptime_secs < 0) {
        return OBS_ERR_PARSE;
    }

    out->uptime_secs = (unsigned long long)uptime_secs;
    return OBS_OK;
}

obs_status_t uptime_collect(uptime_metrics_t *out)
{
    if (!out) {
        return OBS_ERR_INVALID_ARG;
    }

    FILE *fp = fopen(PROC_UPTIME_PATH, "r");
    if (!fp) {
        LOG_ERROR_(COMPONENT, "failed to open '%s'", PROC_UPTIME_PATH);
        return OBS_ERR_IO;
    }

    char buffer[OBS_MAX_LINE];
    size_t read_len = fread(buffer, 1, sizeof(buffer) - 1, fp);
    buffer[read_len] = '\0';
    fclose(fp);

    if (read_len == 0) {
        return OBS_ERR_IO;
    }

    return uptime_parse(buffer, out);
}
