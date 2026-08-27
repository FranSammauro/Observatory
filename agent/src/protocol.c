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

/* Escapa comillas y backslashes minimamente - suficiente para los
 * valores que efectivamente puede tener un device/mountpoint/interfaz
 * en Linux (no se esperan comillas ahi, pero preferimos no confiar
 * ciegamente en el input del kernel). */
static obs_status_t append_json_string(char *buffer, size_t buffer_size, size_t *offset,
                                        const char *value)
{
    obs_status_t status = append(buffer, buffer_size, offset, "\"");
    if (status != OBS_OK) return status;

    for (const char *c = value; *c; c++) {
        if (*c == '"' || *c == '\\') {
            status = append(buffer, buffer_size, offset, "\\%c", *c);
        } else if ((unsigned char)*c < 0x20) {
            status = append(buffer, buffer_size, offset, " ");
        } else {
            status = append(buffer, buffer_size, offset, "%c", *c);
        }
        if (status != OBS_OK) return status;
    }

    return append(buffer, buffer_size, offset, "\"");
}

static obs_status_t serialize_cpu(const cpu_metrics_t *cpu, char *buffer, size_t buffer_size,
                                   size_t *offset, bool *wrote_any)
{
    if (!cpu->valid) {
        return OBS_OK;
    }
    if (*wrote_any) {
        obs_status_t s = append(buffer, buffer_size, offset, ",");
        if (s != OBS_OK) return s;
    }
    obs_status_t status = append(buffer, buffer_size, offset,
        "\"system.cpu.utilization\":%.4f,"
        "\"system.cpu.user\":%.4f,"
        "\"system.cpu.system\":%.4f,"
        "\"system.cpu.iowait\":%.4f",
        cpu->utilization, cpu->user_ratio, cpu->system_ratio, cpu->iowait_ratio);
    if (status == OBS_OK) *wrote_any = true;
    return status;
}

static obs_status_t serialize_memory(const memory_metrics_t *mem, char *buffer, size_t buffer_size,
                                      size_t *offset, bool *wrote_any)
{
    if (mem->mem_total_kb == 0) {
        return OBS_OK;
    }
    if (*wrote_any) {
        obs_status_t s = append(buffer, buffer_size, offset, ",");
        if (s != OBS_OK) return s;
    }
    obs_status_t status = append(buffer, buffer_size, offset,
        "\"system.memory.utilization\":%.4f,"
        "\"system.memory.total\":%llu,"
        "\"system.memory.available\":%llu,"
        "\"system.swap.utilization\":%.4f",
        mem->mem_utilization,
        mem->mem_total_kb * 1024ULL,
        mem->mem_available_kb * 1024ULL,
        mem->swap_utilization);
    if (status == OBS_OK) *wrote_any = true;
    return status;
}

static obs_status_t serialize_uptime(const uptime_metrics_t *up, char *buffer, size_t buffer_size,
                                      size_t *offset, bool *wrote_any)
{
    if (*wrote_any) {
        obs_status_t s = append(buffer, buffer_size, offset, ",");
        if (s != OBS_OK) return s;
    }
    obs_status_t status = append(buffer, buffer_size, offset,
        "\"system.uptime\":%llu", up->uptime_secs);
    if (status == OBS_OK) *wrote_any = true;
    return status;
}

static obs_status_t serialize_process(const process_metrics_t *proc, char *buffer, size_t buffer_size,
                                       size_t *offset, bool *wrote_any)
{
    if (*wrote_any) {
        obs_status_t s = append(buffer, buffer_size, offset, ",");
        if (s != OBS_OK) return s;
    }
    obs_status_t status = append(buffer, buffer_size, offset,
        "\"system.process.count\":%u,"
        "\"system.process.running\":%u,"
        "\"system.process.sleeping\":%u,"
        "\"system.process.stopped\":%u,"
        "\"system.process.zombie\":%u",
        proc->total, proc->running, proc->sleeping, proc->stopped, proc->zombie);
    if (status == OBS_OK) *wrote_any = true;
    return status;
}

