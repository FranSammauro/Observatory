use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::config::{
    DEFAULT_ALERT_HISTORY_LIMIT, DEFAULT_EVENTS_HISTORY_LIMIT, DEFAULT_HEALTH_RESULTS_LIMIT,
    DEFAULT_REBOOTS_LIMIT, DEFAULT_SERIES_POINTS, MAX_ALERT_HISTORY_LIMIT,
    MAX_EVENTS_HISTORY_LIMIT, MAX_HEALTH_RESULTS_LIMIT, MAX_REBOOTS_LIMIT, MAX_SERIES_POINTS,
};
use crate::error::ApiError;

/*
 * Parsing y validacion de los parametros
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

/*
 * Parametros de consulta para alertas:
 *   - `GET /api/v1/alerts`            -> alertas activas (pending/firing).
 *   - `GET /api/v1/alerts/history`    -> historial de transiciones.
 * Mismo bearer token que el resto; las reglas de validacion estan aca,
 * puras y testeables sin DB.
 */

fn parse_agent_uuid(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw.trim())
        .map_err(|_| ApiError::bad_request("invalid_agent_id", "agent_id no es un UUID valido"))
}

#[derive(Debug, Deserialize)]
pub struct AlertsQuery {
    pub agent_id: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AlertsFilter {
    pub agent_id: Option<Uuid>,
    pub state: Option<String>,
}

impl AlertsQuery {
    pub fn into_filter(self) -> Result<AlertsFilter, ApiError> {
        let agent_id = self.agent_id.map(|s| parse_agent_uuid(&s)).transpose()?;

        let state = match self.state {
            Some(s) if s.trim().is_empty() || !matches!(s.trim(), "pending" | "firing") => {
                return Err(ApiError::bad_request(
                    "invalid_alert_state",
                    "state debe ser pending o firing",
                ));
            }
            Some(s) => Some(s.trim().to_string()),
            None => None,
        };

        Ok(AlertsFilter { agent_id, state })
    }
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub agent_id: Option<String>,
    pub rule_id: Option<i64>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct HistoryFilter {
    pub agent_id: Option<Uuid>,
    pub rule_id: Option<i64>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: i64,
}

impl HistoryQuery {
    pub fn into_filter(self) -> Result<HistoryFilter, ApiError> {
        let agent_id = self.agent_id.map(|s| parse_agent_uuid(&s)).transpose()?;

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

        if let Some(rule_id) = self.rule_id {
            if rule_id < 1 {
                return Err(ApiError::bad_request(
                    "invalid_rule_id",
                    "rule_id debe ser un entero positivo",
                ));
            }
        }

        let limit = parse_limit(
            self.limit,
            DEFAULT_ALERT_HISTORY_LIMIT,
            MAX_ALERT_HISTORY_LIMIT,
        )?;

        Ok(HistoryFilter {
            agent_id,
            rule_id: self.rule_id,
            from,
            to,
            limit,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct CheckResultsQuery {
    pub limit: Option<i64>,
}

impl CheckResultsQuery {
    pub fn into_limit(self) -> Result<i64, ApiError> {
        parse_limit(
            self.limit,
            DEFAULT_HEALTH_RESULTS_LIMIT,
            MAX_HEALTH_RESULTS_LIMIT,
        )
    }
}

/*
 * Parametros del timeline unificado de eventos:
 * `GET /api/v1/events/history`: recorta por agent (opcional) y devuelve
 * los ultimos `limit` eventos cruzando las cuatro fuentes (alertas,
 * health, reboots, conectividad) ordenados por timestamp desc. El merge
 * vive en Rust (merge_timeline) para no pagar un UNION ALL heterogeneo
 * en SQL y poder testearlo puro.
 *
 * `limit` reusa el rango de histories previos: default 50, maximo 1000.
 * Nota: el agent_id se aplica en los handlers de cada fuente; cada una
 * trae hasta `limit` filas y el merge corta al total pedido.
 */

#[derive(Debug, Deserialize)]
pub struct TimelineQuery {
    pub agent_id: Option<String>,
    pub limit: Option<i64>,
}

impl TimelineQuery {
    pub fn into_filter(self) -> Result<TimelineFilter, ApiError> {
        let agent_id = self.agent_id.map(|s| parse_agent_uuid(&s)).transpose()?;
        let limit = parse_limit(
            self.limit,
            DEFAULT_EVENTS_HISTORY_LIMIT,
            MAX_EVENTS_HISTORY_LIMIT,
        )?;
        Ok(TimelineFilter { agent_id, limit })
    }
}

#[derive(Debug, Clone)]
pub struct TimelineFilter {
    pub agent_id: Option<Uuid>,
    pub limit: i64,
}

/*
 * Un elemento del timeline unificado: `kind` dice la fuente (alert_event,
 * health_result, reboot_event, connectivity_event) y `ts` es la marca que
 * ordena. El resto de los campos va en el JSON plano de cada evento
 * (mismo shape que en el WebSocket donde corresponde).
 */
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineEntry {
    pub kind: &'static str,
    pub ts: DateTime<Utc>,
    pub payload: serde_json::Value,
}

/*
 * Merge de las cuatro fuentes (pura): concatena, ordena por `ts`
 * descendentemente y corta a `limit`. Orden estable entre filas con el
 * mismo ts: la fuente queda como llego (cada una ya viene ordenada).
 */
pub fn merge_timeline(entries: Vec<TimelineEntry>, limit: usize) -> Vec<TimelineEntry> {
    let mut merged = entries;
    merged.sort_by_key(|e| std::cmp::Reverse(e.ts));
    merged.truncate(limit);
    merged
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

    #[test]
    fn alerts_empty_query_uses_defaults() {
        let f = AlertsQuery {
            agent_id: None,
            state: None,
        }
        .into_filter()
        .unwrap();
        assert_eq!(f.agent_id, None);
        assert_eq!(f.state, None);
    }

    #[test]
    fn alerts_state_is_validated_and_trimmed() {
        let f = AlertsQuery {
            agent_id: None,
            state: Some("  firing  ".into()),
        }
        .into_filter()
        .unwrap();
        assert_eq!(f.state.as_deref(), Some("firing"));
    }

    #[test]
    fn alerts_state_is_rejected() {
        let r = AlertsQuery {
            agent_id: None,
            state: Some("resolved".into()),
        }
        .into_filter();
        assert!(r.is_err());
        assert_eq!(r.unwrap_err().code, "invalid_alert_state");
    }

    #[test]
    fn alerts_empty_state_is_rejected() {
        let r = AlertsQuery {
            agent_id: None,
            state: Some("   ".into()),
        }
        .into_filter();
        assert!(r.is_err());
    }

    #[test]
    fn alerts_invalid_agent_id_is_rejected() {
        let r = AlertsQuery {
            agent_id: Some("not-a-uuid".into()),
            state: None,
        }
        .into_filter();
        assert_eq!(r.unwrap_err().code, "invalid_agent_id");
    }

    #[test]
    fn alerts_valid_agent_id_is_parsed() {
        let f = AlertsQuery {
            agent_id: Some("bca99f71-8eaa-f6f1-55b2-14a92fdd309f".into()),
            state: None,
        }
        .into_filter()
        .unwrap();
        assert!(f.agent_id.is_some());
    }

    #[test]
    fn history_defaults() {
        let f = HistoryQuery {
            agent_id: None,
            rule_id: None,
            from: None,
            to: None,
            limit: None,
        }
        .into_filter()
        .unwrap();
        assert_eq!(f.limit, DEFAULT_ALERT_HISTORY_LIMIT);
        assert!(f.from.is_none() && f.to.is_none() && f.agent_id.is_none());
    }

    #[test]
    fn history_from_after_to_is_rejected() {
        let r = HistoryQuery {
            agent_id: None,
            rule_id: None,
            from: Some(200),
            to: Some(100),
            limit: None,
        }
        .into_filter();
        assert!(r.is_err());
    }

    #[test]
    fn history_zero_rule_id_is_rejected() {
        let r = HistoryQuery {
            agent_id: None,
            rule_id: Some(0),
            from: None,
            to: None,
            limit: None,
        }
        .into_filter();
        assert_eq!(r.unwrap_err().code, "invalid_rule_id");
    }

    #[test]
    fn history_limit_clamped_to_default() {
        let f = HistoryQuery {
            agent_id: None,
            rule_id: None,
            from: None,
            to: None,
            limit: Some(MAX_ALERT_HISTORY_LIMIT),
        }
        .into_filter()
        .unwrap();
        assert_eq!(f.limit, MAX_ALERT_HISTORY_LIMIT);
    }

    #[test]
    fn check_results_default_limit() {
        let l = CheckResultsQuery { limit: None }.into_limit().unwrap();
        assert_eq!(l, DEFAULT_HEALTH_RESULTS_LIMIT);
    }

    #[test]
    fn check_results_limit_above_max_is_rejected() {
        let r = CheckResultsQuery {
            limit: Some(MAX_HEALTH_RESULTS_LIMIT + 1),
        }
        .into_limit();
        assert!(r.is_err());
    }

    #[test]
    fn timeline_defaults() {
        let f = TimelineQuery {
            agent_id: None,
            limit: None,
        }
        .into_filter()
        .unwrap();
        assert_eq!(f.agent_id, None);
        assert_eq!(f.limit, DEFAULT_EVENTS_HISTORY_LIMIT);
    }

    #[test]
    fn timeline_limit_is_validated() {
        let r = TimelineQuery {
            agent_id: None,
            limit: Some(0),
        }
        .into_filter();
        assert!(r.is_err());

        let f = TimelineQuery {
            agent_id: None,
            limit: Some(MAX_EVENTS_HISTORY_LIMIT),
        }
        .into_filter()
        .unwrap();
        assert_eq!(f.limit, MAX_EVENTS_HISTORY_LIMIT);
    }

    #[test]
    fn timeline_invalid_agent_is_rejected() {
        let r = TimelineQuery {
            agent_id: Some("nope".into()),
            limit: None,
        }
        .into_filter();
        assert_eq!(r.unwrap_err().code, "invalid_agent_id");
    }

    fn entry(kind: &'static str, ts: &str) -> TimelineEntry {
        TimelineEntry {
            kind,
            ts: DateTime::parse_from_rfc3339(ts)
                .unwrap()
                .with_timezone(&Utc),
            payload: serde_json::json!({"ts": ts}),
        }
    }

    #[test]
    fn merge_timeline_sorts_desc_and_truncates() {
        let entries = vec![
            entry("alert_event", "2024-07-03T09:46:40Z"),
            entry("health_result", "2024-07-03T09:47:00Z"),
            entry("reboot_event", "2024-07-03T09:46:00Z"),
            entry("connectivity_event", "2024-07-03T09:47:30Z"),
        ];
        let out = merge_timeline(entries, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, "connectivity_event");
        assert_eq!(out[1].kind, "health_result");
    }

    #[test]
    fn merge_timeline_keeps_everything_under_limit() {
        let entries = vec![entry("alert_event", "2024-07-03T09:46:40Z")];
        let out = merge_timeline(entries, 50);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn merge_timeline_empty_input() {
        assert!(merge_timeline(vec![], 10).is_empty());
    }
}
