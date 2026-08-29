use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db::{self, AlertRuleRow};
use crate::error::ApiError;
use crate::events::{Event as StreamEvent, EventBus};

/*
 * Alert engine (Fase 5, bloque 5.1): reglas declarativas, evaluacion y
 * gestion por API.
 *
 * Una regla es declarativa: metric_name + entidad opcional + operador +
 * umbral (+ for_secs). La evaluacion es una funcion pura sobre la serie
 * que el engine lee de `metric_samples`: decide si la CONDICION se
 * sostiene sobre la muestra mas reciente y, cuando se sostiene, desde
 * cuando (alimenta el `for` de la maquina de estados del bloque 5.2).
 * Este bloque no persiste transiciones de estado: es el motor que el
 * bloque 5.2 va a consumir.
 */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondOp {
    Ge,
    Gt,
    Le,
    Lt,
}

impl CondOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            CondOp::Ge => "ge",
            CondOp::Gt => "gt",
            CondOp::Le => "le",
            CondOp::Lt => "lt",
        }
    }

    pub fn matches(&self, value: f64, threshold: f64) -> bool {
        /* Un valor no finito nunca satisface una regla (defensa contra
         * NaN/Inf, que de todos modos no pueden llegar a la DB). */
        if !value.is_finite() {
            return false;
        }
        match self {
            CondOp::Ge => value >= threshold,
            CondOp::Gt => value > threshold,
            CondOp::Le => value <= threshold,
            CondOp::Lt => value < threshold,
        }
    }
}

