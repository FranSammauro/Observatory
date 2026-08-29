use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::config::Config;
use crate::db;
use crate::error::ApiError;
use crate::events::EventBus;

/*
 * Health checks proactivos (Fase 6, bloque 6.1): el collector ejecuta
 * probes HTTP/TCP periodicos contra targets externos, registra cada
 * corrida y mantiene el estado up/down actual de cada check.
 *
 * Un check es: nombre unico, kind (http|tcp), target, intervalo propio,
 * timeout y enabled. El scheduler (`spawn_health_runner`) revisa cada
 * OBS_HEALTH_POLL_SECS que checks habilitados estan vencidos (>= 1
 * corrida por intervalo, arranque inmediato) y los ejecuta con timeout.
 *
 * HTTP: pedido GET minimal sobre TcpStream (mismo espiritu que
 * transport.c del agent: sin libcurl ni crates de HTTP), ok si el codigo
 * de estado es 2xx/3xx. TCP: conexion exitosa => ok. `https://` se
 * rechaza: TLS se difiere a la Fase 8 (ver ADR-0002).
 */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    Http,
    Tcp,
}

impl CheckKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckKind::Http => "http",
            CheckKind::Tcp => "tcp",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "http" => Some(CheckKind::Http),
            "tcp" => Some(CheckKind::Tcp),
            _ => None,
        }
    }
}

/*
 * Target parseado y ya validado. Para http el path queda para el pedido
 * (default "/"); para tcp no se usa.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTarget {
    pub host: String,
    pub port: u16,
    pub path: String,
}

/*
 * Parseo manual de target, sin crate de URLs. Reglas:
 *   http://host[:port][/path]
 *   host:port
 * - `https://` se rechaza explicitamente (TLS en Fase 8).
 * - host no vacio, puerto u16 valido.
 */
pub fn parse_target(kind: CheckKind, target: &str) -> Result<ParsedTarget, String> {
    let t = target.trim();
    if t.is_empty() {
        return Err("target vacio".to_string());
    }
    match kind {
        CheckKind::Tcp => {
            let (host, port_raw) = t
                .rsplit_once(':')
                .ok_or_else(|| "target tcp debe ser host:puerto".to_string())?;
            let host = host.trim();
            if host.is_empty() {
                return Err("host vacio".to_string());
            }
            let port: u16 = port_raw
                .trim()
                .parse()
                .map_err(|_| format!("puerto invalido: {port_raw}"))?;
            Ok(ParsedTarget {
                host: host.to_string(),
                port,
                path: String::new(),
            })
        }
        CheckKind::Http => {
            let rest = t
                .strip_prefix("http://")
                .ok_or_else(|| {
                    "target http debe empezar con http:// (https llega en la Fase 8: TLS)"
                        .to_string()
                })?
                .trim_end();
            if rest.is_empty() {
                return Err("target http vacio".to_string());
            }
            let (authority, path) = match rest.find('/') {
                Some(i) => (&rest[..i], rest[i..].to_string()),
                None => (rest, "/".to_string()),
            };
            let (host, port) = match authority.find(':') {
                Some(i) => {
                    let port_raw = &authority[i + 1..];
                    let port: u16 = port_raw
                        .trim()
                        .parse()
                        .map_err(|_| format!("puerto invalido: {port_raw}"))?;
                    (&authority[..i], port)
                }
                None => (authority, 80),
            };
            let host = host.trim();
            if host.is_empty() {
                return Err("host vacio".to_string());
            }
            Ok(ParsedTarget {
                host: host.to_string(),
                port,
                path,
            })
        }
    }
}

/*
 * Pedido HTTP/1.1 minimal para el probe: solo nos interesa el codigo de
 * estado de la primera linea. `Connection: close` para que el servidor
 * cierre y la lectura termine.
 */
pub fn build_http_request(target: &ParsedTarget) -> String {
    let authority = if target.port == 80 {
        target.host.clone()
    } else {
        format!("{}:{}", target.host, target.port)
    };
    format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        target.path, authority
    )
}

/*
 * Codigo de estado de la primera linea: "HTTP/1.1 200 OK" -> 200.
 */
pub fn parse_http_status(text: &str) -> Option<u16> {
    let first = text.lines().next()?;
    let mut parts = first.split_whitespace();
    let _version = parts.next()?;
    parts.next()?.parse().ok()
}

/*
 * Resultado de una corrida.
 */
#[derive(Debug, Clone)]
pub struct CheckOutcome {
    pub ok: bool,
    pub latency_ms: i64,
    pub detail: String,
}

