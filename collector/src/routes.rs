use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::rejection::QueryRejection,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::{delete, get, post},
    Router,
};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::broadcast;
use tower_http::services::{ServeDir, ServeFile};

use crate::alerts::CreateRule;
use crate::auth::{check_bearer, check_bearer_str};
use crate::config::Config;
use crate::db;
use crate::error::ApiError;
use crate::events::EventBus;
use crate::health::CreateHealthCheck;
use crate::models::{HeartbeatPayload, MetricsPayload, Validate};
use crate::query::{
    AlertsQuery, CheckResultsQuery, HistoryQuery, RebootsQuery, SeriesQuery, TimelineEntry,
    TimelineFilter, TimelineQuery,
};
use crate::ratelimit::{RateLimiter, RatePolicy, Take};
use crate::state::{connectivity_state, StateLimits};
use crate::validation::{utc_from_unix_ts, TimeLimits};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub events: EventBus,
    pub limiter: Arc<RateLimiter>,
}

type Result<T> = std::result::Result<T, ApiError>;

pub fn build_router(state: AppState) -> Router {
    let dashboard_dir = state.config.dashboard_dir.clone();
    let index_path = format!("{dashboard_dir}/index.html");
    let limiter = state.limiter.clone();
    Router::new()
        .route("/api/v1/agents/heartbeat", post(heartbeat_handler))
        .route("/api/v1/metrics", post(metrics_handler))
        /* Rate limiting (Fase 8, bloque 8.1): solo sobre los endpoints de
         * ingestion (los que los agents golpean repetido). route_layer
         * afecta solo a las rutas registradas hasta este punto; /healthz
         * se registra despues y queda exento. */
        .route_layer(middleware::from_fn_with_state(
            limiter,
            rate_limit_middleware,
        ))
        .route("/healthz", get(healthz))
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
        .route("/api/v1/events", get(events_handler))
        .route("/api/v1/events/history", get(event_history_handler))
        .route("/api/v1/health/summary", get(health_summary_handler))
        .layer(DefaultBodyLimit::max(state.config.max_body_bytes))
        .with_state(state)
        .fallback_service(
            ServeDir::new(&dashboard_dir).not_found_service(ServeFile::new(index_path)),
        )
}

/*
 * Rate limiting por IP (Fase 8, bloque 8.1). Clave = IP de origen del
 * socket; cuando no hay ConnectInfo (tests unitarios / handshakes
 * raros) se cae a una clave compartida y el limiter de policy deshabilitada
 * simplemente deja pasar.
 */
async fn rate_limit_middleware(
    State(limiter): State<Arc<RateLimiter>>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let key = addr.ip().to_string();
    let take = limiter.allow(key);
    if matches!(take, Take::Denied) {
        return too_many_requests(limiter.policy());
    }
    next.run(req).await
}

fn too_many_requests(policy: RatePolicy) -> Response {
    let retry_after = if policy.rate_per_sec > 0.0 {
        1.0 / policy.rate_per_sec
    } else {
        0.0
    };
    let body = Json(json!({
        "error": {
            "code": "rate_limited",
            "message": "demasiadas peticiones, reintente en un momento",
        }
    }));
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(axum::http::header::RETRY_AFTER, format!("{retry_after:.1}"))],
        body,
    )
        .into_response()
}

/*
 * Dashboard (Fase 7, bloque 7.1): la UI estatica (HTML/CSS/JS vanilla)
 * vive en `OBS_DASHBOARD_DIR` (default `dashboard/`, relativo a CWD) y se
 * sirve por cualquier ruta no capturada por la API. Sirve `index.html`
 * como SPA fallback para gaps de ruta del navegador; las rutas explicitas
 * (la API y `/healthz`) siempre ganan por precedencia. `index.html` al
 * servir un path de la SPA que no existe en disco (patron que documenta
 * el propio tower-http).
 */

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
 * Historial unificado + summary de salud (Fase 6, bloque 6.3).
 *
 * `GET /api/v1/events/history` es el timeline del dashboard: cruza las
 * cuatro fuentes de eventos (alertas, health checks, reboots y
 * conectividad), en orden cronologico desc y acotado por `limit`. Los
 * eventos respetan el mismo shape que el WebSocket (type/ts, etc.)
 *
 * `GET /api/v1/health/summary` agrega el estado de la plataforma en un
 * solo GET: conectividad derivada de los agentes (bloque 4.2), estado
 * actual de los checks (bloque 6.1) y alertas pending/firing (bloque 5).
 */

