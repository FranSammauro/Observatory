use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::FromRow;
use uuid::Uuid;

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
 * Inserta un sample completo en una transaccion, en una sola sentencia
 * via UNNEST de tres arrays paralelos (metric_name, entity, value).
 * Los escalares llevan entity = NULL; las metricas por-entidad lo llevan
 * con el label correspondiente.
 */
pub async fn insert_metrics(
    pool: &PgPool,
    agent_id: &Uuid,
    ts: DateTime<Utc>,
    rows: &[(String, Option<String>, f64)],
) -> Result<(), sqlx::Error> {
    if rows.is_empty() {
        return Ok(());
    }

    let names: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
    let entities: Vec<Option<String>> = rows.iter().map(|r| r.1.clone()).collect();
    let values: Vec<f64> = rows.iter().map(|r| r.2).collect();

    let mut tx = pool.begin().await?;
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
    tx.commit().await?;
    Ok(())
}

/*
 * Query API (Fase 4, bloque 1). Endpoints GET de solo lectura. Las
 * consultas explotan los indices de `metric_samples` que ya dejamos para
 * esto en 0001_init.sql (lookup (agent_id, metric_name, entity, ts DESC)).
 */

#[derive(Serialize, FromRow)]
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
