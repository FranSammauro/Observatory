use std::env;
use std::path::PathBuf;

use crate::events::WS_CHANNEL_CAPACITY_DEFAULT;
use crate::state::StateLimits;

pub const PROTOCOL_VERSION: i32 = 1;
pub const MAX_ARRAY_ENTRIES: usize = 16;
/* El agent serializa como maximo ~14 claves escalares en `metrics`
 * (todas las categorias disponibles). El tope defensivo evita que un
 * cliente descompuesto/malicioso inyecte miles de series por sample. */
pub const MAX_METRIC_KEYS: usize = 1024;

/* Limites del query API (Fase 4, bloque 1): cuantos puntos devolver por
 * serie. Topes defensivos contra peticiones gigantes; el dashboard pide
 * rangos cortos (~centenas de puntos). */
pub const DEFAULT_SERIES_POINTS: i64 = 1_000;
pub const MAX_SERIES_POINTS: i64 = 10_000;

/* Timeline de reboots (bloque 4.3): eventos infrecuentes, alcanza con
 * menos margen que una serie. */
pub const DEFAULT_REBOOTS_LIMIT: i64 = 50;
pub const MAX_REBOOTS_LIMIT: i64 = 1_000;

/* Historial de alertas (bloque 5.3): mismo criterio que reboots, son
 * transiciones de estado (infrecuentes), con margen defensivo. */
pub const DEFAULT_ALERT_HISTORY_LIMIT: i64 = 50;
pub const MAX_ALERT_HISTORY_LIMIT: i64 = 1_000;

/* Timeline de health checks (Fase 6, bloque 6.1): una corrida por
 * intervalo, eventos de tamano acotado; mismo criterio que reboots. */
pub const DEFAULT_HEALTH_RESULTS_LIMIT: i64 = 50;
pub const MAX_HEALTH_RESULTS_LIMIT: i64 = 1_000;

/* Historial unificado (Fase 6, bloque 6.3): ultimos eventos del sistema
 * fusionando alertas + health + reboots + conectividad. Mismo criterio
 * que los timelines individuales. */
pub const DEFAULT_EVENTS_HISTORY_LIMIT: i64 = 50;
pub const MAX_EVENTS_HISTORY_LIMIT: i64 = 1_000;

/*
 * WebSocket de eventos realtime (bloque 6.2): capacidad de la cola por
 * suscriptor. Los eventos son infrecuentes (transiciones de alertas +
 * corridas de checks), 256 eventos bufferizados cubren holgadamente los
 * intervalos lanzos; un suscriptor mas lento que la cola recibe un
 * aviso `events_lagged` y sigue desde donde viene (broadcast descarta,
 * no bloquea).
 */

/* Evaluador de alertas (bloque 5.1): cada cuanto evaluar las reglas y
 * que ventana de `metric_samples` mirar. El intervalo es ~el periodo de
 * metricas del agent (10s); 15s da 1.5 evaluaciones por muestra. El
 * lookback de 5 min cubre holgadamente `for_secs` tipicos sin arrastrar
 * datos muertos.
 *
 * Hysteresis (bloque 5.2): ventana de resolucion. Una alerta FIRING no
 * se resuelve apenas la condicion deja de sostenerse; se mantiene hasta
 * OBS_ALERT_RESOLVE_GRACE_SECS sin condicion, asi un valor que oscila
 * alrededor del umbral no flapea entrada/salida. 60s = 4 ciclos del
 * evaluador (15s) o ~6 muestras de metricas (10s). */

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_addr: String,
    pub database_url: String,
    pub auth_token: String,
    pub db_max_connections: u32,
    pub max_future_skew_secs: i64,
    pub max_past_age_secs: i64,
    pub max_body_bytes: usize,
    pub state_online_secs: i64,
    pub state_degraded_secs: i64,
    pub reboot_min_uptime_drop_secs: f64,
    pub alert_eval_interval_secs: i64,
    pub alert_lookback_secs: i64,
    pub alert_resolve_grace_secs: i64,
    pub health_poll_secs: i64,
    pub health_default_timeout_secs: i64,
    pub ws_channel_capacity: usize,
    pub connectivity_poll_secs: i64,
    pub dashboard_dir: String,
    pub rate_limit_enabled: bool,
    pub rate_limit_rate: f64,
    pub rate_limit_burst: f64,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
}

