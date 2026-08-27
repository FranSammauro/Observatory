#include "retry.h"

/*
 * xorshift32: PRNG minimo, sin dependencias, suficiente para jitter
 * (no es criptografico ni necesita serlo - ver identity.c para el uso
 * de /dev/urandom donde si importa la calidad de la entropia).
 */
static uint32_t xorshift32(uint32_t *state)
{
    uint32_t x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    return x;
}

void retry_policy_init(retry_policy_t *policy, unsigned int base_secs,
                        unsigned int max_interval_secs, uint32_t seed)
{
    policy->base_secs = base_secs > 0 ? base_secs : 1;
    policy->max_interval_secs = max_interval_secs > 0 ? max_interval_secs : 30;
    /* xorshift32 no puede arrancar en 0 */
    policy->rng_state = (seed != 0) ? seed : 0xA5A5A5A5u;
}

unsigned int retry_next_delay_secs(retry_policy_t *policy, unsigned int attempt)
{
    unsigned int delay = policy->base_secs;

    /* delay = base * 2^attempt, con clamp para evitar overflow y para
     * respetar max_interval_secs (informe: 1,2,4,8,16,30,30,...). */
    for (unsigned int i = 0; i < attempt; i++) {
        if (delay >= policy->max_interval_secs) {
            delay = policy->max_interval_secs;
            break;
        }
        delay *= 2;
    }

    if (delay > policy->max_interval_secs) {
        delay = policy->max_interval_secs;
    }

    /* Jitter: hasta un 25% del delay, sumado (nunca resta, para no
     * terminar reintentando mas rapido de lo previsto). */
    unsigned int jitter_range = delay / 4;
    unsigned int jitter = 0;
    if (jitter_range > 0) {
        jitter = xorshift32(&policy->rng_state) % jitter_range;
    }

    return delay + jitter;
}