fn timeline_entry(
    kind: &'static str,
    ts: DateTime<Utc>,
    payload: serde_json::Value,
) -> TimelineEntry {
    TimelineEntry { kind, ts, payload }
}

async fn event_history_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: std::result::Result<Query<TimelineQuery>, QueryRejection>,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let filter: TimelineFilter = params
        .map_err(|_| ApiError::bad_request("invalid_query", "parametros de query invalidos"))?
        .0
        .into_filter()?;

    let limit = filter.limit;

    let alerts =
        db::list_alert_history(&state.pool, filter.agent_id, None, None, None, limit).await?;
    let health = db::list_recent_health_results(&state.pool, limit).await?;
    let reboots = db::list_recent_reboots(&state.pool, filter.agent_id, limit).await?;
    let connectivity = db::list_connectivity_events(&state.pool, filter.agent_id, limit).await?;

    let mut entries: Vec<TimelineEntry> = Vec::with_capacity(alerts.len() * 4);
    entries.extend(alerts.iter().map(|e| {
        timeline_entry(
            "alert_event",
            e.ts,
            json!({
                "type": "alert_event",
                "rule_id": e.rule_id,
                "rule_name": e.rule_name,
                "agent_id": e.agent_id,
                "from_state": e.from_state,
                "to_state": e.to_state,
                "ts": e.ts,
            }),
        )
    }));
    entries.extend(health.iter().map(|h| {
        timeline_entry(
            "health_result",
            h.ts,
            json!({
                "type": "health_result",
                "check_id": h.check_id,
                "check_name": h.check_name,
                "ok": h.ok,
                "latency_ms": h.latency_ms,
                "detail": h.detail,
                "ts": h.ts,
            }),
        )
    }));
    entries.extend(reboots.iter().map(|r| {
        timeline_entry(
            "reboot_event",
            r.detected_at,
            json!({
                "type": "reboot_event",
                "agent_id": r.agent_id,
                "uptime_before": r.uptime_before,
                "uptime_after": r.uptime_after,
                "ts": r.detected_at,
            }),
        )
    }));
    entries.extend(connectivity.iter().map(|c| {
        timeline_entry(
            "connectivity_event",
            c.ts,
            json!({
                "type": "connectivity_event",
                "agent_id": c.agent_id,
                "from_state": c.from_state,
                "to_state": c.to_state,
                "ts": c.ts,
            }),
        )
    }));

    let items = crate::query::merge_timeline(entries, limit as usize);
    let events: Vec<serde_json::Value> = items.into_iter().map(|e| e.payload).collect();
    tracing::debug!(n = events.len(), "query: historial unificado de eventos");
    Ok(Json(json!({"events": events, "count": events.len()})))
}

async fn health_summary_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse> {
    check_bearer(&headers, &state.config.auth_token)?;

    let agents = db::list_agents_liveness(&state.pool).await?;
    let now = Utc::now();
    let limits = state.config.state_limits();
    let mut agents_by_state = std::collections::HashMap::new();
    for a in &agents {
        let st = connectivity_state(a.last_seen, now, &limits)
            .as_str()
            .to_string();
        *agents_by_state.entry(st).or_insert(0i64) += 1;
    }

    let check_states = db::count_check_states(&state.pool).await?;
    let alert_counts = db::count_alert_states(&state.pool).await?;
    let checks_total = db::count_health_checks(&state.pool).await?;

    let up = check_state_count(&check_states, "up");
    let down = check_state_count(&check_states, "down");

    Ok(Json(json!({
        "agents": {
            "total": agents.len(),
            "online": agents_by_state.get("online").copied().unwrap_or(0),
            "degraded": agents_by_state.get("degraded").copied().unwrap_or(0),
            "offline": agents_by_state.get("offline").copied().unwrap_or(0),
        },
        "checks": {
            "total": checks_total,
            "up": up,
            "down": down,
            "unknown": checks_total - up - down,
        },
        "alerts": {
            "total": alert_counts.iter().map(|c| c.count).sum::<i64>(),
            "pending": alert_state_count(&alert_counts, "pending"),
            "firing": alert_state_count(&alert_counts, "firing"),
        },
    })))
}