static obs_status_t serialize_temperature(const temperature_metrics_t *temp, char *buffer,
                                           size_t buffer_size, size_t *offset, bool *wrote_any)
{
    if (!temp->available) {
        return OBS_OK; /* metrica opcional - ausencia no es error, se omite */
    }
    if (*wrote_any) {
        obs_status_t s = append(buffer, buffer_size, offset, ",");
        if (s != OBS_OK) return s;
    }
    obs_status_t status = append(buffer, buffer_size, offset,
        "\"system.temperature\":%.1f", temp->celsius);
    if (status == OBS_OK) *wrote_any = true;
    return status;
}

/* disk/network/filesystem tienen arrays de entradas -> no encajan en
 * el modelo flat "metrics": {...} de los escalares de arriba. Se
 * serializan como arrays separados dentro del mismo objeto top-level. */

static obs_status_t serialize_disk_array(const disk_metrics_t *disk, char *buffer,
                                          size_t buffer_size, size_t *offset)
{
    if (!disk->valid || disk->count == 0) {
        return append(buffer, buffer_size, offset, "[]");
    }

    obs_status_t status = append(buffer, buffer_size, offset, "[");
    if (status != OBS_OK) return status;

    for (size_t i = 0; i < disk->count; i++) {
        const disk_rate_entry_t *d = &disk->disks[i];
        if (i > 0) {
            status = append(buffer, buffer_size, offset, ",");
            if (status != OBS_OK) return status;
        }
        status = append(buffer, buffer_size, offset, "{\"device\":");
        if (status != OBS_OK) return status;
        status = append_json_string(buffer, buffer_size, offset, d->device);
        if (status != OBS_OK) return status;
        status = append(buffer, buffer_size, offset,
            ",\"read_bytes_per_sec\":%.2f,\"write_bytes_per_sec\":%.2f,"
            "\"read_ops_per_sec\":%.2f,\"write_ops_per_sec\":%.2f}",
            d->read_bytes_per_sec, d->write_bytes_per_sec,
            d->read_ops_per_sec, d->write_ops_per_sec);
        if (status != OBS_OK) return status;
    }

    return append(buffer, buffer_size, offset, "]");
}

static obs_status_t serialize_network_array(const network_metrics_t *net, char *buffer,
                                             size_t buffer_size, size_t *offset)
{
    if (!net->valid || net->count == 0) {
        return append(buffer, buffer_size, offset, "[]");
    }

    obs_status_t status = append(buffer, buffer_size, offset, "[");
    if (status != OBS_OK) return status;

    for (size_t i = 0; i < net->count; i++) {
        const network_rate_entry_t *n = &net->interfaces[i];
        if (i > 0) {
            status = append(buffer, buffer_size, offset, ",");
            if (status != OBS_OK) return status;
        }
        status = append(buffer, buffer_size, offset, "{\"interface\":");
        if (status != OBS_OK) return status;
        status = append_json_string(buffer, buffer_size, offset, n->interface);
        if (status != OBS_OK) return status;
        status = append(buffer, buffer_size, offset,
            ",\"rx_bytes_per_sec\":%.2f,\"tx_bytes_per_sec\":%.2f,"
            "\"rx_packets_per_sec\":%.2f,\"tx_packets_per_sec\":%.2f,"
            "\"rx_errors_total\":%llu,\"tx_errors_total\":%llu}",
            n->rx_bytes_per_sec, n->tx_bytes_per_sec,
            n->rx_packets_per_sec, n->tx_packets_per_sec,
            n->rx_errors_total, n->tx_errors_total);
        if (status != OBS_OK) return status;
    }

    return append(buffer, buffer_size, offset, "]");
}

