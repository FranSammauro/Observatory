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

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_addr: String,
    pub database_url: String,
    pub auth_token: String,
    pub db_max_connections: u32,
    pub max_future_skew_secs: i64,
    pub max_past_age_secs: i64,
    pub max_body_bytes: usize,
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

        Ok(Self {
            listen_addr,
            database_url,
            auth_token,
            db_max_connections,
            max_future_skew_secs,
            max_past_age_secs,
            max_body_bytes,
        })
    }
}