/*
 * Ejecuta un probe con timeout global (connect + write + read acotados).
 * TCP: conexion exitosa => up. HTTP: status 2xx/3xx => up.
 */
async fn probe(kind: CheckKind, target: &ParsedTarget, timeout: Duration) -> CheckOutcome {
    let start = Instant::now();
    let elapsed = || (start.elapsed().as_millis() as i64).min(i64::from(u32::MAX));

    let fut = async {
        let mut stream = TcpStream::connect((target.host.as_str(), target.port))
            .await
            .map_err(|e| format!("conexion fallida: {e}"))?;

        match kind {
            CheckKind::Tcp => Ok((true, "conectado".to_string())),
            CheckKind::Http => {
                let req = build_http_request(target);
                stream
                    .write_all(req.as_bytes())
                    .await
                    .map_err(|e| format!("escritura fallida: {e}"))?;

                let mut buf = [0u8; 4096];
                let mut total = 0usize;
                let mut header_end = false;
                while total < buf.len() && !header_end {
                    let n = stream
                        .read(&mut buf[total..])
                        .await
                        .map_err(|e| format!("lectura fallida: {e}"))?;
                    if n == 0 {
                        break;
                    }
                    total += n;
                    header_end = buf[..total].windows(4).any(|w| w == b"\r\n\r\n");
                }

                let text = String::from_utf8_lossy(&buf[..total]);
                let code = parse_http_status(&text)
                    .ok_or_else(|| "respuesta sin status HTTP valido".to_string())?;
                let ok = (100..400).contains(&code);
                Ok((ok, format!("HTTP {code}")))
            }
        }
    };

    match tokio::time::timeout(timeout, fut).await {
        Err(_) => CheckOutcome {
            ok: false,
            latency_ms: elapsed(),
            detail: format!("timeout tras {}s", timeout.as_secs()),
        },
        Ok(Err(e)) => CheckOutcome {
            ok: false,
            latency_ms: elapsed(),
            detail: e,
        },
        Ok(Ok((ok, detail))) => CheckOutcome {
            ok,
            latency_ms: elapsed(),
            detail,
        },
    }
}

/*
 * Estado resumido de un check, tal como esta persistido (para la
 * transicion up<->down).
 */
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CurrentCheckState {
    pub state: String,
    pub since: DateTime<Utc>,
}

/*
 * Siguiente estado de la maquina (pura y testeable): UP/DOWN con `since`.
 * `since` se conserva mientras no cambia el estado; al transicionar se
 * reinicia. Sin estado previo (primer corrida) `since` = ahora.
 */
pub struct StateUpdate {
    pub state: &'static str,
    pub since: DateTime<Utc>,
    pub changed: bool,
}

pub fn next_state(prev: Option<&CurrentCheckState>, ok: bool, now: DateTime<Utc>) -> StateUpdate {
    let state = if ok { "up" } else { "down" };
    match prev {
        Some(p) if p.state == state => StateUpdate {
            state,
            since: p.since,
            changed: false,
        },
        _ => StateUpdate {
            state,
            since: now,
            changed: true,
        },
    }
}

/*
 * Payload de creacion (POST /api/v1/health/checks). Validacion pura,
 * igual que el resto de la API.
 */
#[derive(Debug, Deserialize)]
pub struct CreateHealthCheck {
    pub name: String,
    pub kind: String,
    pub target: String,
    #[serde(rename = "interval_secs")]
    pub interval_secs: i64,
    #[serde(rename = "timeout_secs")]
    pub timeout_secs: Option<i64>,
    pub enabled: Option<bool>,
}

pub struct CheckDraft {
    pub name: String,
    pub kind: CheckKind,
    pub target: String,
    pub interval_secs: i64,
    pub timeout_secs: i64,
    pub enabled: bool,
}

impl CreateHealthCheck {
    pub fn into_draft(self, default_timeout_secs: i64) -> Result<CheckDraft, ApiError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(ApiError::bad_request(
                "invalid_check",
                "name no puede estar vacio",
            ));
        }

        let kind = CheckKind::parse(self.kind.trim()).ok_or_else(|| {
            ApiError::bad_request("invalid_check_kind", "kind debe ser http o tcp")
        })?;

        if !(1..=86_400).contains(&self.interval_secs) {
            return Err(ApiError::bad_request(
                "invalid_check_interval",
                "interval_secs debe estar entre 1 y 86400",
            ));
        }

        let timeout_secs = self.timeout_secs.unwrap_or(default_timeout_secs);
        if !(1..=300).contains(&timeout_secs) {
            return Err(ApiError::bad_request(
                "invalid_check_timeout",
                "timeout_secs debe estar entre 1 y 300",
            ));
        }

        let target = self.target.trim().to_string();
        parse_target(kind, &target)
            .map_err(|m| ApiError::bad_request("invalid_check_target", m))?;

        Ok(CheckDraft {
            name: name.to_string(),
            kind,
            target,
            interval_secs: self.interval_secs,
            timeout_secs,
            enabled: self.enabled.unwrap_or(true),
        })
    }
}