fn parse_u32(name: &str, default: u32) -> Result<u32, String> {
    match env::var(name) {
        Ok(v) => v
            .trim()
            .parse()
            .map_err(|_| format!("{name} no es un entero valido: '{v}'")),
        Err(_) => Ok(default),
    }
}

fn parse_i64(name: &str, default: i64) -> Result<i64, String> {
    match env::var(name) {
        Ok(v) => v
            .trim()
            .parse()
            .map_err(|_| format!("{name} no es un entero valido: '{v}'")),
        Err(_) => Ok(default),
    }
}

fn parse_f64(name: &str, default: f64) -> Result<f64, String> {
    match env::var(name) {
        Ok(v) => v
            .trim()
            .parse()
            .map_err(|_| format!("{name} no es un numero valido: '{v}'")),
        Err(_) => Ok(default),
    }
}

fn parse_bool(name: &str, default: bool) -> Result<bool, String> {
    match env::var(name) {
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => Err(format!("{name} debe ser true o false: '{v}'")),
        },
        Err(_) => Ok(default),
    }
}

/* Variables opcionales: vacias o ausentes valen None. */
fn opt_env_path(name: &str) -> Option<PathBuf> {
    env::var(name)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL")
            .map_err(|_| "env DATABASE_URL requerida (ej: postgres://observer:observer@127.0.0.1:5432/observer)".to_string())?;

        let auth_token = env::var("OBS_COLLECTOR_TOKEN").map_err(|_| {
            "env OBS_COLLECTOR_TOKEN requerida (token bearer compartido que los agents envian)"
                .to_string()
        })?;

        let listen_addr =
            env::var("OBS_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
        let db_max_connections = parse_u32("OBS_DB_MAX_CONNECTIONS", 5)?;
        let max_future_skew_secs = parse_i64("OBS_INGEST_FUTURE_SKEW_SECS", 60)?;
        let max_past_age_secs = parse_i64("OBS_INGEST_MAX_AGE_SECS", 600)?;
        let max_body_bytes = parse_u32("OBS_MAX_BODY_BYTES", 256 * 1024)? as usize;
        let state_online_secs = parse_i64("OBS_STATE_ONLINE_SECS", 15)?;
        let state_degraded_secs = parse_i64("OBS_STATE_DEGRADED_SECS", 60)?;
        let reboot_min_uptime_drop_secs = parse_f64("OBS_REBOOT_MIN_UPTIME_DROP_SECS", 2.0)?;
        let alert_eval_interval_secs = parse_i64("OBS_ALERT_EVAL_INTERVAL_SECS", 15)?;
        let alert_lookback_secs = parse_i64("OBS_ALERT_LOOKBACK_SECS", 300)?;
        let alert_resolve_grace_secs = parse_i64("OBS_ALERT_RESOLVE_GRACE_SECS", 60)?;
        let health_poll_secs = parse_i64("OBS_HEALTH_POLL_SECS", 1)?;
        let health_default_timeout_secs = parse_i64("OBS_HEALTH_DEFAULT_TIMEOUT_SECS", 5)?;
        let ws_channel_capacity = parse_u32(
            "OBS_WS_CHANNEL_CAPACITY",
            WS_CHANNEL_CAPACITY_DEFAULT as u32,
        )? as usize;
        let connectivity_poll_secs = parse_i64("OBS_CONNECTIVITY_POLL_SECS", 5)?;
        let dashboard_raw =
            env::var("OBS_DASHBOARD_DIR").unwrap_or_else(|_| "dashboard".to_string());
        let dashboard_dir = sanitize_dashboard_dir(&dashboard_raw)?;
        let rate_limit_enabled = parse_bool("OBS_RATE_LIMIT_ENABLED", true)?;
        let rate_limit_rate = parse_f64("OBS_RATE_LIMIT_RATE", 20.0)?;
        let rate_limit_burst = parse_f64("OBS_RATE_LIMIT_BURST", 50.0)?;
        let tls_cert = opt_env_path("OBS_TLS_CERT");
        let tls_key = opt_env_path("OBS_TLS_KEY");

        validate_state_limits(state_online_secs, state_degraded_secs)?;
        validate_reboot_drop(reboot_min_uptime_drop_secs)?;
        validate_alert_limits(alert_eval_interval_secs, alert_lookback_secs)?;
        validate_alert_grace(alert_resolve_grace_secs)?;
        validate_health_poll(health_poll_secs)?;
        validate_health_timeout(health_default_timeout_secs)?;
        validate_ws_capacity(ws_channel_capacity)?;
        validate_connectivity_poll(connectivity_poll_secs)?;
        validate_rate_limit(rate_limit_enabled, rate_limit_rate, rate_limit_burst)?;
        validate_tls_pair(tls_cert.as_ref(), tls_key.as_ref())?;

        Ok(Self {
            listen_addr,
            database_url,
            auth_token,
            db_max_connections,
            max_future_skew_secs,
            max_past_age_secs,
            max_body_bytes,
            state_online_secs,
            state_degraded_secs,
            reboot_min_uptime_drop_secs,
            alert_eval_interval_secs,
            alert_lookback_secs,
            alert_resolve_grace_secs,
            health_poll_secs,
            health_default_timeout_secs,
            ws_channel_capacity,
            connectivity_poll_secs,
            dashboard_dir,
            rate_limit_enabled,
            rate_limit_rate,
            rate_limit_burst,
            tls_cert,
            tls_key,
        })
    }

    /* Umbrales de la maquina de conectividad (bloque 4.2): compartidos
     * por los handlers de query y el runner de conectividad (6.3). */
    pub fn state_limits(&self) -> StateLimits {
        StateLimits {
            online_secs: self.state_online_secs,
            degraded_secs: self.state_degraded_secs,
        }
    }
}

