use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::rejection::QueryRejection,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{delete, get, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgPool;

use crate::alerts::CreateRule;
use crate::auth::check_bearer;
use crate::config::Config;
use crate::db;
use crate::error::ApiError;
use crate::health::CreateHealthCheck;
use crate::models::{HeartbeatPayload, MetricsPayload, Validate};
use crate::query::{AlertsQuery, CheckResultsQuery, HistoryQuery, RebootsQuery, SeriesQuery};
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
            "/api/v1/agents/{agent_id}/reboots",
            get(agent_reboots_handler),
        )
        .route(
            "/api/v1/agents/{agent_id}/metrics",
            get(agent_metrics_handler),
        )
        .route(
            "/api/v1/agents/{agent_id}/metrics/{metric}",
            get(metric_series_handler),
        )
        .route("/api/v1/alerts/rules", get(list_rules_handler))
        .route("/api/v1/alerts/rules", post(create_rule_handler))
        .route(
            "/api/v1/alerts/rules/{rule_id}",
            delete(delete_rule_handler),
        )
        .route("/api/v1/alerts", get(list_alerts_handler))
        .route("/api/v1/alerts/history", get(alert_history_handler))
        .route("/api/v1/health/checks", get(list_checks_handler))
        .route("/api/v1/health/checks", post(create_check_handler))
        .route(
            "/api/v1/health/checks/{check_id}",
            delete(delete_check_handler),
        )
        .route(
            "/api/v1/health/checks/{check_id}/results",
            get(check_results_handler),
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
    let current_uptime = rows
        .iter()
        .find(|(name, entity, _)| name == "system.uptime" && entity.is_none())
        .map(|(_, _, value)| *value);

    let report = db::ingest_sample(
        &state.pool,
        &agent_id,
        ts,
        &rows,
        current_uptime,
        state.config.reboot_min_uptime_drop_secs,
    )
    .await?;

    if report.reboot_detected {
        tracing::info!(
            %agent_id,
            uptime_before = report.uptime_before,
            uptime_after = report.uptime_after,
            "reboot detectado"
        );
    }

    tracing::debug!(%agent_id, n = rows.len(), "sample almacenado");
    Ok(Json(json!({
        "status": "ok",
        "stored": rows.len(),
        "reboot_detected": report.reboot_detected,
    })))
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
    let stats = db::reboot_stats(&state.pool, &agent_id).await?;

    let mut body = agent_json(&agent, Utc::now(), &limits);
    body["reboot_count"] = json!(stats.count);
    body["last_reboot"] = stats
        .last
        .map(|r| json!({"detected_at": r.detected_at, "sample_ts": r.sample_ts}))
        .unwrap_or_else(|| json!(null));

    let state = body["state"].as_str().unwrap_or("?");
    tracing::debug!(%agent_id, state, "query: detalle de agente");
    Ok(Json(body))
}

async fn agent_reboots_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
    params: std::result::Result<Query<RebootsQuery>, QueryRejection>,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let agent_id = parse_agent_uuid(&agent_id)?;
    let limit = params
        .map_err(|_| ApiError::bad_request("invalid_query", "parametros de query invalidos"))?
        .0
        .into_limit()?;

    let reboots = db::list_reboots(&state.pool, &agent_id, limit).await?;
    tracing::debug!(%agent_id, n = reboots.len(), "query: reboots de agente");
    Ok(Json(
        json!({"agent_id": agent_id, "reboots": reboots, "count": reboots.len()}),
    ))
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

/*
 * Alert engine (Fase 5, bloque 5.1): gestion de reglas declarativas.
 * El evaluador periodico las lee via db::list_enabled_rules; aca solo se
 * crean, listan y borran con el mismo bearer token que el resto.
 */

async fn create_rule_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let payload: CreateRule = serde_json::from_slice(&body).map_err(|e| {
        ApiError::bad_request("invalid_rule", format!("payload de regla invalido: {e}"))
    })?;
    let draft = payload.into_draft()?;

    let row = db::create_rule(
        &state.pool,
        &draft.name,
        &draft.metric_name,
        draft.entity.as_deref(),
        draft.op.as_str(),
        draft.threshold,
        draft.for_secs,
    )
    .await
    .map_err(rule_create_err)?;

    tracing::info!(id = row.id, rule = %row.name, "regla de alerta creada");
    Ok((StatusCode::CREATED, Json(rule_json(&row))))
}

async fn list_rules_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let rules = db::list_rules(&state.pool).await?;
    let body: Vec<_> = rules.iter().map(rule_json).collect();
    tracing::debug!(n = body.len(), "query: reglas de alerta");
    Ok(Json(json!({"rules": body, "count": body.len()})))
}

async fn list_alerts_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: std::result::Result<Query<AlertsQuery>, QueryRejection>,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let filter = params
        .map_err(|_| ApiError::bad_request("invalid_query", "parametros de query invalidos"))?
        .0
        .into_filter()?;

    let alerts =
        db::list_active_alerts(&state.pool, filter.agent_id, filter.state.as_deref()).await?;
    tracing::debug!(n = alerts.len(), "query: alertas activas");
    Ok(Json(json!({"alerts": alerts, "count": alerts.len()})))
}

