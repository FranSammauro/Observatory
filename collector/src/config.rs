use std::env;

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

/* Evaluador de alertas (bloque 5.1): cada cuanto evaluar las reglas y
 * que ventana de `metric_samples` mirar. El intervalo es ~el periodo de
 * metricas del agent (10s); 15s da 1.5 evaluaciones por muestra. El
 * lookback de 5 min cubre holgadamente `for_secs` tipicos sin arrastrar
 * datos muertos. */

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

        validate_state_limits(state_online_secs, state_degraded_secs)?;
        validate_reboot_drop(reboot_min_uptime_drop_secs)?;
        validate_alert_limits(alert_eval_interval_secs, alert_lookback_secs)?;

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
        })
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
}
