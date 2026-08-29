use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::FromRow;
use uuid::Uuid;

use crate::alerts::AlertOp;
use crate::health::{CheckOutcome, CurrentCheckState, StateUpdate};
use crate::reboot::detect_reboot;

/*
 * Acceso a PostgreSQL. El pool se crea al startup; las migraciones se
 * embeben en el binario (sqlx::migrate! con `collector/migrations/`) y
 * se aplican al arrancar, asi no hace falta sqlx-cli para desplegar.
 */

pub async fn connect(url: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(url)
        .await
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

/*
 * Registro de agentes implicito (ADR-0003): no hay endpoint de registro
 * en el protocolo del agent (los payloads solo identifican al agent por
 * agent_id), asi que el primer heartbeat o sample valido crea/actualiza
 * la fila en `agents`. La maquina de estados ONLINE/DEGRADED/OFFLINE es
 * Fase 4; aca solo se mantiene first_seen/last_seen.
 *
 * last_seen usa la hora de ARRIBO al servidor (SQL now()), no el
 * timestamp que reporta el agent: para liveness importa "cuando oimos del
 * agent", y el reloj del host puede tener skew (hasta 60s aceptado). Con
 * el dato del cliente, un host con reloj corrido quedaria marcado como
 * offline estando vivo.
 */
pub async fn upsert_agent(pool: &PgPool, agent_id: &Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO agents (agent_id, first_seen, last_seen)
         VALUES ($1, now(), now())
         ON CONFLICT (agent_id) DO UPDATE SET last_seen = now()",
    )
    .bind(agent_id)
    .execute(pool)
    .await?;
    Ok(())
}

/*
 * Ingestion de un sample completo + deteccion de reboot (bloque 4.3), en
 * una sola transaccion:
 *   1. Lock del agente (SELECT ... FOR UPDATE) para serializar los
 *      samples del mismo host: dos ingestiones concurrentes no pueden
 *      leer el mismo uptime previo y duplicar o perder un reboot.
 *   2. Ultimo `system.uptime` conocido del agente (entity NULL).
 *   3. Si el uptime actual cayo mas que la tolerancia -> reboot: se
 *      registra en `reboot_events`.
 *   4. Insert de las metricas con UNNEST de tres arrays paralelos.
 *
 * Los escalares llevan entity = NULL; las metricas por-entidad lo llevan
 * con el label correspondiente.
 */
