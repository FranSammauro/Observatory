#include "retry.h"

#include <stdio.h>

static int g_failures = 0;

#define CHECK(cond, msg) do { \
    if (!(cond)) { \
        fprintf(stderr, "FAIL: %s (%s:%d)\n", msg, __FILE__, __LINE__); \
        g_failures++; \
    } \
} while (0)

static void test_exponential_growth_before_cap(void)
{
    retry_policy_t policy;
    retry_policy_init(&policy, 1, 30, 42);

    /* Sin jitter no podemos predecir el valor exacto, pero si el rango:
     * attempt 0 -> [1, 1] (jitter_range = 1/4 = 0, sin jitter)
     * attempt 1 -> delay base 2, jitter en [0, 0] (2/4=0)
     * attempt 2 -> delay base 4, jitter en [0,1)
     * attempt 3 -> delay base 8, jitter en [0,2)
     */
    unsigned int d0 = retry_next_delay_secs(&policy, 0);
    unsigned int d1 = retry_next_delay_secs(&policy, 1);
    unsigned int d2 = retry_next_delay_secs(&policy, 2);
    unsigned int d3 = retry_next_delay_secs(&policy, 3);

    CHECK(d0 == 1, "attempt 0 -> 1s (no room for jitter)");
    CHECK(d1 == 2, "attempt 1 -> 2s (no room for jitter)");
    CHECK(d2 >= 4 && d2 < 5, "attempt 2 -> base 4s + jitter < 1s");
    CHECK(d3 >= 8 && d3 < 10, "attempt 3 -> base 8s + jitter < 2s");
}

static void test_caps_at_max_interval(void)
{
    retry_policy_t policy;
    retry_policy_init(&policy, 1, 30, 42);

    /* attempts altos deben quedar acotados cerca de max_interval_secs
     * (30) + hasta 25% de jitter = 37.5, nunca menos de 30. */
    unsigned int d = retry_next_delay_secs(&policy, 20);
    CHECK(d >= 30 && d <= 38, "high attempt count caps near max_interval_secs");
}

static void test_deterministic_with_same_seed(void)
{
    retry_policy_t policy_a, policy_b;
    retry_policy_init(&policy_a, 1, 30, 999);
    retry_policy_init(&policy_b, 1, 30, 999);

    for (unsigned int attempt = 0; attempt < 10; attempt++) {
        unsigned int da = retry_next_delay_secs(&policy_a, attempt);
        unsigned int db = retry_next_delay_secs(&policy_b, attempt);
        CHECK(da == db, "same seed should produce same sequence (reproducible tests)");
    }
}

static void test_zero_seed_does_not_break_prng(void)
{
    retry_policy_t policy;
    retry_policy_init(&policy, 1, 30, 0);

    /* xorshift32 no puede arrancar en 0 - el init debe evitarlo
     * internamente. Si esto no se manejara, todos los delays serian
     * identicos al base (jitter siempre 0), lo cual igual pasaria el
     * test de abajo por accidente, asi que en cambio verificamos que
     * no haya crash / infinite loop y que el resultado sea razonable. */
    unsigned int d = retry_next_delay_secs(&policy, 3);
    CHECK(d >= 8 && d < 10, "zero seed still produces a sane delay");
}

int main(void)
{
    test_exponential_growth_before_cap();
    test_caps_at_max_interval();
    test_deterministic_with_same_seed();
    test_zero_seed_does_not_break_prng();

    if (g_failures == 0) {
        printf("test_retry: all tests passed\n");
        return 0;
    }
    fprintf(stderr, "test_retry: %d failure(s)\n", g_failures);
    return 1;
}
