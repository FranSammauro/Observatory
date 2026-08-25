#ifndef OBSERVER_PROTOCOL_H
#define OBSERVER_PROTOCOL_H

#include "agent.h"
#include "collectors/cpu.h"
#include "collectors/memory.h"

/*
 * Serializacion del payload de ingestion (informe tecnico, seccion 21).
 *
 * Fase 1: solo serializamos a un buffer en memoria. El envio real por
 * HTTPS se implementa en transport.c (Fase 2).
 *
 * Deliberadamente NO usamos una libreria JSON externa: el payload es
 * pequeno y de forma fija, asi que un escritor manual mantiene el
 * binario chico y evita una dependencia mas (informe seccion 6.1).
 */

typedef struct {
    const char *agent_id;
    unsigned long long timestamp_unix;
    cpu_metrics_t cpu;         /* cpu.valid indica si hay dato disponible */
    memory_metrics_t memory;
} obs_sample_t;

/*
 * Escribe el payload JSON del sample en buffer (tamano buffer_size).
 * Devuelve OBS_OK, o OBS_ERR_OVERFLOW si no entra en el buffer.
 */
obs_status_t protocol_serialize_sample(const obs_sample_t *sample,
                                        char *buffer,
                                        size_t buffer_size);

#endif /* OBSERVER_PROTOCOL_H */