pub async fn ingest_sample(
    pool: &PgPool,
    agent_id: &Uuid,
    ts: DateTime<Utc>,
    rows: &[(String, Option<String>, f64)],
    current_uptime: Option<f64>,
    min_uptime_drop_secs: f64,
) -> Result<IngestReport, sqlx::Error> {
    if rows.is_empty() {
        return Ok(IngestReport {
            reboot_detected: false,
            uptime_before: None,
            uptime_after: None,
        });
    }

    let names: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
    let entities: Vec<Option<String>> = rows.iter().map(|r| r.1.clone()).collect();
    let values: Vec<f64> = rows.iter().map(|r| r.2).collect();

    let mut tx = pool.begin().await?;

    sqlx::query("SELECT 1 FROM agents WHERE agent_id = $1 FOR UPDATE")
        .bind(agent_id)
        .execute(&mut *tx)
        .await?;

    let previous_uptime: Option<f64> = sqlx::query_scalar(
        "SELECT value FROM metric_samples
         WHERE agent_id = $1 AND metric_name = 'system.uptime' AND entity IS NULL
         ORDER BY ts DESC, id DESC LIMIT 1",
    )
    .bind(agent_id)
    .fetch_optional(&mut *tx)
    .await?;

    let reboot_detected = current_uptime
        .map(|u| detect_reboot(previous_uptime, u, min_uptime_drop_secs))
        .unwrap_or(false);

    sqlx::query(
        "INSERT INTO metric_samples (agent_id, ts, metric_name, entity, value)
         SELECT $1, $2, u.name, u.entity, u.value
         FROM UNNEST($3::text[], $4::text[], $5::float8[]) AS u(name, entity, value)",
    )
    .bind(agent_id)
    .bind(ts)
    .bind(&names)
    .bind(&entities)
    .bind(&values)
    .execute(&mut *tx)
    .await?;

    if reboot_detected {
        sqlx::query(
            "INSERT INTO reboot_events (agent_id, detected_at, sample_ts, uptime_before, uptime_after)
             VALUES ($1, now(), $2, $3, $4)",
        )
        .bind(agent_id)
        .bind(ts)
        .bind(previous_uptime.unwrap_or(0.0))
        .bind(current_uptime.unwrap_or(0.0))
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(IngestReport {
        reboot_detected,
        uptime_before: previous_uptime,
        uptime_after: current_uptime,
    })
}

/*
 * Query API (Fase 4, bloque 1). Endpoints GET de solo lectura. Las
 * consultas explotan los indices de `metric_samples` que ya dejamos para
 * esto en 0001_init.sql (lookup (agent_id, metric_name, entity, ts DESC)).
 */

#[derive(FromRow)]
pub struct AgentRow {
    pub agent_id: Uuid,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

pub async fn list_agents(pool: &PgPool) -> Result<Vec<AgentRow>, sqlx::Error> {
    let rows = sqlx::query_as::<_, AgentRow>(
        "SELECT agent_id, first_seen, last_seen
         FROM agents
         ORDER BY last_seen DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/*
 * Un agente puntual. El estado de conectividad se deriva en el handler
 * (bloque 4.2) a partir de `last_seen`; aca solo se trae el dato.
 */
pub async fn get_agent(pool: &PgPool, agent_id: &Uuid) -> Result<Option<AgentRow>, sqlx::Error> {
    sqlx::query_as::<_, AgentRow>(
        "SELECT agent_id, first_seen, last_seen
         FROM agents
         WHERE agent_id = $1",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await
}

#[derive(Serialize, FromRow)]
pub struct SeriesMeta {
    pub metric_name: String,
    pub entity: Option<String>,
    pub samples: i64,
    pub first_ts: Option<DateTime<Utc>>,
    pub last_ts: Option<DateTime<Utc>>,
    pub latest_value: Option<f64>,
}

/*
 * Series de un agente: una fila por (metric_name, entity) con el conteo de
 * muestras, el rango temporal y el ultimo valor. El ultimo valor sale de
 * una subconsulta correlacionada (mismo agente+metrica+entidad, ts mas
 * reciente) para que la vista "host" del dashboard no tenga que pedir la
 * serie completa.
 */
pub async fn list_agent_series(
    pool: &PgPool,
    agent_id: &Uuid,
) -> Result<Vec<SeriesMeta>, sqlx::Error> {
    let rows = sqlx::query_as::<_, SeriesMeta>(
        "SELECT s.metric_name,
                s.entity,
                COUNT(*)::bigint              AS samples,
                MIN(s.ts)                     AS first_ts,
                MAX(s.ts)                     AS last_ts,
                (SELECT t.value
                   FROM metric_samples t
                  WHERE t.agent_id = $1
                    AND t.metric_name = s.metric_name
                    AND t.entity IS NOT DISTINCT FROM s.entity
                  ORDER BY t.ts DESC
                  LIMIT 1)                    AS latest_value
         FROM metric_samples s
         WHERE s.agent_id = $1
         GROUP BY s.metric_name, s.entity
         ORDER BY s.metric_name, s.entity",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[derive(Serialize, FromRow)]
pub struct SamplePoint {
    pub ts: DateTime<Utc>,
    pub value: f64,
}

/*
 * Serie temporal de una metrica. Filtros opcionales por entity, from y to
 * (ambos inclusive), y limite de puntos. Ordenada por ts ASC (forma
 * natural para graficar). El OR con parametros NULL evita armar SQL
 * dinamico.
 */
#[allow(clippy::too_many_arguments)]
pub async fn query_series(
    pool: &PgPool,
    agent_id: &Uuid,
    metric: &str,
    entity: Option<&str>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<SamplePoint>, sqlx::Error> {
    let rows = sqlx::query_as::<_, SamplePoint>(
        "SELECT ts, value
         FROM metric_samples
         WHERE agent_id = $1
           AND metric_name = $2
           AND ($3::text IS NULL OR entity = $3)
           AND ($4::timestamptz IS NULL OR ts >= $4)
           AND ($5::timestamptz IS NULL OR ts <= $5)
         ORDER BY ts ASC
         LIMIT $6",
    )
    .bind(agent_id)
    .bind(metric)
    .bind(entity)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/*
 * Timeline de reboots (bloque 4.3): eventos detectados de un agent,
 * mas recientes primero.
 */
#[derive(Serialize, FromRow)]
pub struct RebootEvent {
    pub id: i64,
    pub detected_at: DateTime<Utc>,
    pub sample_ts: DateTime<Utc>,
    pub uptime_before: f64,
    pub uptime_after: f64,
}

pub async fn list_reboots(
    pool: &PgPool,
    agent_id: &Uuid,
    limit: i64,
) -> Result<Vec<RebootEvent>, sqlx::Error> {
    let rows = sqlx::query_as::<_, RebootEvent>(
        "SELECT id, detected_at, sample_ts, uptime_before, uptime_after
         FROM reboot_events
         WHERE agent_id = $1
         ORDER BY detected_at DESC
         LIMIT $2",
    )
    .bind(agent_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[derive(Serialize)]
pub struct RebootStats {
    pub count: i64,
    pub last: Option<RebootEvent>,
}

/*
 * Conteo + ultimo reboot de un agent, para el detalle del host (host
 * page del dashboard).
 */
pub async fn reboot_stats(pool: &PgPool, agent_id: &Uuid) -> Result<RebootStats, sqlx::Error> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::bigint FROM reboot_events WHERE agent_id = $1")
            .bind(agent_id)
            .fetch_one(pool)
            .await?;

    let last = sqlx::query_as::<_, RebootEvent>(
        "SELECT id, detected_at, sample_ts, uptime_before, uptime_after
         FROM reboot_events
         WHERE agent_id = $1
         ORDER BY detected_at DESC
         LIMIT 1",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await?;

    Ok(RebootStats { count, last })
}

pub struct IngestReport {
    pub reboot_detected: bool,
    pub uptime_before: Option<f64>,
    pub uptime_after: Option<f64>,
}

/*
 * Alert engine (Fase 5, bloque 5.1): persistencia de reglas declarativas
 * y lectura de las series que evalua el evaluador periodico. La maquina
 * de estados (bloque 5.2) y el historial (bloque 5.3) vienen despues.
 */

#[derive(FromRow)]
pub struct AlertRuleRow {
    pub id: i64,
    pub name: String,
    pub metric_name: String,
    pub entity: Option<String>,
    pub op: String,
    pub threshold: f64,
    pub for_secs: i64,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[allow(clippy::too_many_arguments)]
pub async fn create_rule(
    pool: &PgPool,
    name: &str,
    metric_name: &str,
    entity: Option<&str>,
    op: &str,
    threshold: f64,
    for_secs: i64,
) -> Result<AlertRuleRow, sqlx::Error> {
    sqlx::query_as::<_, AlertRuleRow>(
        "INSERT INTO alert_rules (name, metric_name, entity, op, threshold, for_secs)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, name, metric_name, entity, op, threshold, for_secs, enabled, created_at",
    )
    .bind(name)
    .bind(metric_name)
    .bind(entity)
    .bind(op)
    .bind(threshold)
    .bind(for_secs)
    .fetch_one(pool)
    .await
}

pub async fn list_rules(pool: &PgPool) -> Result<Vec<AlertRuleRow>, sqlx::Error> {
    sqlx::query_as::<_, AlertRuleRow>(
        "SELECT id, name, metric_name, entity, op, threshold, for_secs, enabled, created_at
         FROM alert_rules
         ORDER BY id",
    )
    .fetch_all(pool)
    .await
}

/*
 * Solo las reglas que el evaluador debe considerar en cada ciclo.
 */
pub async fn list_enabled_rules(pool: &PgPool) -> Result<Vec<AlertRuleRow>, sqlx::Error> {
    sqlx::query_as::<_, AlertRuleRow>(
        "SELECT id, name, metric_name, entity, op, threshold, for_secs, enabled, created_at
         FROM alert_rules
         WHERE enabled
         ORDER BY id",
    )
    .fetch_all(pool)
    .await
}

pub async fn delete_rule(pool: &PgPool, rule_id: i64) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM alert_rules WHERE id = $1")
        .bind(rule_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

#[derive(FromRow)]
pub struct RecentSample {
    pub agent_id: Uuid,
    pub ts: DateTime<Utc>,
    pub value: f64,
}

/*
 * Muestras de la ventana que el evaluador pide por regla: la serie de la
 * metrica/entidad de la regla desde `from`. Ordenada por (agent_id, ts
 * ASC) para agrupar por agent en una sola pasada en el evaluador.
 */
pub async fn recent_samples_for_rule(
    pool: &PgPool,
    metric_name: &str,
    entity: Option<&str>,
    from: DateTime<Utc>,
) -> Result<Vec<RecentSample>, sqlx::Error> {
    sqlx::query_as::<_, RecentSample>(
        "SELECT agent_id, ts, value
         FROM metric_samples
         WHERE metric_name = $1
           AND entity IS NOT DISTINCT FROM $2
           AND ts >= $3
         ORDER BY agent_id, ts ASC",
    )
    .bind(metric_name)
    .bind(entity)
    .bind(from)
    .fetch_all(pool)
    .await
}

/*
 * Maquina de estados (bloque 5.2): el evaluador persiste el estado actual
 * por (rule, agent) en `alerts`. Aplicacion de los pasos del ciclo en una
 * sola transaccion: UPSERT (crear/actualizar), arrancar la ventana de
 * resolucion (hysteresis) o borrar la fila (RESOLVED).
 */

#[derive(FromRow)]
pub struct CurrentAlert {
    pub rule_id: i64,
    pub agent_id: Uuid,
    pub state: String,
    pub resolve_from: Option<DateTime<Utc>>,
}

pub async fn list_current_alerts(pool: &PgPool) -> Result<Vec<CurrentAlert>, sqlx::Error> {
    sqlx::query_as::<_, CurrentAlert>(
        "SELECT rule_id, agent_id, state, resolve_from
         FROM alerts",
    )
    .fetch_all(pool)
    .await
}

pub async fn apply_alert_steps(pool: &PgPool, ops: &[AlertOp]) -> Result<(), sqlx::Error> {
    if ops.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for op in ops {
        match op {
            AlertOp::Upsert {
                rule_id,
                agent_id,
                state,
                since,
                event,
            } => {
                sqlx::query(
                    "INSERT INTO alerts (rule_id, agent_id, state, since, checked_at)
                     VALUES ($1, $2, $3, $4, now())
                     ON CONFLICT (rule_id, agent_id)
                     DO UPDATE SET state = EXCLUDED.state,
                                   since = EXCLUDED.since,
                                   resolve_from = NULL,
                                   checked_at = now()",
                )
                .bind(rule_id)
                .bind(agent_id)
                .bind(state.as_str())
                .bind(since)
                .execute(&mut *tx)
                .await?;
                if let Some(ev) = event {
                    sqlx::query(
                        "INSERT INTO alert_events (rule_id, agent_id, from_state, to_state, ts)
                         VALUES ($1, $2, $3, $4, now())",
                    )
                    .bind(rule_id)
                    .bind(agent_id)
                    .bind(ev.from.map(|s| s.as_str()))
                    .bind(ev.to.as_str())
                    .execute(&mut *tx)
                    .await?;
                }
            }
            AlertOp::StartResolving { rule_id, agent_id } => {
                sqlx::query(
                    "UPDATE alerts SET resolve_from = now(), checked_at = now()
                     WHERE rule_id = $1 AND agent_id = $2",
                )
                .bind(rule_id)
                .bind(agent_id)
                .execute(&mut *tx)
                .await?;
            }
            AlertOp::Resolved {
                rule_id,
                agent_id,
                event,
            } => {
                sqlx::query("DELETE FROM alerts WHERE rule_id = $1 AND agent_id = $2")
                    .bind(rule_id)
                    .bind(agent_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(
                    "INSERT INTO alert_events (rule_id, agent_id, from_state, to_state, ts)
                     VALUES ($1, $2, $3, $4, now())",
                )
                .bind(rule_id)
                .bind(agent_id)
                .bind(event.from.map(|s| s.as_str()))
                .bind(event.to.as_str())
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

/*
 * Query API (bloque 5.3): alertas activas (pending/firing) con contexto
 * de la regla, filtrables por agent y estado.
 */

#[derive(Serialize, FromRow)]
pub struct ActiveAlert {
    pub rule_id: i64,
    pub rule_name: String,
    pub metric_name: String,
    pub entity: Option<String>,
    pub op: String,
    pub threshold: f64,
    pub agent_id: Uuid,
    pub state: String,
    pub since: DateTime<Utc>,
    pub resolve_from: Option<DateTime<Utc>>,
    pub checked_at: DateTime<Utc>,
}

pub async fn list_active_alerts(
    pool: &PgPool,
    agent_id: Option<Uuid>,
    state: Option<&str>,
) -> Result<Vec<ActiveAlert>, sqlx::Error> {
    sqlx::query_as::<_, ActiveAlert>(
        "SELECT r.id AS rule_id, r.name AS rule_name, r.metric_name, r.entity,
                r.op, r.threshold, a.agent_id, a.state, a.since, a.resolve_from,
                a.checked_at
         FROM alerts a
         JOIN alert_rules r ON r.id = a.rule_id
         WHERE ($1::uuid IS NULL OR a.agent_id = $1)
           AND ($2::text IS NULL OR a.state = $2)
         ORDER BY a.state, a.since DESC",
    )
    .bind(agent_id)
    .bind(state)
    .fetch_all(pool)
    .await
}

/*
 * Query API (bloque 5.3): historial de transiciones en `alert_events`
 * ("activas y resueltas"): filtrable por agent, regla y rango de tiempo.
 */

#[derive(Serialize, FromRow)]
pub struct AlertEventRow {
    pub id: i64,
    pub rule_id: i64,
    pub rule_name: String,
    pub agent_id: Uuid,
    pub from_state: Option<String>,
    pub to_state: String,
    pub ts: DateTime<Utc>,
}

pub async fn list_alert_history(
    pool: &PgPool,
    agent_id: Option<Uuid>,
    rule_id: Option<i64>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<AlertEventRow>, sqlx::Error> {
    sqlx::query_as::<_, AlertEventRow>(
        "SELECT e.id, e.rule_id, r.name AS rule_name, e.agent_id,
                e.from_state, e.to_state, e.ts
         FROM alert_events e
         JOIN alert_rules r ON r.id = e.rule_id
         WHERE ($1::uuid IS NULL OR e.agent_id = $1)
           AND ($2::bigint IS NULL OR e.rule_id = $2)
           AND ($3::timestamptz IS NULL OR e.ts >= $3)
           AND ($4::timestamptz IS NULL OR e.ts <= $4)
         ORDER BY e.ts DESC, e.id DESC
         LIMIT $5",
    )
    .bind(agent_id)
    .bind(rule_id)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/*
 * Health checks (Fase 6, bloque 6.1): definiciones, resultados y estado
 * actual. Una corrida es un INSERT en `health_check_results` + UPSERT de
 * `health_check_states` (transicion up/down calculada en health::next_state,
 * con `since` conservado mientras no cambie), en una sola transaccion.
 */

#[derive(Debug, FromRow)]
pub struct HealthCheckRow {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub target: String,
    pub interval_secs: i64,
    pub timeout_secs: i64,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

pub async fn list_enabled_checks(pool: &PgPool) -> Result<Vec<HealthCheckRow>, sqlx::Error> {
    sqlx::query_as::<_, HealthCheckRow>(
        "SELECT id, name, kind, target, interval_secs, timeout_secs, enabled, created_at
         FROM health_checks
         WHERE enabled
         ORDER BY id",
    )
    .fetch_all(pool)
    .await
}

pub async fn list_health_checks(pool: &PgPool) -> Result<Vec<HealthCheckView>, sqlx::Error> {
    sqlx::query_as::<_, HealthCheckView>(
        "SELECT c.id, c.name, c.kind, c.target, c.interval_secs, c.timeout_secs,
                c.enabled, c.created_at,
                s.state, s.since, s.last_checked_at, s.last_ok, s.last_latency_ms,
                s.last_detail
         FROM health_checks c
         LEFT JOIN health_check_states s ON s.check_id = c.id
         ORDER BY c.id",
    )
    .fetch_all(pool)
    .await
}

pub async fn get_health_check(
    pool: &PgPool,
    check_id: i64,
) -> Result<Option<HealthCheckRow>, sqlx::Error> {
    sqlx::query_as::<_, HealthCheckRow>(
        "SELECT id, name, kind, target, interval_secs, timeout_secs, enabled, created_at
         FROM health_checks
         WHERE id = $1",
    )
    .bind(check_id)
    .fetch_optional(pool)
    .await
}

pub async fn create_health_check(
    pool: &PgPool,
    draft: &crate::health::CheckDraft,
) -> Result<HealthCheckRow, sqlx::Error> {
    sqlx::query_as::<_, HealthCheckRow>(
        "INSERT INTO health_checks (name, kind, target, interval_secs, timeout_secs, enabled)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, name, kind, target, interval_secs, timeout_secs, enabled, created_at",
    )
    .bind(&draft.name)
    .bind(draft.kind.as_str())
    .bind(&draft.target)
    .bind(draft.interval_secs)
    .bind(draft.timeout_secs)
    .bind(draft.enabled)
    .fetch_one(pool)
    .await
}

pub async fn delete_health_check(pool: &PgPool, check_id: i64) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM health_checks WHERE id = $1")
        .bind(check_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn get_check_state(
    pool: &PgPool,
    check_id: i64,
) -> Result<Option<CurrentCheckState>, sqlx::Error> {
    sqlx::query_as::<_, CurrentCheckState>(
        "SELECT state, since
         FROM health_check_states
         WHERE check_id = $1",
    )
    .bind(check_id)
    .fetch_optional(pool)
    .await
}

/*
 * Ultima corrida por check, para el scheduler calcula el vencimiento.
 */
pub async fn latest_check_times(
    pool: &PgPool,
) -> Result<std::collections::HashMap<i64, DateTime<Utc>>, sqlx::Error> {
    let rows: Vec<(i64, DateTime<Utc>)> =
        sqlx::query_as("SELECT check_id, MAX(ts) FROM health_check_results GROUP BY check_id")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

pub async fn apply_check_outcome(
    pool: &PgPool,
    check_id: i64,
    outcome: &CheckOutcome,
    update: &StateUpdate,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO health_check_results (check_id, ts, ok, latency_ms, detail)
         VALUES ($1, now(), $2, $3, $4)",
    )
    .bind(check_id)
    .bind(outcome.ok)
    .bind(outcome.latency_ms)
    .bind(&outcome.detail)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO health_check_states
             (check_id, state, since, last_checked_at, last_ok, last_latency_ms, last_detail)
         VALUES ($1, $2, $3, now(), $4, $5, $6)
         ON CONFLICT (check_id) DO UPDATE SET
             state = EXCLUDED.state,
             since = EXCLUDED.since,
             last_checked_at = EXCLUDED.last_checked_at,
             last_ok = EXCLUDED.last_ok,
             last_latency_ms = EXCLUDED.last_latency_ms,
             last_detail = EXCLUDED.last_detail",
    )
    .bind(check_id)
    .bind(update.state)
    .bind(update.since)
    .bind(outcome.ok)
    .bind(outcome.latency_ms)
    .bind(&outcome.detail)
    .execute(&mut *tx)
    .await?;

    tx.commit().await
}

#[derive(Serialize, FromRow)]
pub struct HealthCheckView {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub target: String,
    pub interval_secs: i64,
    pub timeout_secs: i64,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub state: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_ok: Option<bool>,
    pub last_latency_ms: Option<i64>,
    pub last_detail: Option<String>,
}

#[derive(Serialize, FromRow)]
pub struct HealthResultRow {
    pub id: i64,
    pub check_id: i64,
    pub ts: DateTime<Utc>,
    pub ok: bool,
    pub latency_ms: i64,
    pub detail: String,
}

pub async fn list_health_results(
    pool: &PgPool,
    check_id: i64,
    limit: i64,
) -> Result<Vec<HealthResultRow>, sqlx::Error> {
    sqlx::query_as::<_, HealthResultRow>(
        "SELECT id, check_id, ts, ok, latency_ms, detail
         FROM health_check_results
         WHERE check_id = $1
         ORDER BY ts DESC, id DESC
         LIMIT $2",
    )
    .bind(check_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}
