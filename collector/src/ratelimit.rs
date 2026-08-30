/*
 * Rate limiting por cliente: un token bucket por IP
 * para aplanar picos de ingestion y frenar agentes descontrolados o
 * clientes maliciosos que inundan los endpoints de escritura.
 *
 * El nucleo (TokenBucket) es puro y usa un reloj sintetico (`Duration`
 * desde un origen arbitrario) para poder testear aritmetica de tokens sin
 * tiempos reales. `RateLimiter` es la pieza compartida entre requests:
 * un mapa de buckets indexado por clave de cliente con limpieza de
 * buckets inactivos. La regla de permiso de la politica de rate limit
 * vive en `RatePolicy`, tambien pura.
 *
 * Los buckets acumulan tokens hasta `capacity` y se recargan a `rate`
 * tokens por segundo. Un request "cuesta" 1 token; si no alcanza, se
 * rechaza (429). Con rate = 20 y burst(capacity) = 50, un agente que
 * envia cada 10s jamaun agota el bucket, pero una rafaga de 60 samples en
 * el mismo instante ve rechazados los ultimos.
 */

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/* Nucleo puro: credito de tokens en funcion de un reloj sintetico. */
#[derive(Debug, Clone)]
pub struct TokenBucket {
    capacity: f64,
    fill_rate: f64,
    tokens: f64,
    last: Duration,
}

#[derive(Debug, PartialEq)]
pub enum Take {
    Allowed,
    Denied,
}

impl TokenBucket {
    pub fn new(capacity: f64, fill_rate: f64) -> Self {
        debug_assert!(capacity > 0.0);
        debug_assert!(fill_rate >= 0.0);
        Self {
            capacity,
            fill_rate,
            tokens: capacity,
            last: Duration::ZERO,
        }
    }

    /* Reacredita por el tiempo transcurrido desde `last` y descuenta `n`
     * si hay saldo. Siempre avanza `last` a `now`. */
    pub fn take(&mut self, now: Duration, n: f64) -> Take {
        let elapsed = now.saturating_sub(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.fill_rate).min(self.capacity);
        self.last = now;
        if self.tokens >= n {
            self.tokens -= n;
            Take::Allowed
        } else {
            Take::Denied
        }
    }
}

/* Politica: cuantos requests por segundo y que rafaga maxima. */

#[derive(Debug, Clone, Copy)]
pub struct RatePolicy {
    pub rate_per_sec: f64,
    pub burst: f64,
}

impl RatePolicy {
    pub fn enabled(&self) -> bool {
        self.rate_per_sec > 0.0
    }
}

/* Mapa de buckets por clave de cliente, compartido entre requests. */
pub struct RateLimiter {
    inner: Mutex<HashMap<String, Entry>>,
    policy: RatePolicy,
    anchor: std::time::Instant,
}

struct Entry {
    bucket: TokenBucket,
    last_seen: Duration,
}

/* Los buckets inactivos mas viejos que `IDLE_GC_SECS` se purgan en las
 * siguientes request para que el mapa no crezca sin limite. */
const IDLE_GC_SECS: u64 = 300;
const MAX_ENTRIES: usize = 10_000;