/*
 * Caida minima de uptime para considerarla un reboot (bloque 4.3): no
 * puede ser negativa (una caida es una caida).
 */
pub fn validate_reboot_drop(min_drop_secs: f64) -> Result<(), String> {
    if min_drop_secs.is_finite() && min_drop_secs >= 0.0 {
        Ok(())
    } else {
        Err("OBS_REBOOT_MIN_UPTIME_DROP_SECS debe ser un numero no negativo".to_string())
    }
}

/*
 * Umbrales de la maquina de estados (bloque 4.2): online_secs <=
 * degraded_secs y ninguno negativo. Falla ruidoso al arrancar, igual que
 * el resto de la validacion de config (filosofia ADR-0002/0003).
 */
pub fn validate_state_limits(online_secs: i64, degraded_secs: i64) -> Result<(), String> {
    if online_secs < 0 {
        return Err("OBS_STATE_ONLINE_SECS no puede ser negativo".to_string());
    }
    if degraded_secs < online_secs {
        return Err(
            "OBS_STATE_DEGRADED_SECS debe ser mayor o igual que OBS_STATE_ONLINE_SECS".to_string(),
        );
    }
    Ok(())
}

/*
 * Evaluador de alertas (bloque 5.1): intervalo y ventana deben ser
 * positivos (un ciclo cada 0s o una ventana vacia no tienen sentido).
 */
pub fn validate_alert_limits(interval_secs: i64, lookback_secs: i64) -> Result<(), String> {
    if interval_secs <= 0 {
        return Err("OBS_ALERT_EVAL_INTERVAL_SECS debe ser mayor que 0".to_string());
    }
    if lookback_secs <= 0 {
        return Err("OBS_ALERT_LOOKBACK_SECS debe ser mayor que 0".to_string());
    }
    Ok(())
}

/*
 * Hysteresis del bloque 5.2: la ventana de resolucion no puede ser
 * negativa (0 = resolver apenas la condicion deja de sostenerse).
 */
pub fn validate_alert_grace(grace_secs: i64) -> Result<(), String> {
    if grace_secs < 0 {
        return Err("OBS_ALERT_RESOLVE_GRACE_SECS no puede ser negativo".to_string());
    }
    Ok(())
}

/*
 * Health checks (Fase 6, bloque 6.1): el scheduler no puede tener paso 0
 * y el timeout default de un check tiene que ser positivo y acotado.
 */
pub fn validate_health_poll(poll_secs: i64) -> Result<(), String> {
    if poll_secs < 1 {
        return Err("OBS_HEALTH_POLL_SECS debe ser mayor o igual a 1".to_string());
    }
    Ok(())
}

pub fn validate_health_timeout(timeout_secs: i64) -> Result<(), String> {
    if !(1..=300).contains(&timeout_secs) {
        return Err(
            "OBS_HEALTH_DEFAULT_TIMEOUT_SECS debe estar entre 1 y 300 segundos".to_string(),
        );
    }
    Ok(())
}

