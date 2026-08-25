#include "collectors/memory.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>
#include <stdlib.h>

#define COMPONENT "collector.memory"
#define PROC_MEMINFO_PATH "/proc/meminfo"

static bool parse_kb_line(const char *line, const char *key, unsigned long long *out)
{
    size_t key_len = strlen(key);

    if (strncmp(line, key, key_len) != 0) {
        return false;
    }

    /* Formato: "Key:            123456 kB" */
    const char *rest = line + key_len;
    while (*rest == ':' || *rest == ' ' || *rest == '\t') {
        rest++;
    }

    char *endptr = NULL;
    unsigned long long value = strtoull(rest, &endptr, 10);
    if (endptr == rest) {
        return false;
    }

    *out = value;
    return true;
}

obs_status_t memory_parse_meminfo(const char *content, memory_metrics_t *out)
{
    if (!content || !out) {
        return OBS_ERR_INVALID_ARG;
    }

    memset(out, 0, sizeof(*out));

    bool has_total = false;
    bool has_available = false;
    unsigned long long mem_free_kb = 0;
    bool has_free = false;

    char line[OBS_MAX_LINE];
    const char *cursor = content;

    while (*cursor) {
        size_t i = 0;
        while (cursor[i] && cursor[i] != '\n' && i < sizeof(line) - 1) {
            line[i] = cursor[i];
            i++;
        }
        line[i] = '\0';
        cursor += i;
        if (*cursor == '\n') {
            cursor++;
        }

        if (parse_kb_line(line, "MemTotal", &out->mem_total_kb)) {
            has_total = true;
        } else if (parse_kb_line(line, "MemAvailable", &out->mem_available_kb)) {
            has_available = true;
        } else if (parse_kb_line(line, "MemFree", &mem_free_kb)) {
            has_free = true;
        } else if (parse_kb_line(line, "SwapTotal", &out->swap_total_kb)) {
            /* handled */
        } else if (parse_kb_line(line, "SwapFree", &out->swap_free_kb)) {
            /* handled */
        }

        if (i == 0 && *cursor == '\0') {
            break;
        }
    }

    if (!has_total) {
        return OBS_ERR_PARSE;
    }

    /* Kernels muy viejos (o namespaces raros) pueden no exponer
     * MemAvailable. En ese caso, MemFree es una aproximacion peor pero
     * evita que el collector falle por completo (informe seccion 10). */
    if (!has_available) {
        if (!has_free) {
            return OBS_ERR_PARSE;
        }
        LOG_WARN_(COMPONENT, "MemAvailable not present, falling back to MemFree");
        out->mem_available_kb = mem_free_kb;
    }

    if (out->mem_total_kb > 0) {
        unsigned long long used_kb = (out->mem_available_kb <= out->mem_total_kb)
            ? out->mem_total_kb - out->mem_available_kb
            : 0;
        out->mem_utilization = (double)used_kb / (double)out->mem_total_kb;
    }

    if (out->swap_total_kb > 0) {
        unsigned long long swap_used_kb = (out->swap_free_kb <= out->swap_total_kb)
            ? out->swap_total_kb - out->swap_free_kb
            : 0;
        out->swap_utilization = (double)swap_used_kb / (double)out->swap_total_kb;
    } else {
        out->swap_utilization = 0.0;
    }

    return OBS_OK;
}

obs_status_t memory_collect(memory_metrics_t *out)
{
    if (!out) {
        return OBS_ERR_INVALID_ARG;
    }

    FILE *fp = fopen(PROC_MEMINFO_PATH, "r");
    if (!fp) {
        LOG_ERROR_(COMPONENT, "failed to open '%s'", PROC_MEMINFO_PATH);
        return OBS_ERR_IO;
    }

    char buffer[OBS_MAX_JSON_BUFFER];
    size_t total_read = fread(buffer, 1, sizeof(buffer) - 1, fp);
    buffer[total_read] = '\0';
    fclose(fp);

    if (total_read == 0) {
        return OBS_ERR_IO;
    }

    return memory_parse_meminfo(buffer, out);
}