pub fn parse_op(s: &str) -> Option<CondOp> {
    match s.trim() {
        "ge" => Some(CondOp::Ge),
        "gt" => Some(CondOp::Gt),
        "le" => Some(CondOp::Le),
        "lt" => Some(CondOp::Lt),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct AlertRule {
    pub metric_name: String,
    pub entity: Option<String>,
    pub op: CondOp,
    pub threshold: f64,
    pub for_secs: i64,
}

impl AlertRule {
    pub fn from_row(row: &AlertRuleRow) -> Option<AlertRule> {
        Some(AlertRule {
            metric_name: row.metric_name.clone(),
            entity: row.entity.clone(),
            op: parse_op(&row.op)?,
            threshold: row.threshold,
            for_secs: row.for_secs,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SeriesPoint {
    pub ts: DateTime<Utc>,
    pub value: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesVerdict {
    /* No hay muestras de la metrica/entidad en la ventana. */
    NoData,
    /* La muestra mas reciente no satisface la condicion. */
    NotHolding,
    /* Se sostiene: `since` es la ts del inicio del tramo continuo. */
    Holding {
        since: DateTime<Utc>,
        holds_for_secs: i64,
        meets_for: bool,
    },
}

/*
 * Serie en orden temporal ASC (la query del evaluador ya la ordena).
 * Determina si la condicion se sostiene sobre la muestra mas reciente y
 * cuanto lleva sostenida: recorre hacia atras mientras las muestras sigan
 * cumpliendo. La duracion se calcula SOLO con timestamps de muestras
 * (last.ts - since), no con reloj: determinista ante el mismo dataset e
 * inmune al skew admitido en ingestion (hasta 60s). `now` solo cota
 * tramos futuros anomalos. Un hueco en los datos no corta el tramo (no
 * interpolamos: no sabemos el valor entre muestras).
 */
pub fn evaluate_series(
    points: &[SeriesPoint],
    rule: &AlertRule,
    now: DateTime<Utc>,
) -> SeriesVerdict {
    let Some(last) = points.last() else {
        return SeriesVerdict::NoData;
    };

    if !rule.op.matches(last.value, rule.threshold) {
        return SeriesVerdict::NotHolding;
    }

    let mut since = last.ts;
    for p in points.iter().rev().skip(1) {
        if rule.op.matches(p.value, rule.threshold) && p.ts <= since {
            since = p.ts;
        } else {
            break;
        }
    }

    let end = last.ts.min(now);
    let holds_for_secs = end.signed_duration_since(since).num_seconds().max(0);
    SeriesVerdict::Holding {
        since,
        holds_for_secs,
        meets_for: holds_for_secs >= rule.for_secs,
    }
}

/*
 * Payload de creacion de una regla (POST /api/v1/alerts/rules).
 * Validacion pura, igual que el resto de los parametros de la API.
 */
#[derive(Debug, Deserialize)]
pub struct CreateRule {
    pub name: String,
    #[serde(rename = "metric_name")]
    pub metric_name: String,
    pub entity: Option<String>,
    pub op: String,
    pub threshold: f64,
    #[serde(rename = "for_secs", default)]
    pub for_secs: i64,
}

pub struct RuleDraft {
    pub name: String,
    pub metric_name: String,
    pub entity: Option<String>,
    pub op: CondOp,
    pub threshold: f64,
    pub for_secs: i64,
}

impl CreateRule {
    pub fn into_draft(self) -> Result<RuleDraft, ApiError> {
        let name = self.name.trim().to_string();
        if name.is_empty() {
            return Err(ApiError::bad_request(
                "invalid_rule_name",
                "name no puede estar vacio",
            ));
        }

        let metric_name = self.metric_name.trim().to_string();
        if metric_name.is_empty() {
            return Err(ApiError::bad_request(
                "invalid_metric_name",
                "metric_name no puede estar vacio",
            ));
        }

        let entity = match self.entity {
            Some(e) if e.trim().is_empty() => {
                return Err(ApiError::bad_request(
                    "invalid_entity",
                    "entity no puede estar vacia",
                ));
            }
            Some(e) => Some(e.trim().to_string()),
            None => None,
        };

        let op = parse_op(&self.op)
            .ok_or_else(|| ApiError::bad_request("invalid_op", "op debe ser ge, gt, le o lt"))?;

        if !self.threshold.is_finite() {
            return Err(ApiError::bad_request(
                "invalid_threshold",
                "threshold debe ser un numero finito",
            ));
        }

        if self.for_secs < 0 {
            return Err(ApiError::bad_request(
                "invalid_for_secs",
                "for_secs no puede ser negativo",
            ));
        }

        Ok(RuleDraft {
            name,
            metric_name,
            entity,
            op,
            threshold: self.threshold,
            for_secs: self.for_secs,
        })
    }
}

/*
 * Maquina de estados (bloque 5.2). Estados nominales del informe:
 * INACTIVE -> PENDING -> FIRING -> RESOLVED.
 *
 *   INACTIVE : no hay condicion sostenida. No se persiste (ausencia de
 *              fila en `alerts`).
 *   PENDING  : la condicion se sostiene pero todavia no alcanzo `for_secs`
 *              de la regla. Si deja de sostenerse antes -> RESOLVED.
 *   FIRING   : la condicion se sostuvo `for_secs` (o mas). Con
 *              hysteresis: no se resuelve apenas la condicion cae; se
 *              mantiene hasta que la condicion este ausente
 *              OBS_ALERT_RESOLVE_GRACE_SECS (evita flapping).
 *   RESOLVED : el tramo termino (PENDING que cae, o FIRING con ventana
 *              de resolucion vencida). No se persiste; la fila se borra.
 *
 * La transicion es una funcion pura (`next_step`) sobre el estado actual
 * y el veredicto de `evaluate_series` del bloque 5.1; el evaluador
 * aplica el resultado en `alerts` en una transaccion (db::apply_alert_steps).
 */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertState {
    Pending,
    Firing,
}

impl AlertState {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertState::Pending => "pending",
            AlertState::Firing => "firing",
        }
    }
}

/* Estado actual de una alerta activa, como lo devuelve la DB. Solo
 * importan el estado y la ventana de resolucion: `since` de la fila es
 * informativo y se reescribe en cada ciclo con el del veredicto. */
#[derive(Debug, Clone, Copy)]
pub struct CurrentAlert {
    pub state: AlertState,
    pub resolve_from: Option<DateTime<Utc>>,
}

/* Condicion sostenida en este ciclo (veredicto del bloque 5.1 reducido). */
#[derive(Debug, Clone, Copy)]
pub struct Holding {
    pub since: DateTime<Utc>,
    pub meets_for: bool,
}

/*
 * Paso de la maquina que el evaluador debe aplicar. Cada variante mapea a
 * una operacion de `alerts`:
 *   Inactive / StayResolving -> nada
 *   Pending/Firing/StayPending/ToFiring/StayFiring -> UPSERT
 *   StartResolving -> abrir la ventana de resolucion (resolve_from = now)
 *   Resolved -> DELETE
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /* Sin alerta y sin condicion sostenida (INACTIVE). */
    Inactive,
    /* Crear fila en pending. */
    Pending { since: DateTime<Utc> },
    /* Crear fila en firing (la condicion arranco ya con for cubierto). */
    Firing { since: DateTime<Utc> },
    /* Actualizar pending (el tramo sigue, con nuevo inicio). */
    StayPending { since: DateTime<Utc> },
    /* pending -> firing. */
    ToFiring { since: DateTime<Utc> },
    /* Firing sostenida: limpiar ventana de resolucion. */
    StayFiring { since: DateTime<Utc> },
    /* Firing: la condicion cayo, arranca la ventana (hysteresis). */
    StartResolving,
    /* Firing: la ventana de resolucion sigue abierta, esperando. */
    StayResolving,
    /* RESOLVED: la fila se borra. */
    Resolved,
}

/*
 * Transicion de la maquina, pura y testeable. `current` es la fila activa
 * (None = INACTIVE), `holding` es Some si la condicion se sostiene en el
 * tramo (None = NoData/NotHolding). `resolve_grace_secs` es la ventana de
 * hysteresis; con 0 la resolucion es inmediata.
 */
pub fn next_step(
    current: Option<CurrentAlert>,
    holding: Option<Holding>,
    now: DateTime<Utc>,
    resolve_grace_secs: i64,
) -> Step {
    match (current, holding) {
        (None, None) => Step::Inactive,

        (None, Some(h)) if h.meets_for => Step::Firing { since: h.since },
        (None, Some(h)) => Step::Pending { since: h.since },

        (Some(c), Some(h)) if c.state == AlertState::Pending => {
            if h.meets_for {
                Step::ToFiring { since: h.since }
            } else {
                Step::StayPending { since: h.since }
            }
        }
        (Some(_), Some(h)) => Step::StayFiring { since: h.since },

        (Some(c), None) if c.state == AlertState::Pending => Step::Resolved,
        (Some(c), None) => match c.resolve_from {
            None if resolve_grace_secs <= 0 => Step::Resolved,
            None => Step::StartResolving,
            Some(t0) => {
                if now.signed_duration_since(t0).num_seconds() >= resolve_grace_secs {
                    Step::Resolved
                } else {
                    Step::StayResolving
                }
            }
        },
    }
}

/*
 * Historial (bloque 5.3): una transicion de estado se registra en
 * `alert_events` solo cuando la maquina CAMBIA de estado (creacion,
 * promocion o resolucion). `from = None` significa desde INACTIVE (no
 * habia fila). Stay* / StartResolving / StayResolving no emiten evento:
 * ahi radica la idempotencia de las transiciones.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventState {
    Pending,
    Firing,
    Resolved,
}

impl EventState {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventState::Pending => "pending",
            EventState::Firing => "firing",
            EventState::Resolved => "resolved",
        }
    }
}

impl From<AlertState> for EventState {
    fn from(s: AlertState) -> Self {
        match s {
            AlertState::Pending => EventState::Pending,
            AlertState::Firing => EventState::Firing,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Event {
    pub from: Option<EventState>,
    pub to: EventState,
}

/*
 * Operaciones que el evaluador le pide a db::apply_alert_steps (una
 * transaccion por ciclo). Los pasos que cambian de estado llevan el
 * `Event` que se inserta en `alert_events` en la misma transaccion.
 */
#[derive(Debug, Clone, Copy)]
pub enum AlertOp {
    Upsert {
        rule_id: i64,
        agent_id: Uuid,
        state: AlertState,
        since: DateTime<Utc>,
        event: Option<Event>,
    },
    StartResolving {
        rule_id: i64,
        agent_id: Uuid,
    },
    Resolved {
        rule_id: i64,
        agent_id: Uuid,
        event: Event,
    },
}

pub struct EvalSummary {
    pub rules: usize,
    pub agents: usize,
    pub pending: usize,
    pub firing: usize,
    pub resolved: usize,
}

/*
 * Un ciclo de evaluacion (bloque 5.2): reglas habilitadas -> ventana de
 * samples por (regla, agent) -> veredicto -> transicion -> persistencia
 * en `alerts`.
 *
 * El evaluador solo visita agents con series en la ventana, pero las
 * alertas activas de agents que dejaron de reportar (o de reglas
 * deshabilitadas) tambien se procesan: entran como "condicion ausente" y
 * la maquina las resuelve (o las mantiene en la ventana de hysteresis).
 */
pub async fn eval_cycle(
    pool: &PgPool,
    config: &Config,
) -> Result<(EvalSummary, Vec<StreamEvent>), sqlx::Error> {
    let now = Utc::now();
    let rows = db::list_enabled_rules(pool).await?;
    let mut summary = EvalSummary {
        rules: rows.len(),
        agents: 0,
        pending: 0,
        firing: 0,
        resolved: 0,
    };

    if rows.is_empty() {
        return Ok((summary, Vec::new()));
    }

    let from = now - Duration::seconds(config.alert_lookback_secs);

    /* Alertas activas actuales, indexadas por (rule_id, agent_id). */
    let current_rows = db::list_current_alerts(pool).await?;
    let mut current: HashMap<(i64, Uuid), CurrentAlert> = HashMap::new();
    for c in &current_rows {
        current.insert(
            (c.rule_id, c.agent_id),
            CurrentAlert {
                state: if c.state == "firing" {
                    AlertState::Firing
                } else {
                    AlertState::Pending
                },
                resolve_from: c.resolve_from,
            },
        );
    }

    let enabled_ids: HashSet<i64> = rows.iter().map(|r| r.id).collect();
    let mut name_by_id: HashMap<i64, String> = HashMap::new();

    /* Veredictos del ciclo: (rule_id, agent_id) -> condicion sostenida? */
    let mut verdicts: HashMap<(i64, Uuid), Option<Holding>> = HashMap::new();

    for row in &rows {
        name_by_id.insert(row.id, row.name.clone());
        let Some(rule) = AlertRule::from_row(row) else {
            tracing::warn!(rule = %row.name, op = %row.op, "regla con operador desconocido, se ignora");
            continue;
        };

        let samples =
            db::recent_samples_for_rule(pool, &rule.metric_name, rule.entity.as_deref(), from)
                .await?;

        /* La query ordena por (agent_id, ts ASC): agrupar en un lugar
         * solo. Por (regla, agent) son pocas decenas de puntos. */
        let mut series: Vec<(Uuid, Vec<SeriesPoint>)> = Vec::new();
        for s in samples {
            match series.last_mut() {
                Some((id, pts)) if *id == s.agent_id => {
                    pts.push(SeriesPoint {
                        ts: s.ts,
                        value: s.value,
                    });
                }
                _ => series.push((
                    s.agent_id,
                    vec![SeriesPoint {
                        ts: s.ts,
                        value: s.value,
                    }],
                )),
            }
        }

        for (agent_id, pts) in &series {
            let holding = match evaluate_series(pts, &rule, now) {
                SeriesVerdict::Holding {
                    since, meets_for, ..
                } => Some(Holding { since, meets_for }),
                _ => None,
            };
            verdicts.insert((row.id, *agent_id), holding);
        }
    }

    /* Union de claves: con serie evaluada o solo con alerta activa. */
    let mut keys: HashSet<(i64, Uuid)> = verdicts.keys().cloned().collect();
    keys.extend(current.keys().cloned());

    let mut ops: Vec<AlertOp> = Vec::new();
    let mut out_events: Vec<StreamEvent> = Vec::new();
    for key in keys {
        summary.agents += 1;
        let (rule_id, agent_id) = key;
        let cur = current.get(&key).copied();
        let holding = verdicts.get(&key).copied().flatten();

        let rule_name = name_by_id.get(&rule_id).map(String::as_str).unwrap_or("?");
        let step = if cur.is_some() && !enabled_ids.contains(&rule_id) {
            /* Regla deshabilitada/borrada: su alerta activa se resuelve. */
            Step::Resolved
        } else {
            next_step(cur, holding, now, config.alert_resolve_grace_secs)
        };

        match step {
            Step::Inactive | Step::StayResolving => {}
            Step::Pending { since } => {
                summary.pending += 1;
                ops.push(AlertOp::Upsert {
                    rule_id,
                    agent_id,
                    state: AlertState::Pending,
                    since,
                    event: Some(Event {
                        from: None,
                        to: EventState::Pending,
                    }),
                });
                out_events.push(StreamEvent::alert(
                    rule_id,
                    rule_name,
                    agent_id,
                    None,
                    EventState::Pending.as_str(),
                    now,
                ));
                tracing::info!(rule = rule_name, %agent_id, ?since, "alerta pending");
            }
            Step::Firing { since } => {
                summary.firing += 1;
                ops.push(AlertOp::Upsert {
                    rule_id,
                    agent_id,
                    state: AlertState::Firing,
                    since,
                    event: Some(Event {
                        from: None,
                        to: EventState::Firing,
                    }),
                });
                out_events.push(StreamEvent::alert(
                    rule_id,
                    rule_name,
                    agent_id,
                    None,
                    EventState::Firing.as_str(),
                    now,
                ));
                tracing::info!(rule = rule_name, %agent_id, ?since, "alerta firing");
            }
            Step::StayPending { since } => {
                summary.pending += 1;
                ops.push(AlertOp::Upsert {
                    rule_id,
                    agent_id,
                    state: AlertState::Pending,
                    since,
                    event: None,
                });
            }
            Step::ToFiring { since } => {
                summary.firing += 1;
                ops.push(AlertOp::Upsert {
                    rule_id,
                    agent_id,
                    state: AlertState::Firing,
                    since,
                    event: Some(Event {
                        from: Some(EventState::Pending),
                        to: EventState::Firing,
                    }),
                });
                out_events.push(StreamEvent::alert(
                    rule_id,
                    rule_name,
                    agent_id,
                    Some(EventState::Pending.as_str()),
                    EventState::Firing.as_str(),
                    now,
                ));
                tracing::info!(rule = rule_name, %agent_id, ?since, "alerta firing (pending -> firing)");
            }
            Step::StayFiring { since } => {
                summary.firing += 1;
                ops.push(AlertOp::Upsert {
                    rule_id,
                    agent_id,
                    state: AlertState::Firing,
                    since,
                    event: None,
                });
            }
            Step::StartResolving => {
                summary.firing += 1;
                ops.push(AlertOp::StartResolving { rule_id, agent_id });
                tracing::info!(rule = rule_name, %agent_id, "alerta entrando en ventana de resolucion (hysteresis)");
            }
            Step::Resolved => {
                summary.resolved += 1;
                let event = Event {
                    from: cur.map(|c| c.state.into()),
                    to: EventState::Resolved,
                };
                ops.push(AlertOp::Resolved {
                    rule_id,
                    agent_id,
                    event,
                });
                out_events.push(StreamEvent::alert(
                    rule_id,
                    rule_name,
                    agent_id,
                    event.from.map(|s| s.as_str()),
                    EventState::Resolved.as_str(),
                    now,
                ));
                tracing::info!(rule = rule_name, %agent_id, "alerta resuelta");
            }
        }
    }

    db::apply_alert_steps(pool, &ops).await?;
    Ok((summary, out_events))
}

/*
 * Lanza el evaluador periodico. SIEMPRE consume un tick inmediato y
 * luego los intervalos; ante un fallo del ciclo (p.ej. DB caida) loguea
 * y sigue en el siguiente tick.
 *
 * Publica al bus los eventos de transicion SOLO cuando el ciclo termino
 * en OK: las transiciones ya estan commiteadas en `alerts`/`alert_events`
 * (misma atomicidad que el historial, bloque 5.3).
 */
pub fn spawn_evaluator(pool: PgPool, config: Arc<Config>, bus: EventBus) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(
            config.alert_eval_interval_secs.max(1) as u64,
        ));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            match eval_cycle(&pool, &config).await {
                Ok((s, events)) => {
                    for ev in &events {
                        bus.publish(ev);
                    }
                    tracing::debug!(
                        rules = s.rules,
                        agents = s.agents,
                        pending = s.pending,
                        firing = s.firing,
                        resolved = s.resolved,
                        events = events.len(),
                        "ciclo de alertas evaluado"
                    );
                }
                Err(e) => tracing::warn!("ciclo de evaluacion de alertas fallo: {e}"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn pt(ts_secs: i64, value: f64) -> SeriesPoint {
        SeriesPoint {
            ts: Utc.timestamp_opt(ts_secs, 0).unwrap(),
            value,
        }
    }

    fn rule(op: CondOp, threshold: f64, for_secs: i64) -> AlertRule {
        AlertRule {
            metric_name: "m".into(),
            entity: None,
            op,
            threshold,
            for_secs,
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_000_000_000, 0).unwrap()
    }

    #[test]
    fn empty_series_is_no_data() {
        assert_eq!(
            evaluate_series(&[], &rule(CondOp::Ge, 0.5, 0), now()),
            SeriesVerdict::NoData
        );
    }

    #[test]
    fn latest_not_satisfying_is_not_holding() {
        let pts = [pt(1_000_000_000 - 60, 0.9), pt(1_000_000_000 - 30, 0.8)];
        assert_eq!(
            evaluate_series(&pts, &rule(CondOp::Ge, 0.85, 0), now()),
            SeriesVerdict::NotHolding
        );
    }

    #[test]
    fn single_point_with_zero_for_holds_immediately() {
        let pts = [pt(1_000_000_000 - 10, 0.95)];
        match evaluate_series(&pts, &rule(CondOp::Ge, 0.9, 0), now()) {
            SeriesVerdict::Holding {
                holds_for_secs,
                meets_for,
                ..
            } => {
                assert_eq!(holds_for_secs, 0);
                assert!(meets_for);
            }
            other => panic!("esperaba Holding, obtuve {other:?}"),
        }
    }

    #[test]
    fn holds_but_below_for_secs_does_not_meet_for() {
        let pts = [pt(1_000_000_000 - 60, 0.95), pt(1_000_000_000 - 30, 0.96)];
        match evaluate_series(&pts, &rule(CondOp::Ge, 0.9, 60), now()) {
            SeriesVerdict::Holding {
                holds_for_secs,
                meets_for,
                ..
            } => {
                assert_eq!(holds_for_secs, 30);
                assert!(!meets_for);
            }
            other => panic!("esperaba Holding, obtuve {other:?}"),
        }
    }

    #[test]
    fn holding_span_reaching_for_secs_meets_for() {
        let pts = [
            pt(1_000_000_000 - 120, 0.95),
            pt(1_000_000_000 - 90, 0.96),
            pt(1_000_000_000 - 30, 0.97),
        ];
        match evaluate_series(&pts, &rule(CondOp::Ge, 0.9, 60), now()) {
            SeriesVerdict::Holding {
                since,
                holds_for_secs,
                meets_for,
            } => {
                assert_eq!(since, Utc.timestamp_opt(1_000_000_000 - 120, 0).unwrap());
                assert_eq!(holds_for_secs, 90);
                assert!(meets_for);
            }
            other => panic!("esperaba Holding, obtuve {other:?}"),
        }
    }

    #[test]
    fn interrupted_run_resets_since() {
        let pts = [
            pt(1_000_000_000 - 60, 0.95),
            pt(1_000_000_000 - 50, 0.96),
            pt(1_000_000_000 - 40, 0.80),
            pt(1_000_000_000 - 30, 0.97),
            pt(1_000_000_000 - 20, 0.98),
        ];
        match evaluate_series(&pts, &rule(CondOp::Ge, 0.9, 0), now()) {
            SeriesVerdict::Holding {
                since,
                holds_for_secs,
                ..
            } => {
                assert_eq!(since, Utc.timestamp_opt(1_000_000_000 - 30, 0).unwrap());
                assert_eq!(holds_for_secs, 10);
            }
            other => panic!("esperaba Holding, obtuve {other:?}"),
        }
    }

    #[test]
    fn non_finite_latest_never_holds() {
        let pts = [pt(1_000_000_000 - 10, f64::NAN)];
        assert_eq!(
            evaluate_series(&pts, &rule(CondOp::Ge, 0.0, 0), now()),
            SeriesVerdict::NotHolding
        );
    }

    #[test]
    fn ge_boundary_is_inclusive() {
        let pts = [pt(1_000_000_000 - 10, 0.9)];
        assert!(matches!(
            evaluate_series(&pts, &rule(CondOp::Ge, 0.9, 0), now()),
            SeriesVerdict::Holding { .. }
        ));
    }

    #[test]
    fn lt_requires_strictly_less() {
        let pts = [pt(1_000_000_000 - 10, 0.5)];
        assert!(matches!(
            evaluate_series(&pts, &rule(CondOp::Lt, 0.5, 0), now()),
            SeriesVerdict::NotHolding
        ));
    }

    #[test]
    fn future_latest_sample_is_clamped_to_now() {
        let pts = [pt(1_000_000_000 + 60, 0.95), pt(1_000_000_000 + 90, 0.96)];
        match evaluate_series(&pts, &rule(CondOp::Ge, 0.9, 30), now()) {
            SeriesVerdict::Holding {
                holds_for_secs,
                meets_for,
                ..
            } => {
                assert_eq!(holds_for_secs, 0);
                assert!(!meets_for);
            }
            other => panic!("esperaba Holding, obtuve {other:?}"),
        }
    }

    #[test]
    fn parse_op_accepts_all_forms() {
        assert_eq!(parse_op("ge"), Some(CondOp::Ge));
        assert_eq!(parse_op(" gt "), Some(CondOp::Gt));
        assert_eq!(parse_op("lt"), Some(CondOp::Lt));
        assert_eq!(parse_op("le"), Some(CondOp::Le));
    }

    #[test]
    fn parse_op_rejects_unknown() {
        assert_eq!(parse_op("=="), None);
        assert_eq!(parse_op(""), None);
    }

    fn cur(state: AlertState, resolve_ago: Option<i64>, base: DateTime<Utc>) -> CurrentAlert {
        CurrentAlert {
            state,
            resolve_from: resolve_ago.map(|ago| base - Duration::seconds(ago)),
        }
    }

    fn holding(since_ago: i64, meets_for: bool, base: DateTime<Utc>) -> Holding {
        Holding {
            since: base - Duration::seconds(since_ago),
            meets_for,
        }
    }

    #[test]
    fn nothing_holding_is_inactive() {
        let base = now();
        assert_eq!(next_step(None, None, base, 60), Step::Inactive);
    }

    #[test]
    fn creates_pending_when_holding_below_for() {
        let base = now();
        assert_eq!(
            next_step(None, Some(holding(10, false, base)), base, 60),
            Step::Pending {
                since: base - Duration::seconds(10)
            }
        );
    }

    #[test]
    fn creates_firing_when_for_already_met() {
        let base = now();
        assert_eq!(
            next_step(None, Some(holding(120, true, base)), base, 60),
            Step::Firing {
                since: base - Duration::seconds(120)
            }
        );
    }

    #[test]
    fn pending_stays_pending_below_for() {
        let base = now();
        let c = cur(AlertState::Pending, None, base);
        assert_eq!(
            next_step(Some(c), Some(holding(30, false, base)), base, 60),
            Step::StayPending {
                since: base - Duration::seconds(30)
            }
        );
    }

    #[test]
    fn pending_promotes_to_firing_when_for_met() {
        let base = now();
        let c = cur(AlertState::Pending, None, base);
        assert_eq!(
            next_step(Some(c), Some(holding(60, true, base)), base, 60),
            Step::ToFiring {
                since: base - Duration::seconds(60)
            }
        );
    }

    #[test]
    fn pending_drops_resolves_immediately() {
        let base = now();
        let c = cur(AlertState::Pending, None, base);
        assert_eq!(next_step(Some(c), None, base, 60), Step::Resolved);
    }

    #[test]
    fn firing_stays_firing_while_holding() {
        let base = now();
        /* Incluso si el valor se mantiene por debajo de for_secs: una vez
         * firing, la condicion sostenida lo mantiene. */
        let c = cur(AlertState::Firing, None, base);
        assert_eq!(
            next_step(Some(c), Some(holding(10, false, base)), base, 60),
            Step::StayFiring {
                since: base - Duration::seconds(10)
            }
        );
    }

    #[test]
    fn firing_opens_resolve_window_when_condition_drops() {
        let base = now();
        let c = cur(AlertState::Firing, None, base);
        assert_eq!(next_step(Some(c), None, base, 60), Step::StartResolving);
    }

    #[test]
    fn firing_with_zero_grace_resolves_immediately() {
        let base = now();
        let c = cur(AlertState::Firing, None, base);
        assert_eq!(next_step(Some(c), None, base, 0), Step::Resolved);
    }

    #[test]
    fn firing_waits_while_within_grace_window() {
        let base = now();
        /* La condicion cayo hace 10s, la ventana es 60s. */
        let c = cur(AlertState::Firing, Some(10), base);
        assert_eq!(next_step(Some(c), None, base, 60), Step::StayResolving);
    }

    #[test]
    fn firing_resolves_when_grace_window_elapses() {
        let base = now();
        let c = cur(AlertState::Firing, Some(60), base);
        assert_eq!(next_step(Some(c), None, base, 60), Step::Resolved);
    }

    #[test]
    fn firing_resolves_at_grace_boundary() {
        let base = now();
        let c = cur(AlertState::Firing, Some(59), base);
        assert_eq!(next_step(Some(c), None, base, 60), Step::StayResolving);
    }

    #[test]
    fn alert_state_strings() {
        assert_eq!(AlertState::Pending.as_str(), "pending");
        assert_eq!(AlertState::Firing.as_str(), "firing");
    }
}
