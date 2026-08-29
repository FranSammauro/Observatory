# observer-collector

Collector central en **Rust (Axum)** que recibe los payloads que emite el
agent C (`observer-agent`), los valida, los autentica por bearer token, y
los persiste en PostgreSQL.

> Estado actual: **Fase 7, bloques 7.1 y 7.2 entregados** — dashboard web
> estatico servido por el propio collector con login bearer: Overview
> (summary cards, lista de agents, timeline en vivo por WS) y host page
> (detalle por agent, series con ultimo valor, grafica SVG, timeline del
> host y reboots). Queda 7.3 (alertas e historicos). Fases 6 entera
> (health checks 6.1, WS realtime 6.2, historial unificado + summary +
> conectividad 6.3), 5 entera (alert engine: 5.1, 5.2 y 5.3) y Fases 4,
> 4.2 y 4.3 tambien. Ver [`../PHASES.md`](../PHASES.md).

## Arquitectura

```
Agent (C) --POST /api/v1/metrics--> Collector (Rust/Axum) --> PostgreSQL
          --POST /api/v1/agents/heartbeat-->          ^
                          (Authorization: Bearer <token>)

Dashboard  --GET /api/v1/...--> Collector --> PostgreSQL
                          (Authorization: Bearer <token>)
        --WS /api/v1/events?token=-->

Browser     --GET / (dashboard)--> Collector (HTML+CSS+JS vanilla)
```

## Dashboard (Fase 7)

El collector sirve en su propio listener una UI web estatica de
**HTML+CSS+JS vanilla** (`collector/dashboard/`): sin framework, sin build
step y sin dependencias JS. Consume la REST API y el WebSocket de eventos
de arriba con el mismo bearer token.

- `GET /` — el dashboard (`index.html` + `app.js` + `style.css`), servido
  por cualquier ruta no capturada por la API (`ServeDir` + SPA fallback a
  `index.html`); `/api/*` y `/healthz` ganan por precedencia.
- Login: guarda el token en `sessionStorage` (`obs_token`), lo manda como
  `Authorization: Bearer` en cada fetch y como `?token=` en el WS; un 401
  devuelve al login.
- Vista **Overview** (bloque 7.1): summary cards
  (`GET /api/v1/health/summary`) de agents online/degraded/offline, checks
  up/down/unknown y alertas pending/firing; lista de agents con badge de
  estado y tiempos relativos (`GET /api/v1/agents`); timeline unificado
  (`GET /api/v1/events/history` limit 50) renderizado por `type`, con
  append en vivo de los eventos del WS, deduplicacion y refresco debounced
  del summary.
- Vista **Host page** (bloque 7.2): `host.html?agent=<uuid>` — estado del
  agente (`GET /api/v1/agents/{id}`), alertas activas del host
  (`GET /api/v1/alerts?agent_id=...`), ultimo valor de cada serie
  (`GET /api/v1/agents/{id}/metrics`), grafica SVG sin dependencias de la
  serie elegida (`.../metrics/{metric}?limit=300`, con `entity`),
  timeline del host (`/api/v1/events/history?agent_id=...`) en vivo por
  WS y reboots (`/api/v1/agents/{id}/reboots`). El overview enlaza cada
  agente a su host page.
- Vista **Alertas e historicos**: placeholder (bloque 7.3).

Endpoints:

Ingestion (Fase 3):

- `POST /api/v1/metrics` — sample completo de metricas del agent.
- `POST /api/v1/agents/heartbeat` — heartbeat liviano (mas frecuente).

Query API (Fase 4, bloques 4.1 y 4.2) — de solo lectura, mismo bearer
token:

- `GET /api/v1/agents` — agentes registrados con **estado de
  conectividad derivado** (bloque 4.2): `state` `online|degraded|offline`
  y `last_seen_age_secs`, ordenados por actividad descendente.
- `GET /api/v1/agents/{agent_id}` — detalle de un agente (formato de
  lista + `reboot_count` y `last_reboot`); `404 unknown_agent` si no está
  registrado.
- `GET /api/v1/agents/{agent_id}/reboots` — timeline de reboots
  detectados (`detected_at`, `sample_ts`, `uptime_before`, `uptime_after`).
  Query param `limit` (default 50, tope 1000).
