use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

/*
 * Canal de eventos realtime para el WebSocket.
 *
 * Un `EventBus` (wrap de un `broadcast` de tokio) desacopla a los
 * productores (evaluador de alertas, runner de health checks) de los
 * consumidores (el endpoint WebSocket `GET /api/v1/events`): cada
 * publicador hace `bus.publish(&event)` y todos los suscriptores
 * conectados reciben la misma serializacion.
 *
 * `broadcast` es la primitiva correcta para fan-out a N clientes: un
 * suscriptor lento no bloquea a los demas, pero sus eventos vencidos se
 * descartan (el cliente recibe un mensaje `events_lagged` y sabe que se
 * perdio `n` eventos). La capacidad de la cola es acotada y configurable
 * (`OBS_WS_CHANNEL_CAPACITY`; cada suscriptor mantiene su propia cola).
 *
 * El evento es una serializacion JSON con `type` como tag: el dashboard
 * dispacha por campo. `ts` es la hora de arribo del ciclo que lo genero
 * (filosofia last_seen, ADR-0003: el reloj que importa es el del
 * collector, no el del cliente).
 */

pub const WS_CHANNEL_CAPACITY_DEFAULT: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)] // los nombres repiten el tag del serde
pub enum Event {
    /* Transicion real de una alerta (lo mismo que persiste
     * `apply_alert_steps` en `alert_events`: creacion, promocion o
     * resolucion). Los pasos Stay* no emiten, igual que en el historial. */
    AlertEvent {
        rule_id: i64,
        rule_name: String,
        agent_id: Uuid,
        from_state: Option<String>,
        to_state: String,
        ts: DateTime<Utc>,
    },
    /* Corrida de un health check, con la transicion de estado up/down
     * (si hubo flip) y el detalle del probe. */
    HealthResult {
        check_id: i64,
        check_name: String,
        ok: bool,
        latency_ms: i64,
        detail: String,
        ts: DateTime<Utc>,
        state_changed: bool,
        state: Option<String>,
        since: Option<DateTime<Utc>>,
    },
    /* Transicion del estado de conectividad derivado de un agent
     * (ONLINE/DEGRADED/OFFLINE): el runner detecta un cambio
     * contra el ultimo estado persistido y lo emite al bus. */
    ConnectivityEvent {
        agent_id: Uuid,
        from_state: Option<String>,
        to_state: String,
        ts: DateTime<Utc>,
    },
}

impl Event {
    /* Evento de transicion de alerta. `from`/`to` van como estan en
     * `alert_events` (strings, `from` NULL en la creacion). */
    pub fn alert(
        rule_id: i64,
        rule_name: impl Into<String>,
        agent_id: Uuid,
        from: Option<&str>,
        to: &str,
        ts: DateTime<Utc>,
    ) -> Self {
        Event::AlertEvent {
            rule_id,
            rule_name: rule_name.into(),
            agent_id,
            from_state: from.map(str::to_string),
            to_state: to.to_string(),
            ts,
        }
    }

    /* Evento de corrida de check. `state`/`since` son los del resultado
     * (Some siempre que haya corrida), `state_changed` marca el flip. */
    #[allow(clippy::too_many_arguments)]
    pub fn health(
        check_id: i64,
        check_name: impl Into<String>,
        ok: bool,
        latency_ms: i64,
        detail: impl Into<String>,
        ts: DateTime<Utc>,
        state_changed: bool,
        state: &str,
        since: DateTime<Utc>,
    ) -> Self {
        Event::HealthResult {
            check_id,
            check_name: check_name.into(),
            ok,
            latency_ms,
            detail: detail.into(),
            ts,
            state_changed,
            state: Some(state.to_string()),
            since: Some(since),
        }
    }

