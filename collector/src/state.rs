use chrono::{DateTime, Duration, Utc};

/*
 * Maquina de estados de conectividad (Fase 4, bloque 4.2).
 *
 * El estado se DERIVA de `agents.last_seen` (hora de arribe al servidor,
 * ADR-0003): es una funcion pura del lapso desde que oimos del agent, no
 * un dato persistido ni un flujo de transiciones. El dashboard pregunta
 * el estado leido de la DB; no hay reprocesamiento de payloads.
 *
 *   ahora - last_seen <= online_secs   -> ONLINE
 *   ahora - last_seen <= degraded_secs -> DEGRADED
 *   caso contrario                     -> OFFLINE
 *
 * Umbrales configurables (OBS_STATE_ONLINE_SECS / OBS_STATE_DEGRADED_SECS).
 * Base: el agent emite heartbeat cada 5s y metricas cada 10s; ONLINE a 15s
 * (= ~3 heartbeats perdidos) y DEGRADED a 60s cubren el rango entre "se
 * lo ve" y "se lo da por perdido". Un last_seen futuro (reloj del
 * servidor atrasado) cuenta como ONLINE.
 */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectivityState {
    Online,
    Degraded,
    Offline,
}

impl ConnectivityState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConnectivityState::Online => "online",
            ConnectivityState::Degraded => "degraded",
            ConnectivityState::Offline => "offline",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StateLimits {
    pub online_secs: i64,
    pub degraded_secs: i64,
}

pub fn connectivity_state(
    last_seen: DateTime<Utc>,
    now: DateTime<Utc>,
    limits: &StateLimits,
) -> ConnectivityState {
    let age = now.signed_duration_since(last_seen);
    if age <= Duration::seconds(limits.online_secs) {
        ConnectivityState::Online
    } else if age <= Duration::seconds(limits.degraded_secs) {
        ConnectivityState::Degraded
    } else {
        ConnectivityState::Offline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> StateLimits {
        StateLimits {
            online_secs: 15,
            degraded_secs: 60,
        }
    }

    #[test]
    fn sees_recent_agent_as_online() {
        let now = Utc::now();
        let last_seen = now - Duration::seconds(14);
        assert_eq!(
            connectivity_state(last_seen, now, &limits()),
            ConnectivityState::Online
        );
    }

    #[test]
    fn online_boundary_is_inclusive() {
        let now = Utc::now();
        let last_seen = now - Duration::seconds(15);
        assert_eq!(
            connectivity_state(last_seen, now, &limits()),
            ConnectivityState::Online
        );
    }

    #[test]
    fn between_thresholds_is_degraded() {
        let now = Utc::now();
        let last_seen = now - Duration::seconds(59);
        assert_eq!(
            connectivity_state(last_seen, now, &limits()),
            ConnectivityState::Degraded
        );
    }

    #[test]
    fn degraded_boundary_is_inclusive() {
        let now = Utc::now();
        let last_seen = now - Duration::seconds(60);
        assert_eq!(
            connectivity_state(last_seen, now, &limits()),
            ConnectivityState::Degraded
        );
    }

    #[test]
    fn beyond_degraded_is_offline() {
        let now = Utc::now();
        let last_seen = now - Duration::seconds(61);
        assert_eq!(
            connectivity_state(last_seen, now, &limits()),
            ConnectivityState::Offline
        );
    }

    #[test]
    fn future_last_seen_counts_as_online() {
        let now = Utc::now();
        let last_seen = now + Duration::seconds(30);
        assert_eq!(
            connectivity_state(last_seen, now, &limits()),
            ConnectivityState::Online
        );
    }
}
