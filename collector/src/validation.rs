use chrono::{DateTime, TimeZone, Utc};

use crate::error::ApiError;

/*
 * Validacion temporal de los payloads de ingestion (ADR-0003).
 *
 * El agent envia `timestamp` en segundos desde epoch (wall clock del
 * host). El Collector acepta un rango acotado alrededor del "ahora" del
 * servidor: tolera skew hacia el futuro (reloj del host adelantado) y
 * rechaza datos viejos mas alla de una ventana de retraso razonable
 * (el agente no persiste samples en disco; si el timestamp esta fuera
 * un dato muy viejo es ruido o un reloj descompuesto, no un backlog
 * legitimo).
 */

#[derive(Debug, Clone, Copy)]
pub struct TimeLimits {
    pub future_skew_secs: i64,
    pub max_past_age_secs: i64,
}

pub fn validate_timestamp(ts: i64, now_epoch_secs: i64, limits: &TimeLimits) -> bool {
    let diff = now_epoch_secs - ts;
    diff >= -limits.future_skew_secs && diff <= limits.max_past_age_secs
}

pub fn utc_from_unix_ts(
    ts: u64,
    now_epoch_secs: i64,
    limits: &TimeLimits,
) -> Result<DateTime<Utc>, ApiError> {
    let ts_i64 = i64::try_from(ts)
        .map_err(|_| ApiError::bad_request("invalid_timestamp", "timestamp fuera de rango"))?;
    if !validate_timestamp(ts_i64, now_epoch_secs, limits) {
        return Err(ApiError::bad_request(
            "timestamp_out_of_range",
            format!("timestamp {ts_i64} fuera de la ventana aceptada (skew futuro <= {}s, edad maxima {}s)",
                limits.future_skew_secs, limits.max_past_age_secs),
        ));
    }
    Utc.timestamp_opt(ts_i64, 0)
        .single()
        .ok_or_else(|| ApiError::bad_request("invalid_timestamp", "timestamp invalido"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> TimeLimits {
        TimeLimits {
            future_skew_secs: 60,
            max_past_age_secs: 600,
        }
    }

    #[test]
    fn accepts_now_and_recent_past() {
        let now = 1_000_000_000i64;
        assert!(validate_timestamp(now, now, &limits()));
        assert!(validate_timestamp(now - 599, now, &limits()));
        assert!(validate_timestamp(now + 59, now, &limits()));
    }

    #[test]
    fn rejects_too_far_in_future() {
        let now = 1_000_000_000i64;
        assert!(!validate_timestamp(now + 61, now, &limits()));
    }

    #[test]
    fn rejects_too_old() {
        let now = 1_000_000_000i64;
        assert!(!validate_timestamp(now - 601, now, &limits()));
    }

    #[test]
    fn boundaries_are_inclusive() {
        let now = 1_000_000_000i64;
        assert!(validate_timestamp(now - 600, now, &limits()));
        assert!(validate_timestamp(now + 60, now, &limits()));
    }
}
