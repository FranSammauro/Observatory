#include "protocol.h"

#include <stdio.h>
#include <string.h>
#include <stdarg.h>

/*
 * Helper: hace snprintf hacia (buffer + *offset), avanza *offset, y
 * detecta overflow (snprintf trunca pero devuelve el tamano que hubiera
 * escrito, asi que lo usamos para detectar que no entro).
 */
static obs_status_t append(char *buffer, size_t buffer_size, size_t *offset,
                            const char *fmt, ...)
{
    if (*offset >= buffer_size) {
        return OBS_ERR_OVERFLOW;
    }

    va_list args;
    va_start(args, fmt);
    int written = vsnprintf(buffer + *offset, buffer_size - *offset, fmt, args);
    va_end(args);

    if (written < 0 || (size_t)written >= buffer_size - *offset) {
        return OBS_ERR_OVERFLOW;
    }

    *offset += (size_t)written;
    return OBS_OK;
}

obs_status_t protocol_serialize_sample(const obs_sample_t *sample,
                                        char *buffer,
                                        size_t buffer_size)
{
    if (!sample || !buffer || buffer_size == 0) {
        return OBS_ERR_INVALID_ARG;
    }

    size_t offset = 0;
    obs_status_t status;

    status = append(buffer, buffer_size, &offset,
        "{\"protocol_version\":%d,\"agent_id\":\"%s\",\"timestamp\":%llu,\"metrics\":{",
        PROTOCOL_VERSION, sample->agent_id ? sample->agent_id : "", sample->timestamp_unix);
    if (status != OBS_OK) return status;

    bool wrote_any = false;

    if (sample->cpu.valid) {
        status = append(buffer, buffer_size, &offset,
            "\"system.cpu.utilization\":%.4f,"
            "\"system.cpu.user\":%.4f,"
            "\"system.cpu.system\":%.4f,"
            "\"system.cpu.iowait\":%.4f",
            sample->cpu.utilization,
            sample->cpu.user_ratio,
            sample->cpu.system_ratio,
            sample->cpu.iowait_ratio);
        if (status != OBS_OK) return status;
        wrote_any = true;
    }

    if (sample->memory.mem_total_kb > 0) {
        if (wrote_any) {
            status = append(buffer, buffer_size, &offset, ",");
            if (status != OBS_OK) return status;
        }
        status = append(buffer, buffer_size, &offset,
            "\"system.memory.utilization\":%.4f,"
            "\"system.memory.total\":%llu,"
            "\"system.memory.available\":%llu,"
            "\"system.swap.utilization\":%.4f",
            sample->memory.mem_utilization,
            sample->memory.mem_total_kb * 1024ULL,
            sample->memory.mem_available_kb * 1024ULL,
            sample->memory.swap_utilization);
        if (status != OBS_OK) return status;
        wrote_any = true;
    }

    status = append(buffer, buffer_size, &offset, "}}");
    if (status != OBS_OK) return status;

    return OBS_OK;
}
