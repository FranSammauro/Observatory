use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::rejection::QueryRejection,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgPool;

use crate::auth::check_bearer;
use crate::config::Config;
use crate::db;
use crate::error::ApiError;
use crate::models::{HeartbeatPayload, MetricsPayload, Validate};
use crate::query::SeriesQuery;
use crate::state::{connectivity_state, StateLimits};
use crate::validation::{utc_from_unix_ts, TimeLimits};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
}

type Result<T> = std::result::Result<T, ApiError>;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/v1/agents/heartbeat", post(heartbeat_handler))
        .route("/api/v1/metrics", post(metrics_handler))
        .route("/api/v1/agents", get(agents_list_handler))
        .route("/api/v1/agents/{agent_id}", get(agent_detail_handler))
        .route(
            "/api/v1/agents/{agent_id}/metrics",
            get(agent_metrics_handler),
        )
        .route(
            "/api/v1/agents/{agent_id}/metrics/{metric}",
            get(metric_series_handler),
        )
        .layer(DefaultBodyLimit::max(state.config.max_body_bytes))
        .with_state(state)
}

async fn healthz(State(state): State<AppState>) -> Result<impl IntoResponse> {
    let healthy = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .is_ok();
    if healthy {
        Ok(Json(json!({"status": "ok"})))
    } else {
        Err(ApiError::internal("base de datos no accesible"))
    }
}

async fn heartbeat_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let payload: HeartbeatPayload = serde_json::from_slice(&body).map_err(|e| {
        ApiError::bad_request(
            "invalid_json",
            format!("payload de heartbeat invalido: {e}"),
        )
    })?;
    payload.validate()?;

    let agent_id = uuid::Uuid::parse_str(payload.agent_id.trim())
        .map_err(|_| ApiError::bad_request("invalid_agent_id", "agent_id no es un UUID valido"))?;

    // El heartbeat no persiste metricas: validamos la ventana temporal
    // (rechaza relojes/garbage fuera de rango) pero last_seen usa la hora
    // de arribe al servidor (ver db::upsert_agent).
    utc_from_unix_ts(payload.timestamp, now_epoch_secs(), &state.config.limits())?;

    db::upsert_agent(&state.pool, &agent_id).await?;

    tracing::debug!(%agent_id, "heartbeat registrado");
    Ok(Json(json!({"status": "ok"})))
}

async fn metrics_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let payload: MetricsPayload = serde_json::from_slice(&body).map_err(|e| {
        ApiError::bad_request("invalid_json", format!("payload de metricas invalido: {e}"))
    })?;
    payload.validate()?;

    let agent_id = uuid::Uuid::parse_str(payload.agent_id.trim())
        .map_err(|_| ApiError::bad_request("invalid_agent_id", "agent_id no es un UUID valido"))?;

    let ts = utc_from_unix_ts(payload.timestamp, now_epoch_secs(), &state.config.limits())?;

    db::upsert_agent(&state.pool, &agent_id).await?;

    let rows = payload.to_metric_rows();
    db::insert_metrics(&state.pool, &agent_id, ts, &rows).await?;

    tracing::debug!(%agent_id, n = rows.len(), "sample almacenado");
    Ok(Json(json!({"status": "ok", "stored": rows.len()})))
}

/*
 * Query API (Fase 4, bloque 1). Endpoints GET de solo lectura, tambien
 * tras el bearer token compartido (lee datos de la misma plataforma).
 */

async fn agents_list_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let agents = db::list_agents(&state.pool).await?;
    let limits = state.config.state_limits();
    let now = Utc::now();

    let body: Vec<_> = agents.iter().map(|a| agent_json(a, now, &limits)).collect();

    tracing::debug!(n = agents.len(), "query: lista de agentes");
    Ok(Json(json!({"agents": body, "count": body.len()})))
}

async fn agent_detail_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let agent_id = parse_agent_uuid(&agent_id)?;
    let agent = db::get_agent(&state.pool, &agent_id)
        .await?
        .ok_or_else(|| ApiError::not_found("unknown_agent", "agent no registrado"))?;

    let limits = state.config.state_limits();
    let body = agent_json(&agent, Utc::now(), &limits);
    let state = body["state"].as_str().unwrap_or("?");
    tracing::debug!(%agent_id, state, "query: detalle de agente");
    Ok(Json(body))
}

/*
 * Serializa un agente con su estado de conectividad derivado (bloque 4.2).
 * El estado es funcion pura de la antiguedad de `last_seen`, calculado al
 * leer, no persistido.
 */
fn agent_json(agent: &db::AgentRow, now: DateTime<Utc>, limits: &StateLimits) -> serde_json::Value {
    let age_secs = now.signed_duration_since(agent.last_seen).num_seconds();
    json!({
        "agent_id": agent.agent_id,
        "first_seen": agent.first_seen,
        "last_seen": agent.last_seen,
        "last_seen_age_secs": age_secs,
        "state": connectivity_state(agent.last_seen, now, limits).as_str(),
    })
}

async fn agent_metrics_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let agent_id = parse_agent_uuid(&agent_id)?;
    let series = db::list_agent_series(&state.pool, &agent_id).await?;
    tracing::debug!(%agent_id, n = series.len(), "query: series de agente");
    Ok(Json(
        json!({"agent_id": agent_id, "series": series, "count": series.len()}),
    ))
}

async fn metric_series_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((agent_id, metric)): Path<(String, String)>,
    params: std::result::Result<Query<SeriesQuery>, QueryRejection>,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let agent_id = parse_agent_uuid(&agent_id)?;
    if metric.trim().is_empty() {
        return Err(ApiError::bad_request(
            "invalid_metric_name",
            "metric no puede estar vacia",
        ));
    }

    let filter = params
        .map_err(|_| ApiError::bad_request("invalid_query", "parametros de query invalidos"))?
        .0
        .into_filter()?;

    let points = db::query_series(
        &state.pool,
        &agent_id,
        &metric,
        filter.entity.as_deref(),
        filter.from,
        filter.to,
        filter.limit,
    )
    .await?;

    tracing::debug!(%agent_id, metric, n = points.len(), "query: serie de metricas");
    Ok(Json(json!({
        "agent_id": agent_id,
        "metric": metric,
        "entity": filter.entity,
        "from": filter.from,
        "to": filter.to,
        "count": points.len(),
        "points": points,
    })))
}

fn parse_agent_uuid(raw: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(raw.trim())
        .map_err(|_| ApiError::bad_request("invalid_agent_id", "agent_id no es un UUID valido"))
}

impl Config {
    fn limits(&self) -> TimeLimits {
        TimeLimits {
            future_skew_secs: self.max_future_skew_secs,
            max_past_age_secs: self.max_past_age_secs,
        }
    }

    fn state_limits(&self) -> StateLimits {
        StateLimits {
            online_secs: self.state_online_secs,
            degraded_secs: self.state_degraded_secs,
        }
    }
}

fn now_epoch_secs() -> i64 {
    use chrono::Utc;
    Utc::now().timestamp()
}