async fn alert_history_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: std::result::Result<Query<HistoryQuery>, QueryRejection>,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let filter = params
        .map_err(|_| ApiError::bad_request("invalid_query", "parametros de query invalidos"))?
        .0
        .into_filter()?;

    let events = db::list_alert_history(
        &state.pool,
        filter.agent_id,
        filter.rule_id,
        filter.from,
        filter.to,
        filter.limit,
    )
    .await?;
    tracing::debug!(n = events.len(), "query: historial de alertas");
    Ok(Json(json!({"events": events, "count": events.len()})))
}

/*
 * Health checks (Fase 6, bloque 6.1): creacion, listado con estado,
 * borrado e historial de corridas. Mismo bearer token que el resto.
 */

async fn create_check_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let payload: CreateHealthCheck = serde_json::from_slice(&body).map_err(|e| {
        ApiError::bad_request("invalid_check", format!("payload de check invalido: {e}"))
    })?;
    let draft = payload.into_draft(state.config.health_default_timeout_secs)?;

    let row = db::create_health_check(&state.pool, &draft)
        .await
        .map_err(check_create_err)?;

    tracing::info!(id = row.id, check = %row.name, "health check creado");
    Ok((StatusCode::CREATED, Json(check_json(&row))))
}

async fn list_checks_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let checks = db::list_health_checks(&state.pool).await?;
    let body: Vec<_> = checks.iter().map(check_view_json).collect();
    tracing::debug!(n = body.len(), "query: health checks");
    Ok(Json(json!({"checks": body, "count": body.len()})))
}

async fn delete_check_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(check_id): Path<String>,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let check_id: i64 = check_id
        .trim()
        .parse()
        .map_err(|_| ApiError::bad_request("invalid_check_id", "check_id debe ser un entero"))?;

    if !db::delete_health_check(&state.pool, check_id).await? {
        return Err(ApiError::not_found("unknown_check", "check no encontrado"));
    }

    tracing::info!(check_id, "health check borrado");
    Ok(Json(json!({"status": "ok"})))
}

async fn check_results_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(check_id): Path<String>,
    params: std::result::Result<Query<CheckResultsQuery>, QueryRejection>,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let check_id: i64 = check_id
        .trim()
        .parse()
        .map_err(|_| ApiError::bad_request("invalid_check_id", "check_id debe ser un entero"))?;

    if db::get_health_check(&state.pool, check_id).await?.is_none() {
        return Err(ApiError::not_found("unknown_check", "check no encontrado"));
    }

    let limit = params
        .map_err(|_| ApiError::bad_request("invalid_query", "parametros de query invalidos"))?
        .0
        .into_limit()?;

    let results = db::list_health_results(&state.pool, check_id, limit).await?;
    tracing::debug!(check_id, n = results.len(), "query: resultados de check");
    Ok(Json(
        json!({"check_id": check_id, "results": results, "count": results.len()}),
    ))
}

/*
 * "name" tiene UNIQUE en la DB: un duplicado es un 400 explicito, no un
 * 500. Cualquier otro error sqlx se propaga como internal_error.
 */
fn check_create_err(e: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(db) = &e {
        if db.is_unique_violation() {
            return ApiError::bad_request(
                "check_already_exists",
                "ya existe un check con ese nombre",
            );
        }
    }
    e.into()
}

fn check_json(row: &db::HealthCheckRow) -> serde_json::Value {
    json!({
        "id": row.id,
        "name": row.name,
        "kind": row.kind,
        "target": row.target,
        "interval_secs": row.interval_secs,
        "timeout_secs": row.timeout_secs,
        "enabled": row.enabled,
        "created_at": row.created_at,
    })
}

fn check_view_json(view: &db::HealthCheckView) -> serde_json::Value {
    json!({
        "id": view.id,
        "name": view.name,
        "kind": view.kind,
        "target": view.target,
        "interval_secs": view.interval_secs,
        "timeout_secs": view.timeout_secs,
        "enabled": view.enabled,
        "created_at": view.created_at,
        "state": view.state,
        "since": view.since,
        "last_checked_at": view.last_checked_at,
        "last_ok": view.last_ok,
        "last_latency_ms": view.last_latency_ms,
        "last_detail": view.last_detail,
    })
}

async fn delete_rule_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(rule_id): Path<String>,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let rule_id: i64 = rule_id
        .trim()
        .parse()
        .map_err(|_| ApiError::bad_request("invalid_rule_id", "rule_id debe ser un entero"))?;

    if !db::delete_rule(&state.pool, rule_id).await? {
        return Err(ApiError::not_found("unknown_rule", "regla no encontrada"));
    }

    tracing::info!(rule_id, "regla de alerta borrada");
    Ok(Json(json!({"status": "ok"})))
}

fn rule_json(row: &db::AlertRuleRow) -> serde_json::Value {
    json!({
        "id": row.id,
        "name": row.name,
        "metric_name": row.metric_name,
        "entity": row.entity,
        "op": row.op,
        "threshold": row.threshold,
        "for_secs": row.for_secs,
        "enabled": row.enabled,
        "created_at": row.created_at,
    })
}

/*
 * "name" tiene UNIQUE en la DB: un duplicado es un 400 explícito, no un
 * 500. Cualquier otro error sqlx se propaga como internal_error.
 */
fn rule_create_err(e: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(db) = &e {
        if db.is_unique_violation() {
            return ApiError::bad_request(
                "rule_already_exists",
                "ya existe una regla con ese nombre",
            );
        }
    }
    e.into()
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