/*
 * Scheduler: cada poll revisa que checks habilitados estan vencidos y los
 * ejecuta; aplica el resultado y la transicion de estado en una sola
 * transaccion.
 */
pub fn spawn_health_runner(pool: PgPool, config: Arc<Config>, bus: EventBus) {
    tokio::spawn(async move {
        let poll = Duration::from_secs(config.health_poll_secs.max(1) as u64);
        let mut ticker = tokio::time::interval(poll);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(e) = run_cycle(&pool, &bus).await {
                tracing::error!("ciclo de health checks fallo: {e}");
            }
        }
    });
}

async fn run_cycle(pool: &PgPool, bus: &EventBus) -> Result<(), sqlx::Error> {
    let checks = db::list_enabled_checks(pool).await?;
    if checks.is_empty() {
        return Ok(());
    }
    let last_ts = db::latest_check_times(pool).await?;
    let now = Utc::now();

    for check in checks {
        let due = match last_ts.get(&check.id) {
            None => true,
            Some(t) => now.signed_duration_since(*t).num_seconds() >= check.interval_secs,
        };
        if !due {
            continue;
        }

        let Some(kind) = CheckKind::parse(&check.kind) else {
            let outcome = CheckOutcome {
                ok: false,
                latency_ms: 0,
                detail: "kind de check invalido en DB".to_string(),
            };
            let prev = db::get_check_state(pool, check.id).await?;
            let update = next_state(prev.as_ref(), false, now);
            db::apply_check_outcome(pool, check.id, &outcome, &update).await?;
            tracing::error!(check_id = check.id, "kind de check invalido en DB");
            continue;
        };

        let outcome = match parse_target(kind, &check.target) {
            Ok(target) => {
                probe(
                    kind,
                    &target,
                    Duration::from_secs(check.timeout_secs as u64),
                )
                .await
            }
            Err(e) => CheckOutcome {
                ok: false,
                latency_ms: 0,
                detail: format!("target invalido: {e}"),
            },
        };

        let prev = db::get_check_state(pool, check.id).await?;
        let update = next_state(prev.as_ref(), outcome.ok, now);
        db::apply_check_outcome(pool, check.id, &outcome, &update).await?;

        /* Evento realtime (bloque 6.2): se publica solo cuando la corrida
         * ya quedo commiteada (resultado + estado), con la transicion
         * up/down del ciclo para que el dashboard la pinte al instante. */
        bus.publish(&crate::events::Event::health(
            check.id,
            &check.name,
            outcome.ok,
            outcome.latency_ms,
            &outcome.detail,
            now,
            update.changed,
            update.state,
            update.since,
        ));

        tracing::info!(
            check_id = check.id,
            check = %check.name,
            ok = outcome.ok,
            latency_ms = outcome.latency_ms,
            detail = %outcome.detail,
            "health check ejecutado"
        );
        if update.changed {
            tracing::info!(
                check_id = check.id,
                check = %check.name,
                state = update.state,
                "health check cambio de estado"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    #[test]
    fn kind_parse_roundtrip() {
        assert_eq!(CheckKind::parse("http"), Some(CheckKind::Http));
        assert_eq!(CheckKind::parse("tcp"), Some(CheckKind::Tcp));
        assert_eq!(CheckKind::parse("icmp"), None);
        assert_eq!(CheckKind::as_str(&CheckKind::Http), "http");
        assert_eq!(CheckKind::as_str(&CheckKind::Tcp), "tcp");
    }

    #[test]
    fn tcp_target_defaults_empty_path() {
        let t = parse_target(CheckKind::Tcp, " router.local:22 ").unwrap();
        assert_eq!(t.host, "router.local");
        assert_eq!(t.port, 22);
        assert_eq!(t.path, "");
    }

    #[test]
    fn tcp_target_rejects_missing_or_bad_port() {
        assert!(parse_target(CheckKind::Tcp, "router.local").is_err());
        assert!(parse_target(CheckKind::Tcp, ":22").is_err());
        assert!(parse_target(CheckKind::Tcp, "router.local:99999").is_err());
        assert!(parse_target(CheckKind::Tcp, "  ").is_err());
    }

    #[test]
    fn http_target_defaults_host_and_path() {
        let t = parse_target(CheckKind::Http, "http://example.com").unwrap();
        assert_eq!(t.host, "example.com");
        assert_eq!(t.port, 80);
        assert_eq!(t.path, "/");
    }

    #[test]
    fn http_target_parses_port_and_path() {
        let t = parse_target(CheckKind::Http, "http://host:8080/status").unwrap();
        assert_eq!(t.host, "host");
        assert_eq!(t.port, 8080);
        assert_eq!(t.path, "/status");
    }

    #[test]
    fn http_target_rejects_bad_port() {
        assert!(parse_target(CheckKind::Http, "http://host:xyz/").is_err());
        assert!(parse_target(CheckKind::Http, "http://host:70000/").is_err());
    }

    #[test]
    fn http_target_rejects_https_and_missing_scheme() {
        let e = parse_target(CheckKind::Http, "https://example.com").unwrap_err();
        assert!(e.contains("Fase 8"));
        assert!(parse_target(CheckKind::Http, "example.com").is_err());
        assert!(parse_target(CheckKind::Http, "").is_err());
    }

    #[test]
    fn http_target_rejects_empty_host() {
        assert!(parse_target(CheckKind::Http, "http:///path").is_err());
        assert!(parse_target(CheckKind::Http, "http://:8080/").is_err());
    }

    #[test]
    fn request_line_omits_default_port() {
        let t = parse_target(CheckKind::Http, "http://example.com/health").unwrap();
        assert_eq!(
            build_http_request(&t),
            "GET /health HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n"
        );
    }

    #[test]
    fn request_line_keeps_non_default_port() {
        let t = parse_target(CheckKind::Http, "http://host:9000/").unwrap();
        let req = build_http_request(&t);
        assert!(req.starts_with("GET / HTTP/1.1\r\nHost: host:9000\r\n"));
    }

    #[test]
    fn status_parsing() {
        assert_eq!(parse_http_status("HTTP/1.1 200 OK\r\n\r\n"), Some(200));
        assert_eq!(parse_http_status("HTTP/1.0 404 Not Found\r\n"), Some(404));
        assert_eq!(parse_http_status("HTTP/1.1 5xx nope\r\n"), None);
        assert_eq!(parse_http_status("garbage\r\n"), None);
    }

    #[test]
    fn state_transition_from_absent() {
        let u = next_state(None, true, ts(100));
        assert_eq!(u.state, "up");
        assert!(u.changed);
        assert_eq!(u.since, ts(100));
        let d = next_state(None, false, ts(100));
        assert_eq!(d.state, "down");
        assert!(d.changed);
    }

    #[test]
    fn state_transition_preserves_since_while_stable() {
        let prev = CurrentCheckState {
            state: "up".into(),
            since: ts(50),
        };
        let u = next_state(Some(&prev), true, ts(100));
        assert_eq!(u.state, "up");
        assert!(!u.changed);
        assert_eq!(u.since, ts(50));
    }

    #[test]
    fn state_transition_flips_up_down() {
        let prev = CurrentCheckState {
            state: "up".into(),
            since: ts(50),
        };
        let d = next_state(Some(&prev), false, ts(100));
        assert_eq!(d.state, "down");
        assert!(d.changed);
        assert_eq!(d.since, ts(100));

        let up = next_state(
            Some(&CurrentCheckState {
                state: "down".into(),
                since: ts(90),
            }),
            true,
            ts(100),
        );
        assert_eq!(up.state, "up");
        assert!(up.changed);
        assert_eq!(up.since, ts(100));
    }

    #[test]
    fn draft_validates_default_timeout() {
        let d = CreateHealthCheck {
            name: "router".into(),
            kind: "tcp".into(),
            target: "host:22".into(),
            interval_secs: 30,
            timeout_secs: None,
            enabled: None,
        }
        .into_draft(5)
        .unwrap();
        assert_eq!(d.timeout_secs, 5);
        assert!(d.enabled);
    }

    #[test]
    fn draft_rejects_bad_kind_target_and_ranges() {
        let base = |name: &str, kind: &str, target: &str, interval: i64| CreateHealthCheck {
            name: name.into(),
            kind: kind.into(),
            target: target.into(),
            interval_secs: interval,
            timeout_secs: Some(5),
            enabled: None,
        };

        assert!(base("x", "icmp", "host:22", 30).into_draft(5).is_err());
        assert!(base("x", "tcp", "noport", 30).into_draft(5).is_err());
        assert!(base("", "tcp", "host:22", 30).into_draft(5).is_err());
        assert!(base("x", "tcp", "host:22", 0).into_draft(5).is_err());
        assert!(base("x", "tcp", "host:22", 90_000).into_draft(5).is_err());
    }
}