static obs_status_t serialize_filesystem_array(const filesystem_metrics_t *fs, char *buffer,
                                                size_t buffer_size, size_t *offset)
{
    if (fs->count == 0) {
        return append(buffer, buffer_size, offset, "[]");
    }

    obs_status_t status = append(buffer, buffer_size, offset, "[");
    if (status != OBS_OK) return status;

    for (size_t i = 0; i < fs->count; i++) {
        const filesystem_entry_t *f = &fs->entries[i];
        if (i > 0) {
            status = append(buffer, buffer_size, offset, ",");
            if (status != OBS_OK) return status;
        }
        status = append(buffer, buffer_size, offset, "{\"device\":");
        if (status != OBS_OK) return status;
        status = append_json_string(buffer, buffer_size, offset, f->device);
        if (status != OBS_OK) return status;
        status = append(buffer, buffer_size, offset, ",\"mountpoint\":");
        if (status != OBS_OK) return status;
        status = append_json_string(buffer, buffer_size, offset, f->mountpoint);
        if (status != OBS_OK) return status;
        status = append(buffer, buffer_size, offset, ",\"fs_type\":");
        if (status != OBS_OK) return status;
        status = append_json_string(buffer, buffer_size, offset, f->fs_type);
        if (status != OBS_OK) return status;
        status = append(buffer, buffer_size, offset,
            ",\"total_bytes\":%llu,\"available_bytes\":%llu,\"utilization\":%.4f}",
            f->total_bytes, f->available_bytes, f->utilization);
        if (status != OBS_OK) return status;
    }

    return append(buffer, buffer_size, offset, "]");
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
        "{\"protocol_version\":%d,\"agent_id\":",
        PROTOCOL_VERSION);
    if (status != OBS_OK) return status;

    status = append_json_string(buffer, buffer_size, &offset,
        sample->agent_id ? sample->agent_id : "");
    if (status != OBS_OK) return status;

    status = append(buffer, buffer_size, &offset,
        ",\"timestamp\":%llu,\"metrics\":{", sample->timestamp_unix);
    if (status != OBS_OK) return status;

    bool wrote_any = false;
    status = serialize_cpu(&sample->cpu, buffer, buffer_size, &offset, &wrote_any);
    if (status != OBS_OK) return status;
    status = serialize_memory(&sample->memory, buffer, buffer_size, &offset, &wrote_any);
    if (status != OBS_OK) return status;
    status = serialize_uptime(&sample->uptime, buffer, buffer_size, &offset, &wrote_any);
    if (status != OBS_OK) return status;
    status = serialize_process(&sample->process, buffer, buffer_size, &offset, &wrote_any);
    if (status != OBS_OK) return status;
    status = serialize_temperature(&sample->temperature, buffer, buffer_size, &offset, &wrote_any);
    if (status != OBS_OK) return status;

    status = append(buffer, buffer_size, &offset, "},\"disk\":");
    if (status != OBS_OK) return status;
    status = serialize_disk_array(&sample->disk, buffer, buffer_size, &offset);
    if (status != OBS_OK) return status;

    status = append(buffer, buffer_size, &offset, ",\"network\":");
    if (status != OBS_OK) return status;
    status = serialize_network_array(&sample->network, buffer, buffer_size, &offset);
    if (status != OBS_OK) return status;

    status = append(buffer, buffer_size, &offset, ",\"filesystem\":");
    if (status != OBS_OK) return status;
    status = serialize_filesystem_array(&sample->filesystem, buffer, buffer_size, &offset);
    if (status != OBS_OK) return status;

    return append(buffer, buffer_size, &offset, "}");
}

obs_status_t protocol_serialize_heartbeat(const char *agent_id,
                                           unsigned long long timestamp_unix,
                                           char *buffer,
                                           size_t buffer_size)
{
    if (!agent_id || !buffer || buffer_size == 0) {
        return OBS_ERR_INVALID_ARG;
    }

    size_t offset = 0;
    obs_status_t status = append(buffer, buffer_size, &offset,
        "{\"protocol_version\":%d,\"agent_id\":", PROTOCOL_VERSION);
    if (status != OBS_OK) return status;

    status = append_json_string(buffer, buffer_size, &offset, agent_id);
    if (status != OBS_OK) return status;

    return append(buffer, buffer_size, &offset, ",\"timestamp\":%llu}", timestamp_unix);
}
