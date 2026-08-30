#ifndef OBSERVER_RETRY_H
#define OBSERVER_RETRY_H

#include "agent.h"

/*
 * Backoff exponencial con jitter para reintentos de envio.
 * La secuencia base crece como base * 2^attempt hasta max_interval_secs,
 * donde permanece constante. El jitter (hasta el 25% del intervalo base)
 * evita la tormenta de reintentos cuando varios agentes fallan al mismo
 * tiempo.
 */

typedef struct {
    unsigned int base_secs;
    unsigned int max_interval_secs;
    uint32_t rng_state;
} retry_policy_t;

void retry_policy_init(retry_policy_t *policy, unsigned int base_secs,
                        unsigned int max_interval_secs, uint32_t seed);

unsigned int retry_next_delay_secs(retry_policy_t *policy, unsigned int attempt);

#endif /* OBSERVER_RETRY_H */
