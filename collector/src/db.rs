use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::FromRow;
use uuid::Uuid;

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