- `GET /api/v1/agents/{agent_id}/metrics` — series del agente: por
  `(metric_name, entity)`, con conteo de muestras, rango temporal y el
  ultimo valor (para la vista "host" del dashboard sin pedir la serie).
- `GET /api/v1/agents/{agent_id}/metrics/{metric}` — serie temporal de
  una metrica. Query params (todos opcionales):
  - `entity` — label de la serie (device/interface/mountpoint). Se omite
    para escalares (entity NULL).
  - `from` / `to` — rango en segundos epoch, inclusive; `from` debe ser
    `<= to`.
  - `limit` — maximo de puntos (default 1000, tope 10000).
  - Respuesta: `{agent_id, metric, entity, from, to, count, points[]}`
    con `points` de `{ts, value}` ordenados ascendentemente.

Estado de conectividad: se **deriva** de `agents.last_seen` (hora de
arribo, ADR-0003), no se persiste. Con `OBS_STATE_ONLINE_SECS` y
`OBS_STATE_DEGRADED_SECS` (default 15s y 60s, base heartbeat 5s):
`age <= online` -> ONLINE, `age <= degraded` -> DEGRADED, sino OFFLINE.

Deteccion de reboot (bloque 4.3): `system.uptime` es monotono, asi que en
ingestion se compara cada sample contra el ultimo uptime del agente; si
cayo mas que `OBS_REBOOT_MIN_UPTIME_DROP_SECS` (default 2s, filtra el
redondeo del segundo) se registra un `reboot_events`. La lectura y el
registro van en una sola transaccion con lock del agente
(`SELECT ... FOR UPDATE`) para que dos ingestiones concurrentes del mismo
host no dupliquen ni pierdan eventos.

Infra:

- `GET /healthz` — chequeo de salud (incluye ping a la DB).

Alert engine (Fase 5) — gestion de reglas y consulta de estado/historial,
mismo bearer token:

- `GET /api/v1/alerts/rules` y `POST /api/v1/alerts/rules` — listar y
  crear reglas declarativas.
  El payload es `{name, metric_name, entity?, op, threshold, for_secs?}`
  con `op` en `ge|gt|le|lt` y `for_secs` default 0:
  - `201` con la regla creada; `400 rule_already_exists` si el `name` ya
    existe (UNIQUE en la DB); `400 invalid_rule` si el payload u `op` no
    es valido (`ge|gt|le|lt`), `entity` no es un label no vacio,
    `threshold` no es finito o `for_secs` es negativo.
- `DELETE /api/v1/alerts/rules/{rule_id}` — borrar una regla; `404
  unknown_rule` si no existe, `400 invalid_rule_id` si no es entero. Su
  alerta activa y su historial se limpian (FK ON DELETE CASCADE).
- `GET /api/v1/alerts` — alertas **activas** (pending/firing) con el
  contexto de la regla (`rule_name`, `metric_name`, `entity`, `op`,
  `threshold`) y el estado de la maquina (`since`, `resolve_from`,
  `checked_at`). Query params opcionales `agent_id` (UUID valido) y
  `state` (`pending|firing`); `400 invalid_alert_state` /
  `invalid_agent_id` si no cumplen.
- `GET /api/v1/alerts/history` — historial de transiciones (creaciones,
  promociones y resoluciones) ordenado DESC; respuesta `{events[],
  count}` con `{id, rule_id, rule_name, agent_id, from_state, to_state,
  ts}`. `from_state` es NULL en la creacion (desde INACTIVE). Query
  params opcionales `agent_id`, `rule_id`, `from`/`to` (epoch,
  `from <= to`) y `limit` (default 50, tope 1000).

