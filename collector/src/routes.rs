use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::HeaderMap,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde_json::json;
use sqlx::PgPool;

use crate::auth::check_bearer;
use crate::config::Config;
use crate::db;
use crate::error::ApiError;
use crate::models::{HeartbeatPayload, MetricsPayload, Validate};
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

impl Config {
    fn limits(&self) -> TimeLimits {
        TimeLimits {
            future_skew_secs: self.max_future_skew_secs,
            max_past_age_secs: self.max_past_age_secs,
        }
    }
}

fn now_epoch_secs() -> i64 {
    use chrono::Utc;
    Utc::now().timestamp()
}
