use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::Config;
use crate::db::{self, AlertRuleRow};
use crate::error::ApiError;

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
    pub name: String,
    pub metric_name: String,
    pub entity: Option<String>,
    pub op: CondOp,
    pub threshold: f64,
    pub for_secs: i64,
}

impl AlertRule {
    pub fn from_row(row: &AlertRuleRow) -> Option<AlertRule> {
        Some(AlertRule {
            name: row.name.clone(),
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

pub struct EvalSummary {
    pub rules: usize,
    pub agents: usize,
    pub holding: usize,
}

/*
 * Un ciclo de evaluacion: reglas habilitadas -> ventana de samples por
 * (regla, agent) -> veredicto. Aun no hay transiciones ni persistencia.
 */
pub async fn eval_cycle(pool: &PgPool, config: &Config) -> Result<EvalSummary, sqlx::Error> {
    let now = Utc::now();
    let rows = db::list_enabled_rules(pool).await?;
    let mut summary = EvalSummary {
        rules: rows.len(),
        agents: 0,
        holding: 0,
    };

    if rows.is_empty() {
        return Ok(summary);
    }

    let from = now - Duration::seconds(config.alert_lookback_secs);

    for row in &rows {
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
            summary.agents += 1;
            if let SeriesVerdict::Holding {
                since,
                holds_for_secs,
                meets_for,
            } = evaluate_series(pts, &rule, now)
            {
                summary.holding += 1;
                tracing::info!(
                    rule = %rule.name,
                    %agent_id,
                    ?since,
                    holds_for_secs,
                    meets_for,
                    "condicion de alerta sostenida"
                );
            }
        }
    }

    Ok(summary)
}

/*
 * Lanza el evaluador periodico. SIEMPRE consume un tick inmediato y
 * luego los intervalos; ante un fallo del ciclo (p.ej. DB caida) loguea
 * y sigue en el siguiente tick.
 */
pub fn spawn_evaluator(pool: PgPool, config: Arc<Config>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(
            config.alert_eval_interval_secs.max(1) as u64,
        ));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            match eval_cycle(&pool, &config).await {
                Ok(s) => tracing::debug!(
                    rules = s.rules,
                    agents = s.agents,
                    holding = s.holding,
                    "ciclo de alertas evaluado"
                ),
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
            name: "r".into(),
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
}