fn check_state_count(rows: &[db::CheckStateCount], target: &str) -> i64 {
    rows.iter()
        .find(|c| c.state.as_deref() == Some(target))
        .map(|c| c.count)
        .unwrap_or(0)
}

fn alert_state_count(rows: &[db::AlertStateCount], target: &str) -> i64 {
    rows.iter()
        .find(|c| c.state.as_str() == target)
        .map(|c| c.count)
        .unwrap_or(0)
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
}

fn now_epoch_secs() -> i64 {
    use chrono::Utc;
    Utc::now().timestamp()
}

/*
 * WebSocket de eventos realtime (Fase 6, bloque 6.2).
 *
 * `GET /api/v1/events` hace upgrade a WebSocket y suscribe al
 * `EventBus`; desde ahi recibe solo eventos posteriores a la conexion
 * (transiciones de alertas y corridas de health checks), como JSON con
 * `type` como tag. No hay replay: el historial sigue en la REST API.
 *
 * Autenticacion: mismo bearer token que el resto de la API, pero un
 * `WebSocket` de navegador no deja setear headers, asi que se acepta el
 * token tambien por query param `?token=...`. Se valida antes del
 * upgrade (constante de tiempo, auth.rs). Sin token valido -> 401.
 */

#[derive(Debug, Deserialize)]
struct EventsQuery {
    token: Option<String>,
}

async fn events_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<EventsQuery>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse> {
    let token = &state.config.auth_token;
    let via_query = match params.token.as_deref() {
        Some(t) => check_bearer_str(t, token).is_ok(),
        None => false,
    };
    if !via_query && check_bearer(&headers, token).is_err() {
        return Err(ApiError::unauthorized());
    }

    let rx = state.events.subscribe();
    tracing::info!("cliente ws autenticado, upgrade /api/v1/events");
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, rx)))
}

/*
 * Maneja una conexion: reenvia los eventos del bus al socket y responde
 * pings (para que proxies/intermediarios no corten conexiones idle).
 * Textos del cliente se ignoran; un Close del lado cliente o un error de
 * envio terminan el bucle.
 */
async fn handle_socket(socket: WebSocket, mut rx: broadcast::Receiver<String>) {
    let (mut sink, mut stream) = socket.split();
    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Ping(ping))) => {
                        if sink.send(Message::Pong(ping)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
            event = rx.recv() => {
                match event {
                    Ok(text) => {
                        if sink.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    /* Suscriptor lento: broadcast descarto los eventos que
                     * no pudo consumir. Avisamos y seguimos desde donde
                     * viene (el dashboard puede refrescar el historial). */
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        let notice = json!({
                            "type": "events_lagged",
                            "dropped": n,
                        })
                        .to_string();
                        if sink.send(Message::Text(notice.into())).await.is_err() {
                            break;
                        }
                        tracing::warn!(dropped = n, "cliente ws atrasado: eventos descartados");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /* El dashboard (Fase 7) se sirve desde disco junto al manifest: un
     * backend sin su frontend es un arranque roto en vivo. Este test
     * pinchea el bundle frente a olvidos de commit. */
    #[test]
    fn dashboard_index_html_exists_next_to_manifest() {
        let root = env!("CARGO_MANIFEST_DIR");
        let dir = std::path::Path::new(root).join("dashboard");
        for file in [
            "index.html",
            "host.html",
            "common.js",
            "app.js",
            "host.js",
            "style.css",
        ] {
            let path = dir.join(file);
            assert!(
                path.is_file(),
                "falta {}: el collector sirve el dashboard desde ese path",
                path.display()
            );
        }
    }
}