    /* Evento de cambio de estado de conectividad de un agente (publicado
     * 6.3). `from` es None en la primera observacion (no sabemos el
     * estado previo). */
    pub fn connectivity(agent_id: Uuid, from: Option<&str>, to: &str, ts: DateTime<Utc>) -> Self {
        Event::ConnectivityEvent {
            agent_id,
            from_state: from.map(str::to_string),
            to_state: to.to_string(),
            ts,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("el evento siempre es serializable")
    }
}

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<String>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    pub fn publish(&self, event: &Event) {
        /* Sin suscriptores -> SendError con el valor devuelto: no es un
         * fallo, simplemente no hay nadie escuchando. */
        match self.tx.send(event.to_json()) {
            Ok(n) => tracing::debug!(subscribers = n, "evento publicado al bus"),
            Err(_) => tracing::debug!("evento publicado sin suscriptores"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use uuid::Uuid;

    fn ts() -> DateTime<Utc> {
        DateTime::from_timestamp(1_720_000_000, 0).unwrap()
    }

    fn agent() -> Uuid {
        Uuid::parse_str("bca99f71-8eaa-f6f1-55b2-14a92fdd309f").unwrap()
    }

    #[test]
    fn alert_event_serializes_with_type_tag() {
        let e = Event::alert(7, "cpu-alta", agent(), None, "firing", ts());
        let value: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(value["type"], "alert_event");
        assert_eq!(value["rule_id"], 7);
        assert_eq!(value["rule_name"], "cpu-alta");
        assert_eq!(value["agent_id"], agent().to_string());
        assert_eq!(value["from_state"], serde_json::Value::Null);
        assert_eq!(value["to_state"], "firing");
        assert_eq!(value["ts"], "2024-07-03T09:46:40Z");
    }

    #[test]
    fn alert_event_keeps_from_state_when_present() {
        let e = Event::alert(7, "cpu-alta", agent(), Some("pending"), "firing", ts());
        let value: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(value["from_state"], "pending");
    }

    #[test]
    fn health_event_serializes_with_type_tag() {
        let e = Event::health(3, "http-root", true, 12, "HTTP 200", ts(), true, "up", ts());
        let value: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(value["type"], "health_result");
        assert_eq!(value["check_id"], 3);
        assert_eq!(value["check_name"], "http-root");
        assert_eq!(value["ok"], true);
        assert_eq!(value["latency_ms"], 12);
        assert_eq!(value["detail"], "HTTP 200");
        assert_eq!(value["state_changed"], true);
        assert_eq!(value["state"], "up");
        assert!(value["since"].is_string());
    }

    #[test]
    fn tagged_enums_are_unique() {
        assert_ne!(
            Event::alert(1, "a", agent(), None, "pending", ts()).to_json(),
            Event::health(1, "a", true, 1, "d", ts(), false, "up", ts()).to_json()
        );
    }

    #[test]
    fn connectivity_event_serializes_with_type_tag() {
        let e = Event::connectivity(agent(), Some("online"), "degraded", ts());
        let value: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(value["type"], "connectivity_event");
        assert_eq!(value["agent_id"], agent().to_string());
        assert_eq!(value["from_state"], "online");
        assert_eq!(value["to_state"], "degraded");
        assert_eq!(value["ts"], "2024-07-03T09:46:40Z");
    }

    #[test]
    fn connectivity_event_first_observation_has_null_from() {
        let e = Event::connectivity(agent(), None, "online", ts());
        let value: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(value["from_state"], serde_json::Value::Null);
        assert_eq!(value["to_state"], "online");
    }

    #[tokio::test]
    async fn subscriber_receives_published_event() {
        let bus = EventBus::new(4);
        let mut rx = bus.subscribe();
        bus.publish(&Event::alert(1, "cpu-alta", agent(), None, "pending", ts()));
        let text = rx.recv().await.unwrap();
        assert!(text.contains("\"type\":\"alert_event\""));
    }

    #[tokio::test]
    async fn only_events_after_subscribe_are_delivered() {
        let bus = EventBus::new(4);
        bus.publish(&Event::alert(1, "a", agent(), None, "pending", ts()));
        let mut rx = bus.subscribe();
        bus.publish(&Event::alert(2, "b", agent(), None, "firing", ts()));
        let text = rx.recv().await.unwrap();
        assert!(text.contains("\"rule_id\":2"));
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let bus = EventBus::new(8);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        bus.publish(&Event::health(1, "x", true, 1, "d", ts(), true, "up", ts()));
        let a = rx1.recv().await.unwrap();
        let b = rx2.recv().await.unwrap();
        assert_eq!(a, b);
    }
}