/*
 * Canal de eventos del WebSocket (bloque 6.2): una cola vacia (0) no
 * tiene sentido; un broadcast de tokio requiere capacidad >= 1.
 */
pub fn validate_ws_capacity(capacity: usize) -> Result<(), String> {
    if capacity >= 1 {
        Ok(())
    } else {
        Err("OBS_WS_CHANNEL_CAPACITY debe ser mayor que 0".to_string())
    }
}

/*
 * Runner de conectividad (bloque 6.3): detecta transiciones del estado
 * derivado ONLINE/DEGRADED/OFFLINE. El intervalo no puede ser 0 ni
 * negativo; 5s default cubre bien la ventana de los umbrales (15s/60s).
 */
pub fn validate_connectivity_poll(poll_secs: i64) -> Result<(), String> {
    if poll_secs < 1 {
        return Err("OBS_CONNECTIVITY_POLL_SECS debe ser mayor o igual a 1".to_string());
    }
    Ok(())
}

/*
 * Rate limiting (Fase 8, bloque 8.1): si esta habilitado, rate y burst
 * deben ser positivos. Deshabilitado, ambos se ignoran (se pueden dejar
 * en 0). Falla ruidoso ante configuracion contradictoria.
 */
pub fn validate_rate_limit(enabled: bool, rate: f64, burst: f64) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }
    if !rate.is_finite() || rate <= 0.0 {
        return Err("OBS_RATE_LIMIT_RATE debe ser un numero positivo".to_string());
    }
    if !burst.is_finite() || burst <= 0.0 {
        return Err("OBS_RATE_LIMIT_BURST debe ser un numero positivo".to_string());
    }
    Ok(())
}

/*
 * TLS (Fase 8, bloque 8.2): el certificado y la clave privada van
 * siempre juntos. Falta ruidoso al arrancar si solo se seteo uno — un
 * TLS medio configurado (cert sin key, o viceversa) es peor que advertir
 * en claro, misma filosofia de errores que el resto de la config.
 */
pub fn validate_tls_pair(cert: Option<&PathBuf>, key: Option<&PathBuf>) -> Result<(), String> {
    match (cert, key) {
        (Some(_), Some(_)) => Ok(()),
        (None, None) => Ok(()),
        (Some(_), None) => {
            Err("OBS_TLS_CERT esta set pero OBS_TLS_KEY falta: ambos deben ir juntos".to_string())
        }
        (None, Some(_)) => {
            Err("OBS_TLS_KEY esta set pero OBS_TLS_CERT falta: ambos deben ir juntos".to_string())
        }
    }
}

/*
 * Directorio del dashboard (Fase 7, bloque 7.1): se sirve tal cual, con
 * un solo saneamiento — quitar el `/` final para que ServeDir no falle
 * con un path tipo `dashboard/` duplicado al resolver index.html. No
 * puede quedar vacio.
 */