Maquina de estados (bloque 5.2): INACTIVE -> PENDING -> FIRING ->
RESOLVED, derivada en `alerts.next_step` a partir del veredicto del
evaluador (`alerts.evaluate_series`). INACTIVE es la ausencia de fila en
`alerts`, y al resolver se borra y se registra el evento. Hysteresis:
una FIRING cuya condicion cae no se resuelve al instante, entra en
`resolve_from` y solo pasa a RESOLVED cuando la ausencia supera
`OBS_ALERT_RESOLVE_GRACE_SECS` (default 60s; 0 = inmediato); una PENDING
cuyo tramo cae se resuelve de inmediato. Los eventos de
`alert_events` (bloque 5.3) se escriben solo en transiciones reales en
la misma transaccion que aplica el estado: idempotente, sin duplicar
creaciones/promociones/resoluciones aunque el evaluador corra de nuevo
sobre la misma alerta.

Health checks (Fase 6, bloque 6.1) — checks HTTP/TCP definidos
declarativamente, evaluados por el propio collector, mismo bearer token:

- `POST /api/v1/health/checks` — crear un check. Payload
  `{name, kind, target, interval_secs, timeout_secs?, enabled?}` con
  `kind` en `http|tcp`:
  - `http`: target `http://host[:port]/ruta` (GET minimal, ok si responde
    2xx/3xx). El esquema `https://` se rechaza por ahora (TLS llega en la
    Fase 8).
  - `tcp`: target `host:puerto` (ok si el puerto acepta conexion).
  - `interval_secs` entre 1 y 86400 (obligatorio); `timeout_secs` entre 1
    y 300 (default `OBS_HEALTH_DEFAULT_TIMEOUT_SECS`, 5s); `enabled`
    default true.
  - `201` con el check creado; `400 check_already_exists` si el `name` ya
    existe (UNIQUE), `400 invalid_check_kind` / `invalid_check_interval`
    / `invalid_check_timeout` / `invalid_check_target` segun el campo
    invalido.
- `GET /api/v1/health/checks` — lista de checks con su **estado
  derivado** (`state` `up|down|null`, `since`,
  `last_checked_at`, `last_ok`, `last_latency_ms`, `last_detail`).
- `DELETE /api/v1/health/checks/{check_id}` — borrar un check; `404
  unknown_check` / `400 invalid_check_id`. Sus resultados y estado se
  limpian (FK ON DELETE CASCADE).
- `GET /api/v1/health/checks/{check_id}/results` — historial de corridas
  ordenado DESC: `{check_id, results[], count}` con `{ts, ok,
  latency_ms, detail}`. Query param `limit` (default 50, tope 1000);
  `404 unknown_check` si el check no existe.

Eventos realtime (bloque 6.2): `GET /api/v1/events` hace upgrade a
WebSocket y suscribe al `EventBus` (broadcast `tokio`, capacidad
`OBS_WS_CHANNEL_CAPACITY`). Los clientes reciben solo los eventos
posteriores a la conexion (sin replay — el historial queda en la REST
API), como JSON con `type` como tag:

- `health_result` — una por corrida de check: `{type, check_id,
  check_name, ok, latency_ms, detail, ts, state_changed, state, since}`.
- `alert_event` — una por transicion de alerta: `{type, rule_id,
  rule_name, agent_id, from_state, to_state, ts}` (INACTIVE/PENDING →
  PENDING, PENDING → FIRING, FIRING → PENDING, FIRING → RESOLVED).
- `connectivity_event` — una por transicion del estado derivado de un
  agent (bloque 6.3): `{type, agent_id, from_state, to_state, ts}`
  (online/degraded/offline; `from_state` NULL en la primera
  observacion). Lo publica el runner de conectividad, no persiste NADA
  por si mismo.
- `events_lagged` — si el cliente no consume a tiempo, el broadcast
  descarta los eventos atrasados y se avisa `{type, dropped}` antes de
  seguir.

Auth: mismo bearer que el resto de la API. Como un WebSocket de
navegador no deja setear headers, el token se acepta tambien por query
param `?token=...`; se valida antes del upgrade (`401` sin token
valido). Los eventos se publican despues del commit en DB, con la misma
atomicidad que los datos persistidos.

Historial unificado (bloque 6.3) — el timeline del dashboard, mismo
bearer token:

- `GET /api/v1/events/history` — cruza las cuatro fuentes de eventos
  (alertas, health checks, reboots y conectividad) en orden cronologico
  desc y devuelve `{events, count}`. Cada evento mantiene el mismo shape
  que el WebSocket (campo `type`: `alert_event` | `health_result` |
  `reboot_event` | `connectivity_event`, mas `ts`). Query params:
  `agent_id` (UUID, opcional; filtra alertas/reboots/conectividad) y
  `limit` (default 50, tope 1000; `400 invalid_limit` fuera de rango).
