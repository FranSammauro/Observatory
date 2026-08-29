use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db;
use crate::events::{Event, EventBus};
use crate::state::{
    connectivity_state, connectivity_state_from_str, ConnectivityState, StateLimits,
};

/*
 * Runner de eventos de conectividad (Fase 6, bloque 6.3).
 *
 * El estado ONLINE/DEGRADED/OFFLINE (bloque 4.2) es una funcion pura que
 * se calcula al leer a partir de agents.last_seen; no hay un flujo de
 * transiciones porque nada lo registraba. Este runner le agrega la
 * "historia": cada OBS_CONNECTIVITY_POLL_SECS recorre los agentes,
 * calcula el estado derivado y lo compara contra el ultimo persistido en
 * `agents.last_connectivity_state`. Cuando cambia:
 *
 *   1. persiste la transicion en `connectivity_events` (from -> to), y
 *   2. publica un `connectivity_event` al bus (WebSocket).
 *
 * La primera observacion de un agent (last_connectivity_state NULL)
 * registra la transicion con from = NULL, igual que la creacion de una
 * alerta en `alert_events`: el timeline "nacimiento del agent" queda en
 * la historia con su estado inicial.
 *
 * `ts` del evento = hora del ciclo que detecto el cambio (filosofia
 * last_seen, ADR-0003: importa cuando lo vimos).
 */

pub struct ConnectivityTransition {
    pub agent_id: Uuid,
    pub from_state: Option<ConnectivityState>,
    pub to_state: ConnectivityState,
}

/*
 * Transiciones del ciclo (pura): para cada agente, si el estado derivado
 * difiere del ultimo persistido -> una transicion. La primera
 * observacion (NULL persistido) cuenta como transicion desde NULL, como
 * la creacion de alerta en el historial.
 */
pub fn detect_transitions(
    agents: &[db::AgentConnectivityRow],
    now: DateTime<Utc>,
    limits: &StateLimits,
) -> Vec<ConnectivityTransition> {
    let mut out = Vec::with_capacity(agents.len());
    for a in agents {
        let derived = connectivity_state(a.last_seen, now, limits);
        let prev = connectivity_state_from_str(a.last_connectivity_state.as_deref());
        if prev != Some(derived) {
            out.push(ConnectivityTransition {
                agent_id: a.agent_id,
                from_state: prev,
                to_state: derived,
            });
        }
    }
    out
}

pub fn spawn_connectivity_runner(pool: PgPool, config: Arc<Config>, bus: EventBus) {
    tokio::spawn(async move {
        let poll = std::time::Duration::from_secs(config.connectivity_poll_secs.max(1) as u64);
        let mut ticker = tokio::time::interval(poll);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(e) = run_cycle(&pool, &config, &bus).await {
                tracing::error!("ciclo de conectividad fallo: {e}");
            }
        }
    });
}

async fn run_cycle(pool: &PgPool, config: &Config, bus: &EventBus) -> Result<(), sqlx::Error> {
    let agents = db::list_agents_connectivity(pool).await?;
    if agents.is_empty() {
        return Ok(());
    }
    let now = Utc::now();
    let transitions = detect_transitions(&agents, now, &config.state_limits());

    if transitions.is_empty() {
        return Ok(());
    }

    db::apply_connectivity_transitions(pool, &transitions).await?;

    for t in &transitions {
        let ts = now;
        bus.publish(&Event::connectivity(
            t.agent_id,
            t.from_state.as_ref().map(ConnectivityState::as_str),
            t.to_state.as_str(),
            ts,
        ));
        tracing::info!(
            agent_id = %t.agent_id,
            from = t.from_state.as_ref().map(ConnectivityState::as_str).unwrap_or("?"),
            to = t.to_state.as_str(),
            "estado de conectividad cambio"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn limits() -> StateLimits {
        StateLimits {
            online_secs: 15,
            degraded_secs: 60,
        }
    }

    fn row(agent_id: &str, age_secs: i64, last: Option<&str>) -> db::AgentConnectivityRow {
        db::AgentConnectivityRow {
            agent_id: Uuid::parse_str(agent_id).unwrap(),
            last_seen: Utc::now() - Duration::seconds(age_secs),
            last_connectivity_state: last.map(|s| s.to_string()),
        }
    }

    const A: &str = "bca99f71-8eaa-f6f1-55b2-14a92fdd309f";

    #[test]
    fn no_agents_yields_no_transitions() {
        assert!(detect_transitions(&[], Utc::now(), &limits()).is_empty());
    }

    #[test]
    fn first_observation_is_a_transition_from_null() {
        let agents = vec![
            row(A, 5, None),
            row("11111111-8eaa-f6f1-55b2-14a92fdd309f", 120, None),
        ];
        let out = detect_transitions(&agents, Utc::now(), &limits());
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|t| t.from_state.is_none()));
        assert_eq!(out[0].to_state, ConnectivityState::Online);
        assert_eq!(out[1].to_state, ConnectivityState::Offline);
    }

    #[test]
    fn unchanged_state_yields_no_transition() {
        let agents = vec![row(A, 5, Some("online"))];
        assert!(detect_transitions(&agents, Utc::now(), &limits()).is_empty());
    }

    #[test]
    fn online_to_degraded_is_a_transition() {
        let agents = vec![row(A, 50, Some("online"))];
        let out = detect_transitions(&agents, Utc::now(), &limits());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].from_state, Some(ConnectivityState::Online));
        assert_eq!(out[0].to_state, ConnectivityState::Degraded);
    }

    #[test]
    fn degraded_to_offline_is_a_transition() {
        let agents = vec![row(A, 120, Some("degraded"))];
        let out = detect_transitions(&agents, Utc::now(), &limits());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].from_state, Some(ConnectivityState::Degraded));
        assert_eq!(out[0].to_state, ConnectivityState::Offline);
    }

    #[test]
    fn stale_last_state_still_emits() {
        /* last_connectivity_state quedo atrasado (guardado como online
         * hace mucho): el derivado manda. */
        let agents = vec![row(A, 120, Some("online"))];
        let out = detect_transitions(&agents, Utc::now(), &limits());
        assert_eq!(out[0].from_state, Some(ConnectivityState::Online));
        assert_eq!(out[0].to_state, ConnectivityState::Offline);
    }

    #[test]
    fn multiple_agents_in_one_cycle() {
        let agents = vec![
            row(A, 5, Some("online")),
            row("11111111-8eaa-f6f1-55b2-14a92fdd309f", 50, Some("online")),
            row("22222222-8eaa-f6f1-55b2-14a92fdd309f", 10, None),
        ];
        let out = detect_transitions(&agents, Utc::now(), &limits());
        assert_eq!(out.len(), 2);
        assert!(out
            .iter()
            .any(|t| t.to_state == ConnectivityState::Degraded));
        assert!(out.iter().any(|t| t.to_state == ConnectivityState::Online));
    }
}
