use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::config::{
    DEFAULT_REBOOTS_LIMIT, DEFAULT_SERIES_POINTS, MAX_REBOOTS_LIMIT, MAX_SERIES_POINTS,
};
use crate::error::ApiError;

/*
 * Query API (Fase 4, bloque 1): parsing y validacion de los parametros
 * de `GET /api/v1/agents/:id/metrics/:metric`. Los handlers solo llaman
 * `into_filter()`; las reglas estan aca, puras y testeables sin DB.
 *
 * Parametros (todos opcionales):
 *   - `entity`: label de la entidad (device/interface/mountpoint). Para
 *     escalares se omite (entity NULL en la DB). No puede estar vacio.
 *   - `from`/`to`: limites temporales en segundos epoch, inclusive. Si
 *     ambos estan, `from` debe ser <= `to`.
 *   - `limit`: maximo de puntos a devolver (default 1000, top 10000).
 */

#[derive(Debug, Deserialize)]
pub struct SeriesQuery {
    pub entity: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SeriesFilter {
    pub entity: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: i64,
}

fn unix_to_utc(ts: i64, which: &str) -> Result<DateTime<Utc>, ApiError> {
    Utc.timestamp_opt(ts, 0).single().ok_or_else(|| {
        ApiError::bad_request(
            "invalid_time_range",
            format!("{which} no es un timestamp valido: {ts}"),
        )
    })
}

/*
 * Limite comun a series y timeline de reboots: si viene, debe estar en
 * [1, max]; si no, default.
 */
pub fn parse_limit(raw: Option<i64>, default: i64, max: i64) -> Result<i64, ApiError> {
    let limit = raw.unwrap_or(default);
    if !(1..=max).contains(&limit) {
        return Err(ApiError::bad_request(
            "invalid_limit",
            format!("limit debe estar entre 1 y {max}"),
        ));
    }
    Ok(limit)
}

impl SeriesQuery {
    pub fn into_filter(self) -> Result<SeriesFilter, ApiError> {
        let entity = match self.entity {
            Some(e) if e.trim().is_empty() => {
                return Err(ApiError::bad_request(
                    "invalid_entity",
                    "entity no puede estar vacio",
                ));
            }
            Some(e) => Some(e.trim().to_string()),
            None => None,
        };

        let from = self.from.map(|ts| unix_to_utc(ts, "from")).transpose()?;
        let to = self.to.map(|ts| unix_to_utc(ts, "to")).transpose()?;

        if let (Some(start), Some(end)) = (from, to) {
            if start > end {
                return Err(ApiError::bad_request(
                    "invalid_time_range",
                    "from no puede ser posterior a to",
                ));
            }
        }

        let limit = parse_limit(self.limit, DEFAULT_SERIES_POINTS, MAX_SERIES_POINTS)?;

        Ok(SeriesFilter {
            entity,
            from,
            to,
            limit,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct RebootsQuery {
    pub limit: Option<i64>,
}

impl RebootsQuery {
    pub fn into_limit(self) -> Result<i64, ApiError> {
        parse_limit(self.limit, DEFAULT_REBOOTS_LIMIT, MAX_REBOOTS_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_uses_defaults() {
        let f = SeriesQuery {
            entity: None,
            from: None,
            to: None,
            limit: None,
        }
        .into_filter()
        .unwrap();
        assert_eq!(f.entity, None);
        assert_eq!(f.from, None);
        assert_eq!(f.to, None);
        assert_eq!(f.limit, DEFAULT_SERIES_POINTS);
    }

    #[test]
    fn entity_is_trimmed() {
        let f = SeriesQuery {
            entity: Some("  sda  ".into()),
            from: None,
            to: None,
            limit: None,
        }
        .into_filter()
        .unwrap();
        assert_eq!(f.entity.as_deref(), Some("sda"));
    }

    #[test]
    fn empty_entity_is_rejected() {
        let r = SeriesQuery {
            entity: Some("   ".into()),
            from: None,
            to: None,
            limit: None,
        }
        .into_filter();
        assert!(r.is_err());
    }

    #[test]
    fn from_after_to_is_rejected() {
        let r = SeriesQuery {
            entity: None,
            from: Some(200),
            to: Some(100),
            limit: None,
        }
        .into_filter();
        assert!(r.is_err());
    }

    #[test]
    fn from_equal_to_is_inclusive() {
        let f = SeriesQuery {
            entity: None,
            from: Some(100),
            to: Some(100),
            limit: None,
        }
        .into_filter()
        .unwrap();
        assert!(f.from.is_some() && f.to.is_some());
    }

    #[test]
    fn limit_lower_than_one_is_rejected() {
        let r = SeriesQuery {
            entity: None,
            from: None,
            to: None,
            limit: Some(0),
        }
        .into_filter();
        assert!(r.is_err());
    }

    #[test]
    fn limit_above_max_is_rejected() {
        let r = SeriesQuery {
            entity: None,
            from: None,
            to: None,
            limit: Some(MAX_SERIES_POINTS + 1),
        }
        .into_filter();
        assert!(r.is_err());
    }

    #[test]
    fn limit_boundaries_are_inclusive() {
        let f = SeriesQuery {
            entity: None,
            from: None,
            to: None,
            limit: Some(MAX_SERIES_POINTS),
        }
        .into_filter()
        .unwrap();
        assert_eq!(f.limit, MAX_SERIES_POINTS);
    }

    #[test]
    fn reboots_default_limit() {
        let l = RebootsQuery { limit: None }.into_limit().unwrap();
        assert_eq!(l, DEFAULT_REBOOTS_LIMIT);
    }

    #[test]
    fn reboots_limit_above_max_is_rejected() {
        let r = RebootsQuery {
            limit: Some(MAX_REBOOTS_LIMIT + 1),
        }
        .into_limit();
        assert!(r.is_err());
    }

    #[test]
    fn parse_limit_rejects_zero() {
        assert!(parse_limit(Some(0), DEFAULT_REBOOTS_LIMIT, MAX_REBOOTS_LIMIT).is_err());
    }
}