- `GET /api/v1/health/summary` — agrega el estado de la plataforma en un
  GET: `{agents: {total, online, degraded, offline}, checks: {total, up,
  down, unknown}, alerts: {total, pending, firing}}`. La conectividad de
  los agents usa la misma funcion derivada que el query API (bloque 4.2);
  `unknown` = checks definidos que aun no tienen estado (nunca corrieron
  o `enabled = false`).

Evaluacion (bloque 6.1): el runner (`health::spawn_health_runner`, en
una tarea tokio) consulta cada `OBS_HEALTH_POLL_SECS` los checks
habilitados y corre los que estan vencidos (`interval_secs`). Un probe
HTTP es un GET minimal sobre `TcpStream` `tokio` (sin dependencias
nuevas, mismo espiritu que el transport.c del agent); `Connection:
close` y lectura del status line. La maquina de estados `up|down`
(`health::next_state`, pura) conserva `since` mientras no cambia y en
cada corrida se persiste en una transaccion: INSERT del resultado +
UPSERT del estado. Los checks con `enabled = false` no se corren.

Contrato de payloads: ver [`../agent/src/protocol.c`](../agent/src/protocol.c)
(serializador del agent) y [`../docs/adr/0003-collector-ingestion.md`]
(../docs/adr/0003-collector-ingestion.md) para las decisiones de diseno.

## Requisitos

- Rust toolchain (edition 2021).
- PostgreSQL 14+ corriendo (local o en contenedor).

## Configuracion (env)

| Variable | Default | Descripcion |
|---|---|---|
| `DATABASE_URL` | (requerida) | `postgres://user:pass@host:5432/db` |
| `OBS_COLLECTOR_TOKEN` | (requerida) | token bearer compartido que los agents envian. El arranque **falla** si falta. |
| `OBS_LISTEN_ADDR` | `0.0.0.0:8080` | direccion/port de escucha |
| `OBS_DB_MAX_CONNECTIONS` | `5` | tamano del pool de conexiones |
| `OBS_INGEST_FUTURE_SKEW_SECS` | `60` | timestamps futuros tolerados |
| `OBS_INGEST_MAX_AGE_SECS` | `600` | antiguedad maxima aceptada de un sample |
| `OBS_MAX_BODY_BYTES` | `262144` | limite de body (el agent manda <= 16 KB) |
| `OBS_STATE_ONLINE_SECS` | `15` | antiguedad de `last_seen` para ONLINE (bloque 4.2) |
| `OBS_STATE_DEGRADED_SECS` | `60` | antiguedad de `last_seen` para DEGRADED (bloque 4.2) |
| `OBS_REBOOT_MIN_UPTIME_DROP_SECS` | `2.0` | caida minima de uptime para considerar reboot (bloque 4.3) |
| `OBS_ALERT_EVAL_INTERVAL_SECS` | `15` | periodo del evaluador de alertas (bloque 5.1) |
| `OBS_ALERT_LOOKBACK_SECS` | `300` | ventana de muestras que el evaluador consulta por regla (bloque 5.1) |
| `OBS_ALERT_RESOLVE_GRACE_SECS` | `60` | ventana de hysteresis de una alerta FIRING (bloque 5.2); 0 = resolucion inmediata, no puede ser negativa |
| `OBS_HEALTH_POLL_SECS` | `1` | periodo del runner de health checks (bloque 6.1) |
| `OBS_HEALTH_DEFAULT_TIMEOUT_SECS` | `5` | timeout por defecto de un check (1-300), si el payload no trae `timeout_secs` (bloque 6.1) |
| `OBS_WS_CHANNEL_CAPACITY` | `256` | capacidad del canal broadcast de eventos WS; suscriptores lentos descartan eventos (bloque 6.2) |
| `OBS_CONNECTIVITY_POLL_SECS` | `5` | periodo del runner de conectividad (bloque 6.3); >= 1, fail-fast al arrancar |
| `OBS_DASHBOARD_DIR` | `dashboard` | carpeta relativa (a CWD) con la UI estatica del dashboard (bloque 7.1); no puede quedar vacia |
| `RUST_LOG` | `info` | nivel de log (tracing) |

