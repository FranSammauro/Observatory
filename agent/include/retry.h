#ifndef OBSERVER_RETRY_H
#define OBSERVER_RETRY_H

#include "agent.h"

/*
 * Backoff exponencial con jitter (informe tecnico, seccion 26).
 *
 * Secuencia base sin jitter: 1s, 2s, 4s, 8s, 16s, 30s, 30s, ...
 * (crece hasta max_interval_secs y despues se mantiene constante).
 *
 * El jitter evita que, si muchos agentes pierden conectividad al mismo
 * tiempo (p.ej. el Collector se reinicia), todos reintenten exactamente
 * en el mismo instante ("thundering herd").
 */

typedef struct {
    unsigned int base_secs;      /* delay del primer intento (tipicamente 1) */
    unsigned int max_interval_secs; /* techo del backoff (tipicamente 30) */
    uint32_t rng_state;           /* estado del PRNG interno (xorshift32) */
} retry_policy_t;

void retry_policy_init(retry_policy_t *policy, unsigned int base_secs,
                        unsigned int max_interval_secs, uint32_t seed);

/*
 * Calcula el delay (en segundos) para el intento numero `attempt`
 * (0-indexado: attempt=0 es el primer reintento tras el fallo inicial).
 * Incluye jitter aleatorio en el rango [0, delay_base * 0.25].
 */
unsigned int retry_next_delay_secs(retry_policy_t *policy, unsigned int attempt);

#endif /* OBSERVER_RETRY_H */