impl RateLimiter {
    pub fn new(policy: RatePolicy) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            policy,
            anchor: std::time::Instant::now(),
        }
    }

    pub fn policy(&self) -> RatePolicy {
        self.policy
    }

    /* Version HTTP: reloj real monotono derivado del anchor. */
    pub fn allow(&self, key: impl Into<String>) -> Take {
        self.allow_at(key, self.anchor.elapsed())
    }

    /* Decide si la clave puede pasar. Usa `now` como reloj monotono
     * sintetico (testeable) para el GC y la recarga. */
    fn allow_at(&self, key: impl Into<String>, now: Duration) -> Take {
        let key = key.into();
        if !self.policy.enabled() {
            return Take::Allowed;
        }
        let mut map = self.inner.lock().unwrap();
        if map.len() >= MAX_ENTRIES && !map.contains_key(&key) {
            /* Mapa saturado y clave nueva: rechaza en vez de crecer sin
             * fin (fallo ruidoso ante una lluvia de IPs). */
            return Take::Denied;
        }
        let entry = map.entry(key).or_insert_with(|| Entry {
            bucket: TokenBucket::new(self.policy.burst, self.policy.rate_per_sec),
            last_seen: now,
        });
        entry.last_seen = now;
        let result = entry.bucket.take(now, 1.0);

        if now.as_secs() > IDLE_GC_SECS {
            let cutoff = now.saturating_sub(Duration::from_secs(IDLE_GC_SECS));
            map.retain(|_, e| e.last_seen >= cutoff);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn bucket_starts_full() {
        let mut b = TokenBucket::new(3.0, 1.0);
        assert_eq!(b.take(secs(0), 3.0), Take::Allowed);
        assert_eq!(b.take(secs(0), 1.0), Take::Denied);
    }

    #[test]
    fn bucket_refills_over_time() {
        let mut b = TokenBucket::new(3.0, 1.0);
        assert_eq!(b.take(secs(0), 3.0), Take::Allowed);
        assert_eq!(b.take(secs(1), 1.0), Take::Allowed); // recargo 1
        assert_eq!(b.take(secs(1), 1.0), Take::Denied); // sin recargo extra
        assert_eq!(b.take(secs(4), 3.0), Take::Allowed); // recargo 3 (hasta capacity)
    }

    #[test]
    fn bucket_caps_at_capacity() {
        let mut b = TokenBucket::new(2.0, 1.0);
        b.take(secs(0), 1.0);
        // 1000s despues no puede acumular mas que capacity
        b.take(secs(1000), 2.0);
        assert_eq!(b.take(secs(1000), 1.0), Take::Denied);
    }

    #[test]
    fn policy_disabled_allows_everything() {
        let l = RateLimiter::new(RatePolicy {
            rate_per_sec: 0.0,
            burst: 0.0,
        });
        assert_eq!(l.allow_at("ip", secs(0)), Take::Allowed);
    }

    #[test]
    fn limiter_enforces_per_key_burst() {
        let l = RateLimiter::new(RatePolicy {
            rate_per_sec: 1.0,
            burst: 2.0,
        });
        assert_eq!(l.allow_at("a", secs(0)), Take::Allowed);
        assert_eq!(l.allow_at("a", secs(0)), Take::Allowed);
        assert_eq!(l.allow_at("a", secs(0)), Take::Denied);
        // otra clave tiene su propio bucket
        assert_eq!(l.allow_at("b", secs(0)), Take::Allowed);
    }

    #[test]
    fn limiter_refills_after_idle() {
        let l = RateLimiter::new(RatePolicy {
            rate_per_sec: 1.0,
            burst: 2.0,
        });
        assert_eq!(l.allow_at("a", secs(0)), Take::Allowed);
        assert_eq!(l.allow_at("a", secs(0)), Take::Allowed);
        assert_eq!(l.allow_at("a", secs(0)), Take::Denied);
        assert_eq!(l.allow_at("a", secs(2)), Take::Allowed); // 1 token recargado
    }

    #[test]
    fn limiter_gc_evicts_idle_entries() {
        let l = RateLimiter::new(RatePolicy {
            rate_per_sec: 1.0,
            burst: 2.0,
        });
        assert_eq!(l.allow_at("a", secs(1000)), Take::Allowed);
        // 500s despues: entrada purgada, bucket fresco
        assert_eq!(l.allow_at("a", secs(1500)), Take::Allowed);
    }

    #[test]
    fn limiter_rejects_new_keys_on_full_map() {
        let mut map = HashMap::new();
        for i in 0..MAX_ENTRIES {
            map.insert(
                format!("ip-{i}"),
                Entry {
                    bucket: TokenBucket::new(1.0, 1.0),
                    last_seen: secs(0),
                },
            );
        }
        let l = RateLimiter {
            inner: Mutex::new(map),
            policy: RatePolicy {
                rate_per_sec: 1.0,
                burst: 1.0,
            },
            anchor: std::time::Instant::now(),
        };
        // mapa lleno: una clave nueva se rechaza sin crecer
        assert_eq!(l.allow_at("nueva", secs(0)), Take::Denied);
    }

    #[test]
    fn policy_returns_policy() {
        let l = RateLimiter::new(RatePolicy {
            rate_per_sec: 5.0,
            burst: 20.0,
        });
        let p = l.policy();
        assert_eq!(p.rate_per_sec, 5.0);
        assert_eq!(p.burst, 20.0);
        assert!(p.enabled());
    }
}