## Build y tests

```sh
cargo build            # debug
cargo build --release  # release (LTO + codegen-units=1)
cargo test             # 140 tests unitarios (sin DB)
cargo clippy           # lint, sin warnings
cargo fmt              # formato
```

Las migraciones estan en `migrations/` y se aplican **al arrancar**
(`sqlx::migrate!`), asi no hace falta `sqlx-cli` para desplegar.

## Correr en local

Con Postgres local (ver `deploy/docker-compose.yml` para la opcion de
contenedor):

```sh
DATABASE_URL="postgres://observer@127.0.0.1:55432/observer" \
OBS_COLLECTOR_TOKEN="tu-token" \
cargo run
```

Luego el agent apunta al collector:

```ini
# /etc/observer/agent.conf
collector_url = http://127.0.0.1:8080
agent_token = tu-token
```

## Esquema

- `agents` — identidad del agent (id UUID de 128 bits, `first_seen`,
  `last_seen`). El registro es implicito: el primer heartbeat/sample
  valido lo crea (ADR-0003).
- `metric_samples` — una fila por `(agent_id, ts, metric_name, entity)`:
  los escalares de `metrics` usan `entity = NULL`; las metricas de los
  arrays (disk/network/filesystem) usan `entity` = device / interface /
  mountpoint.
- `reboot_events` — una fila por reboot detectado (bloque 4.3):
  `detected_at`, `sample_ts`, `uptime_before`, `uptime_after`.
- `alert_rules` — reglas declarativas (Fase 5): `name` UNIQUE,
  `metric_name`, `entity`, `op`, `threshold`, `for_secs` (default 0),
  `enabled`.
- `alerts` — estado **actual** de una alerta activa, deduplicado por PK
  `(rule_id, agent_id)`: `state` (`pending|firing`), `since`, opcional
  `resolve_from` (ventana de hysteresis en curso). INACTIVE = ausencia
  de fila; RESOLVED = borrado al resolver.
- `alert_events` — historial de transiciones (bloque 5.3): una fila por
  transicion real (`from_state`/`to_state`/`ts`); FK a `alert_rules`
  con ON DELETE CASCADE.
- `health_checks` — checks declarativos (bloque 6.1): `name` UNIQUE,
  `kind` (`http|tcp` con CHECK), `target`, `interval_secs`,
  `timeout_secs`, `enabled`.
- `health_check_results` — una fila por corrida (bloque 6.1): `ts`,
  `ok`, `latency_ms`, `detail`; FK a `health_checks` con ON DELETE
  CASCADE, indice por `(check_id, ts)`.
- `health_check_states` — estado **actual** del check (bloque 6.1),
  deduplicado por PK `check_id`: `state` (`up|down`), `since`,
  `last_checked_at`, `last_ok`, `last_latency_ms`, `last_detail`.
  INACTIVO = ausencia de fila (no corrió aun o `enabled = false`).
- `connectivity_events` — historial de transiciones de conectividad
  (bloque 6.3): una fila por transicion del estado derivado
  (`from_state` NULL en la primera observacion, `to_state`
  `online|degraded|offline`, `ts`); FK a `agents` con ON DELETE CASCADE,
  indice por `(agent_id, ts)`. `agents.last_connectivity_state` guarda el
  ultimo estado observado por el runner (NULL hasta la primera pasada).

## Limites de cardinalidad

El Collector valida del lado del servidor lo que el agent ya acota
(`OBS_MAX_DISKS/INTERFACES/FILESYSTEMS = 16`): arrays con mas de 16
entradas -> `400 too_many_entities`; el objeto `metrics` con mas de
1024 claves -> `400 too_many_metrics` (el agent emite ~14). Los valores
no finitos (NaN/Inf) no pueden llegar a la DB: `serde_json` los rechaza
a nivel de parsing (`400 invalid_json`).

Ver [`../PHASES.md`](../PHASES.md).