pub fn sanitize_dashboard_dir(raw: &str) -> Result<String, String> {
    let dir = raw.trim().trim_end_matches('/').to_string();
    if dir.is_empty() {
        return Err("OBS_DASHBOARD_DIR no puede quedar vacio".to_string());
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_limits_accept_sane_values() {
        assert!(validate_state_limits(15, 60).is_ok());
    }

    #[test]
    fn state_limits_reject_negative_online() {
        assert!(validate_state_limits(-1, 60).is_err());
    }

    #[test]
    fn state_limits_reject_degraded_below_online() {
        assert!(validate_state_limits(60, 59).is_err());
    }

    #[test]
    fn state_limits_equal_is_valid() {
        assert!(validate_state_limits(15, 15).is_ok());
    }

    #[test]
    fn reboot_drop_accepts_zero_and_positive() {
        assert!(validate_reboot_drop(0.0).is_ok());
        assert!(validate_reboot_drop(2.5).is_ok());
    }

    #[test]
    fn reboot_drop_rejects_negative_and_non_finite() {
        assert!(validate_reboot_drop(-1.0).is_err());
        assert!(validate_reboot_drop(f64::NAN).is_err());
        assert!(validate_reboot_drop(f64::INFINITY).is_err());
    }

    #[test]
    fn alert_limits_accept_positive_values() {
        assert!(validate_alert_limits(15, 300).is_ok());
    }

    #[test]
    fn alert_limits_reject_zero_and_negative() {
        assert!(validate_alert_limits(0, 300).is_err());
        assert!(validate_alert_limits(15, 0).is_err());
        assert!(validate_alert_limits(-5, 300).is_err());
        assert!(validate_alert_limits(15, -5).is_err());
    }

    #[test]
    fn alert_grace_accepts_zero_and_positive() {
        assert!(validate_alert_grace(0).is_ok());
        assert!(validate_alert_grace(60).is_ok());
    }

    #[test]
    fn alert_grace_rejects_negative() {
        assert!(validate_alert_grace(-1).is_err());
    }

    #[test]
    fn health_poll_accepts_positive() {
        assert!(validate_health_poll(1).is_ok());
        assert!(validate_health_poll(5).is_ok());
    }

    #[test]
    fn health_poll_rejects_zero_and_negative() {
        assert!(validate_health_poll(0).is_err());
        assert!(validate_health_poll(-1).is_err());
    }

    #[test]
    fn health_timeout_accepts_bounds() {
        assert!(validate_health_timeout(1).is_ok());
        assert!(validate_health_timeout(5).is_ok());
        assert!(validate_health_timeout(300).is_ok());
    }

    #[test]
    fn health_timeout_rejects_out_of_range() {
        assert!(validate_health_timeout(0).is_err());
        assert!(validate_health_timeout(301).is_err());
        assert!(validate_health_timeout(-5).is_err());
    }

    #[test]
    fn ws_capacity_accepts_positive() {
        assert!(validate_ws_capacity(1).is_ok());
        assert!(validate_ws_capacity(256).is_ok());
    }

    #[test]
    fn ws_capacity_rejects_zero() {
        assert!(validate_ws_capacity(0).is_err());
    }

    #[test]
    fn connectivity_poll_accepts_positive() {
        assert!(validate_connectivity_poll(1).is_ok());
        assert!(validate_connectivity_poll(5).is_ok());
    }

    #[test]
    fn connectivity_poll_rejects_zero_and_negative() {
        assert!(validate_connectivity_poll(0).is_err());
        assert!(validate_connectivity_poll(-1).is_err());
    }

    #[test]
    fn dashboard_dir_default_strips_trailing_slash() {
        assert_eq!(sanitize_dashboard_dir("dashboard").unwrap(), "dashboard");
        assert_eq!(sanitize_dashboard_dir("dashboard/").unwrap(), "dashboard");
        assert_eq!(sanitize_dashboard_dir(" /web/ ").unwrap(), "/web");
    }

    #[test]
    fn dashboard_dir_rejects_empty() {
        assert!(sanitize_dashboard_dir("").is_err());
        assert!(sanitize_dashboard_dir("/").is_err());
        assert!(sanitize_dashboard_dir("   ").is_err());
    }

    #[test]
    fn rate_limit_accepts_positive_when_enabled() {
        assert!(validate_rate_limit(true, 20.0, 50.0).is_ok());
        assert!(validate_rate_limit(true, 0.5, 5.0).is_ok());
    }

    #[test]
    fn rate_limit_rejects_zero_negative_or_nan_when_enabled() {
        assert!(validate_rate_limit(true, 0.0, 50.0).is_err());
        assert!(validate_rate_limit(true, -1.0, 50.0).is_err());
        assert!(validate_rate_limit(true, 20.0, 0.0).is_err());
        assert!(validate_rate_limit(true, f64::NAN, 50.0).is_err());
        assert!(validate_rate_limit(true, f64::INFINITY, 50.0).is_err());
    }

    #[test]
    fn rate_limit_disabled_ignores_values() {
        assert!(validate_rate_limit(false, 0.0, 0.0).is_ok());
        assert!(validate_rate_limit(false, -1.0, -5.0).is_ok());
    }

    fn path(p: &str) -> Option<PathBuf> {
        Some(PathBuf::from(p))
    }

    #[test]
    fn tls_pair_accepts_both_or_none() {
        assert!(validate_tls_pair(None, None).is_ok());
        assert!(validate_tls_pair(path("/c.pem").as_ref(), path("/k.pem").as_ref()).is_ok());
    }

    #[test]
    fn tls_pair_rejects_cert_without_key() {
        assert!(validate_tls_pair(path("/c.pem").as_ref(), None).is_err());
    }

    #[test]
    fn tls_pair_rejects_key_without_cert() {
        assert!(validate_tls_pair(None, path("/k.pem").as_ref()).is_err());
    }
}
