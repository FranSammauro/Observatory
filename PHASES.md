# Fases del proyecto

Mapeo del roadmap del informe técnico (Milestones 0–8) a fases de
desarrollo con commits progresivos.

- [x] **Fase 1 — Agent core (C)**
      Estructura `agent/`, collectors de CPU y memoria (snapshot/delta),
      config parsing, logging, serialización JSON del payload, tests
      unitarios, build limpio con `-Wall -Wextra -Wpedantic -Wconversion
      -Wshadow` y con AddressSanitizer/UBSan. ADR-0001 (elección de C).
- [x] **Fase 2 — Agent: resto de collectors + transporte**
      Disk, network, filesystem, uptime, process count, temperatura
      opcional, `transport.c` (HTTP client con timeouts), retry con
      backoff+jitter, heartbeat, identidad persistente del agent.
- [x] **Fase 3 — Collector core (Rust)**
      Proyecto Axum, migraciones PostgreSQL (`agents`, `metric_samples`),
      registro de agentes, ingestion con validación, autenticación por
      bearer token. Ver detalle abajo. ADR-0003.
- [x] **Fase 4 — Collector: query API + estados de conectividad**
      Endpoints GET, máquina de estados ONLINE/DEGRADED/OFFLINE, detección
      de reboot. Subdividida en 3 bloques: **4.1** query API de lectura
      (entregado), **4.2** máquina de estados (entregado), **4.3**
      detección de reboot (entregado). Ver detalle abajo.
- [ ] **Fase 5 — Alert engine**
      Reglas declarativas, máquina de estados
      INACTIVE→PENDING→FIRING→RESOLVED, hysteresis, deduplicación,
      historial. Subdividida en 3 bloques: **5.1** reglas declarativas y
      evaluador (entregado), **5.2** máquina de estados + hysteresis
      (entregado), **5.3** deduplicación + historial + query API
      (entregado). Ver detalle abajo.
- [x] **Fase 6 — Health checks + WebSocket**
      Checks HTTP/TCP, eventos realtime. Subdividida en 3 bloques:
      **6.1** health checks HTTP/TCP (definiciones + ejecutor +
      resultados + estado + query API, entregado), **6.2** WebSocket de
      eventos realtime (canal + publicación desde alertas y health
      checks, entregado), **6.3** historial unificado + summary de salud +
      eventos de conectividad (entregado). Ver detalle abajo.
- [ ] **Fase 7 — Dashboard**
      UI web estatica servida por el propio collector (sin framework ni
      build step, filosofia del repo). Subdividida en 3 bloques: **7.1**
      overview + skeleton (entregado): login bearer + layout +
      navegacion, summary cards, lista de agents y timeline unificado en
      vivo por WS; **7.2** host page (detalle por agent: estado, series
      de metricas, reboots, conectividad, alertas del host); **7.3**
      alertas e historicos (gestion de rules y checks + historiales
      completos). Ver detalle abajo.
- [ ] **Fase 8 — Hardening y benchmark experimental**
      TLS, rate limiting, sanitizers/fuzzing, benchmark reproducible en
      Pentium M + Alpine Linux.

## Fase 1 — detalle

**Entregado:**

- `agent/include/` — headers: `agent.h`, `logging.h`, `config.h`,
  `protocol.h`, `collectors/cpu.h`, `collectors/memory.h`.
- `agent/src/` — implementaciones correspondientes + `main.c` (loop que
  arma un `obs_sample_t` cada `metrics_interval_secs` y lo imprime como
  JSON; el envío HTTPS real llega en Fase 2).
- `agent/tests/` — `test_cpu.c`, `test_memory.c`, `test_config.c`.
- `agent/Makefile` — targets `all`, `debug`, `sanitize`, `test`, `clean`.
- `agent/README.md`.
- `docs/adr/0001-agent-language.md`.

**Validado en este entorno:**

- Build limpio (`make all`) sin warnings bajo
  `-Wall -Wextra -Wpedantic -Wconversion -Wshadow`.
- `make test` — 3/3 suites en verde.
- `make sanitize` + ejecución real contra `/proc/stat` y `/proc/meminfo`
  del sistema — sin hallazgos de ASan/UBSan.
- Comportamiento verificado: primera muestra de CPU se descarta
  (`system.cpu.*` ausente) porque aún no hay delta; memoria se reporta
  desde la primera lectura.

**Pendiente explícitamente para después (no es parte de Fase 1):**

- Envío real por HTTPS (`transport.c`).
- Heartbeat como canal separado.
- Generación/persistencia real de `agent_id` (hoy hay un placeholder si
  `/etc/observer/agent-id` no existe).
- Resto de collectors (disk, network, filesystem, uptime, procesos,
  temperatura, métricas custom).

## Fase 2 — detalle

**Entregado:**

- `agent/src/collectors/{disk,network,filesystem,uptime,process,temperature}.c`
  + headers correspondientes.
- `agent/src/transport.c` — cliente HTTP sobre sockets POSIX (sin
  libcurl ni otra dependencia externa), con connect/write/read timeouts
  explícitos. `https://` se rechaza explícitamente (TLS diferido a
  Fase 8 — ver ADR-0002).
- `agent/src/retry.c` — backoff exponencial + jitter (xorshift32,
  reproducible con seed).
- `agent/src/identity.c` — identidad persistente del agent (128 bits
  vía `/dev/urandom`, persistida con permisos 0600, con manejo de
  carrera entre instancias concurrentes vía `O_EXCL`).
- `agent/src/protocol.c` reescrito: serializa las 7 categorías de
  métricas (cpu, memory, uptime, process, temperature como escalares;
  disk, network, filesystem como arrays) + un payload de heartbeat
  separado y más liviano.
- `agent/src/main.c` reescrito: scheduling con `CLOCK_MONOTONIC`
  (nunca wall clock) para metrics y heartbeat de forma independiente;
  ante fallo de envío, backoff sin bloquear el loop ni el otro canal.
- 8 suites de tests nuevas (`test_disk`, `test_network`,
  `test_filesystem`, `test_uptime`, `test_process`, `test_retry`,
  `test_transport`, `test_identity`) — 11/11 en total.
- `docs/adr/0002-transport-protocol.md`.

**Validado en este entorno:**

- Build limpio (`make all`) sin warnings.
- `make test` — 11/11 suites en verde.
- `make sanitize` (ASan + UBSan) corriendo un flujo real de ~16 requests
  HTTP contra un mock collector local (heartbeat + metrics con retry
  incluido) — sin hallazgos.
- Prueba de integración manual: agent real → mock collector Python,
  confirmando JSON bien formado, header `Authorization: Bearer`
  correcto, y backoff exponencial real (1s → 2s → 4s) ante fallos de
  conexión.

**Pendiente explícitamente para después (no es parte de Fase 2):**

- TLS en el transporte (Fase 8 — ver ADR-0002).
- El Collector en Rust que efectivamente reciba estos payloads
  (Fase 3+) — hasta ahora solo se validó contra un mock de prueba, no
  forma parte del repo.
- Unit de systemd para correr el agent como daemon.
- Buffer local / spool en disco ante interrupciones prolongadas
  (explícitamente fuera de alcance para V1 según el informe, sección 27).

## Fase 3 — detalle

**Entregado:**

- `collector/` — crate Rust (Axum 0.8 + sqlx/tokio/serde, sin ninguna
  otra dependencia de aplicacion).
- `collector/migrations/0001_init.sql` — `agents` (id UUID 128-bit,
  first_seen, last_seen) y `metric_samples` (una fila por
  agent+timestamp+metrica, con `entity` NULL para escalares y
  device/mountpoint/interface para las metricas por-entidad).
  Migraciones embebidas (`sqlx::migrate!`) aplicadas al arrancar.
- Endpoints: `POST /api/v1/metrics`, `POST /api/v1/agents/heartbeat`,
  `GET /healthz`. Implementan el contrato exacto que el agent emite
  (`agent/src/protocol.c`) sin tocar el agent.
- Autenticacion `Authorization: Bearer <token>` con token compartido
  (`OBS_COLLECTOR_TOKEN`, requerido al arrancar — fail-fast), comparacion
  en tiempo casi-constante.
- Registro de agentes implicito: el primer payload valido hace
  `INSERT ... ON CONFLICT` en `agents` (mantiene first_seen, actualiza
  last_seen).
- Validacion de ingestion: `protocol_version` (== 1), `agent_id` UUID,
  ventana temporal (`OBS_INGEST_FUTURE_SKEW_SECS` default 60s /
  `OBS_INGEST_MAX_AGE_SECS` default 600s), cardinalidad de arrays (max
  16, igual que los `OBS_MAX_*` del agent) y de claves de `metrics` (max
  1024), entidades no vacias, body size limit. NaN/Inf rechazados por el
  parser de JSON.
- `agents.last_seen` se actualiza con la hora de **arribo** al servidor
  (no con el timestamp que reporta el agent), para que la liveness de la
  Fase 4 no dependa del reloj de cada host.
- Modelo de datos resuelto en `docs/adr/0003-collector-ingestion.md`
  (las 5 decisiones abiertas de ingestion).
- `deploy/` — `docker-compose.yml` (Postgres de desarrollo, puerto 55432)
  y `.env.example`.
- 17 tests unitarios (auth, parsing/flatten de payloads, validacion
  temporal, de entidades y de cardinalidad) — sin warnings.

**Validado en este entorno:**

- Build limpio (`cargo build`) y `cargo clippy`/`cargo fmt` sin
  hallazgos; 16/16 tests en verde.
- Integración real: postgres local scratch + collector + agent real
  (`observer-agent 0.2.0-phase2`) corriendo 8s. Verificado en DB: agente
  registrado (1 fila en `agents`, last_seen avanzando), 399 filas en
  `metric_samples`, escalares con entity NULL y entidades por-device/
  mountpoint bien pobladas (disk sda/sdb/sdc, filesystem /, /boot,
  /home), con el header bearer correcto.
- Casos negativos por HTTP: sin token / token incorrecto -> 401;
  protocol_version erroneo -> 400 `unsupported_protocol_version`;
  timestamp fuera de ventana -> 400 `timestamp_out_of_range`; json
  malformado -> 400 `invalid_json`; array de >16 entidades -> 400
  `too_many_entities`; >1024 claves en `metrics` -> 400
  `too_many_metrics`; body > 256 KB -> 413. NaN y overflow numerico
  (`1e999`) rechazados por el parser (400) — nunca llegan a la DB.
- Token desalineado (agent con otro token) -> el agent real recibe 401
  y entra en backoff 1s/2s/4s sin colgarse.
- `last_seen` usa hora de arribo: heartbeat con ts 300s atras (dentro de
  ventana) -> 200 y `last_seen` = ahora del servidor.
- Shutdown graceful por SIGTERM ("shutdown signal recibido").

**Pendiente para Fases 4+ (no es parte de Fase 3):**

- Maquina de estados ONLINE/DEGRADED/OFFLINE (Fase 4 — ver ADR-0003, la
  DB ya mantiene last_seen para derivarla).
- Deteccion de reboot al comparar uptime entre muestras (Fase 4).
- Tokens per-agent y TLS (Fase 8).

## Fase 4 — detalle

**Subdivision en 3 bloques:**

- **Bloque 4.1 — Query API de lectura (entregado):**
  - `GET /api/v1/agents` — agentes registrados (id, first_seen,
    last_seen), ordenados por actividad descendente.
  - `GET /api/v1/agents/{agent_id}/metrics` — series del agente
    agrupadas por `(metric_name, entity)` con conteo, rango temporal y el
    ultimo valor (subconsulta correlacionada con `$1` constante, indice
    de 0001_init.sql).
  - `GET /api/v1/agents/{agent_id}/metrics/{metric}` — serie temporal con
    filtros opcionales `entity`, `from`/`to` (epoch, inclusive, `from <=
    to`) y `limit` (default 1000, tope 10000). Respuesta JSON con `ts`
    RFC3339 y `value`, ordenada ASC. SQL con `($n IS NULL OR col = $n)`
    para evitar armar queries dinamicos.
  - Validacion de los parametros en `src/query.rs` (pura, sin DB):
    entity no vacia, de `from`/`to` (timestamps validos, `from <= to`) y
    de `limit` (1..=10000). Parametros malformados -> 400 `invalid_query`.
  - Los GET usan el mismo bearer token compartido que la ingestion.
  - `src/db.rs`: `list_agents`, `list_agent_series`, `query_series`.
  - 8 tests unitarios nuevos en `src/query.rs` (25 en total).

- **Bloque 4.2 — Maquina de estados ONLINE/DEGRADED/OFFLINE (entregado):**
  - Estado **derivado** de `agents.last_seen` (hora de arribe, ADR-0003):
    funcion pura, no se persiste ni hay flujo de transiciones. Regla:
    `age <= online_secs` -> ONLINE, `age <= degraded_secs` -> DEGRADED,
    sino OFFLINE. `last_seen` futuro cuenta como ONLINE.
  - Umbrales configurables: `OBS_STATE_ONLINE_SECS` (default **15s**,
    ~3 heartbeats de 5s) y `OBS_STATE_DEGRADED_SECS` (default **60s**),
    validados al arrancar (`degraded >= online`, ninguno negativo — fail
    fast, filosofia ADR-0002/0003).
  - Exposicion en el query API: `GET /api/v1/agents` incluye `state` +
    `last_seen_age_secs`; nuevo `GET /api/v1/agents/{agent_id}` (detalle,
    `404 unknown_agent` si no existe).
  - `src/state.rs` (`ConnectivityState`, `connectivity_state()`),
    `src/db.rs::get_agent`, `ApiError::not_found`.
  - 10 tests unitarios nuevos (4 de config + 6 de state; 35 en total).

- **Bloque 4.3 — Deteccion de reboot (entregado):**
  - `system.uptime` es monotono (segundos desde el boot del host); en
    ingestion se compara cada sample contra el ultimo uptime del agente
    y, si cayo mas que la tolerancia, se registra un `reboot_events`.
  - Migracion `migrations/0002_reboot_events.sql`: tabla `reboot_events`
    (agent_id FK, `detected_at` hora de arribo, `sample_ts` del agent,
    `uptime_before`/`uptime_after`) + indice `(agent_id, detected_at)`.
  - Tolerancia `OBS_REBOOT_MIN_UPTIME_DROP_SECS` (default **2.0s**)
    validada no-negativa al arrancar; filtra el redondeo del segundo de
    `/proc/uptime` sin perder un reboot real (que cae a segundos desde
    cero). `src/reboot.rs::detect_reboot` (puro, rechaza NaN/Inf).
  - `src/db.rs::ingest_sample` reemplaza a `insert_metrics`: lectura del
    uptime previo (`ORDER BY ts DESC, id DESC LIMIT 1`), deteccion,
    insert de metricas y de `reboot_events` en UNA transaccion con lock
    del agente (`SELECT ... FOR UPDATE`) para serializar ingestiones
    concurrentes del mismo host. Previo por ts con tiebreak de id
    (determinista ante timestamps iguales).
  - Query API: `GET /api/v1/agents/{agent_id}/reboots` (timeline,
    `limit` default 50 tope 1000); detalle de agente con
    `reboot_count` + `last_reboot`.
  - 13 tests unitarios nuevos (8 reboot + 2 config + 3 query; 48 en
    total).

**Validado en este entorno:**

- Build limpio (`cargo build`), `cargo clippy` y `cargo fmt` sin
  hallazgos; 25/25 tests en verde (bloque 4.1) y 35/35 tras el bloque
  4.2.
- Integración real: postgres de desarrollo + collector + 3 samples
  inyectados por `POST /api/v1/metrics`. Verificado: `GET /api/v1/agents`
  (1 agente, last_seen avanzando), series del agente (7 series con
  entity NULL y "sda" correctos, latest_value = ultima muestra), serie
  de `system.memory.total` con 4 puntos ordenados ASC, filtro `entity=sda`
  (3 puntos), rango `from`/`to` (2 puntos), `limit=2`.
- Casos negativos por HTTP: sin token -> 401; `agent_id` invalido -> 400
  `invalid_agent_id`; `from > to` -> 400 `invalid_time_range`; `entity`
  vacia -> 400 `invalid_entity`; `limit=0` -> 400 `invalid_limit`; `from`
  no numerico -> 400 `invalid_query`.
- Bloque 4.2 validado en este entorno: collector con
  `OBS_STATE_ONLINE_SECS=10 OBS_STATE_DEGRADED_SECS=30`, 2 heartbeats
  reales y `last_seen` envejecidos via SQL (20s, 40s, 13min). Estados
  derivados correctos: `age 13s` -> degraded, `age 25s` -> degraded,
  `age 45s` -> offline, `age 812s` -> offline. `GET /api/v1/agents/{id}`
  devuelve el detalle con estado; desconocido -> 404 `unknown_agent`; sin
  token -> 401. La DB contenía ademas un agente de la prueba del bloque
  4.1 (13min) correctamente marcado OFFLINE.
- Bloque 4.3 validado en este entorno: collector real + 5 samples con
  uptimes `3600.5 -> 3601.5 -> 3601.9 -> 3.2 -> 5.4` (ts ascendentes).
  Solo `3.2` (caida de ~3598s) marco `reboot_detected=true`; las subidas
  y la caida de 0.6s (redondeo < 2s de tolerancia) no. `GET .../reboots`
  devolvio el evento con `uptime_before=3601.9`, `uptime_after=3.2`;
  detalle con `reboot_count=1` y `last_reboot`. Descubierto y corregido
  en el camino: con timestamps identicos `ORDER BY ts DESC` era no
  determinista (falso reboot); se agrego tiebreak `id DESC`. Agent sin
  reboots -> `reboot_count=0`, `last_reboot=null`. `limit=0` -> 400;
  sin token -> 401.
- Cierre de Fase 4: blocques 4.1, 4.2 y 4.3 validados con postgres real
  (migraciones 0001 + 0002 aplicadas al arranque), build limpio y 48
  tests en verde.

## Fase 5 — detalle

**Subdivision en 3 bloques:**

- **Bloque 5.1 — Reglas declarativas y evaluador (entregado):**
  - Migracion `0003_alert_rules.sql`: tabla `alert_rules` (regla
    declarativa: `name` unico, `metric_name`, `entity` opcional — NULL
    para metricas escalares, label exacto (device/interface/mountpoint)
    para las por-entidad —, `op` ge/gt/le/lt con CHECK, `threshold`
    finito, `for_secs` >= 0, `enabled`). `for_secs` lo consume la
    maquina de estados del bloque 5.2.
  - `src/alerts.rs`:
    - `CondOp` + `parse_op` (ge/gt/le/lt) y `evaluate_series` — funcion
      pura sobre la serie temporal leida de `metric_samples`:
      `NoData` (sin muestras en la ventana), `NotHolding` (la muestra
      mas reciente no satisface la condicion) u `Holding { since,
      holds_for_secs, meets_for }` (se sostiene, desde cuando y si ya
      alcanzo `for_secs` — la base del PENDING/FIRING de 5.2). Valores
      no finitos (NaN/Inf) nunca satisfacen la condicion.
  - Evaluador periodico: task de tokio lanzada en `main.rs` que cada
    `OBS_ALERT_EVAL_INTERVAL_SECS` (default **15s**) carga las reglas
    habilitadas, consulta la ventana `OBS_ALERT_LOOKBACK_SECS` (default
    **300s**) de `metric_samples` por (regla, agent) y evalúa la
    condicion. En 5.1 solo logueaba; desde 5.2 transiciona y persiste.
  - Gestion de reglas por API (mismo bearer token que el resto):
    `POST /api/v1/alerts/rules` (201 con la regla creada),
    `GET /api/v1/alerts/rules`, `DELETE /api/v1/alerts/rules/{id}`
    (404 `unknown_rule`). Validacion: name no vacio y unico
    (duplicado -> 400 `rule_already_exists`), metric_name no vacio,
    entity no vacia si viene, op en {ge,gt,le,lt}, threshold finito,
    for_secs >= 0.
  - `src/db.rs`: `create_rule`, `list_rules`, `list_enabled_rules`,
    `delete_rule`, `recent_samples_for_rule`. `src/config.rs`:
    `OBS_ALERT_EVAL_INTERVAL_SECS` / `OBS_ALERT_LOOKBACK_SECS`
    validados positivos al arrancar (fail-fast, filosofia ADR-0002/0003).
  - Tests unitarios (evaluacion de series + config).

- **Bloque 5.2 — Maquina de estados + hysteresis (entregado):**
  - Migracion `0004_alerts.sql`: tabla `alerts` con el estado **actual**
    de cada alerta activa, deduplicado por `PRIMARY KEY (rule_id,
    agent_id)` (una alerta por regla+agent). INACTIVE y RESOLVED no se
    persisten: INACTIVE es la ausencia de fila, y al resolverse la fila
    se borra.
  - Maquina de estados **INACTIVE→PENDING→FIRING→RESOLVED** en
    `src/alerts.rs::next_step` — funcion pura y testeable sobre
    `CurrentAlert` (estado + `resolve_from`) y el veredicto de
    `evaluate_series` (bloque 5.1):
    - condicion ausente sin fila -> INACTIVE;
    - condicion sostenida sin fila -> PENDING (o FIRING directo si ya
      cubre `for_secs`);
    - PENDING -> FIRING al cubrir `for_secs`; PENDING que cae ->
      RESOLVED al instante;
    - FIRING que cae no se resuelve en el acto: entra en
      **hysteresis** (`resolve_from`), se mantiene FIRING mientras la
      condicion este ausente y solo -> RESOLVED cuando la ausencia
      supera `OBS_ALERT_RESOLVE_GRACE_SECS` (default **60s**, validado
      no-negativo; 0 = resolucion inmediata). Si la condicion se
      reestablece, se limpia la ventana.
  - El evaluador aplica los pasos en UNA transaccion por ciclo
    (`db::apply_alert_steps`): UPSERT (crear/actualizar con `since` del
    veredicto y `resolve_from = NULL`), `StartResolving` (abre la ventana)
    y DELETE (RESOLVED). Ademas resuelve alertas activas de agents que
    dejaron de reportar (sin serie en la ventana -> condicion ausente) y
    de reglas deshabilitadas/borradas.
  - `src/alerts.rs`: `AlertState`, `CurrentAlert`, `Holding`, `Step`,
    `AlertOp`; `src/db.rs`: `list_current_alerts`, `apply_alert_steps`;
    `src/config.rs`: `OBS_ALERT_RESOLVE_GRACE_SECS`.
  - 15 tests unitarios nuevos (12 `next_step` + 3 config; 77 en total).

- **Bloque 5.3 — Deduplicacion + historial + query API (entregado):**
  - Migracion `0005_alert_events.sql`: historial de transiciones
    (timeline reusable por el dashboard). Una fila por transicion REAL de
    la maquina: `from_state` (NULL = desde INACTIVE), `to_state`
    (`pending|firing|resolved`) y `ts` (hora de arribo del ciclo).
    FK a `alert_rules` con ON DELETE CASCADE; indices por (agent, ts) y
    (rule, ts).
  - Deduplicacion: una sola alerta activa por (rule, agent) via la PK de
    `alerts`, y transiciones idempotentes — los eventos se escriben en la
    MISMA transaccion que aplica el estado y solo cuando la maquina
    cambia de estado (los pasos Stay*/StartResolving no emiten),
    por lo que re-evaluar una alerta ya transicionada no vuelve a emitir
    nada. En el evaluador, `AlertOp` lleva el `Event` a insertar; en la
    DB, `apply_alert_steps` lo persiste junto al UPSERT/DELETE del
    estado: atomicidad garantizada.
  - Query API (`src/query.rs`, `src/db.rs`, `src/routes.rs`) — mismo
    bearer token y validacion pura en parametros:
    - `GET /api/v1/alerts` — alertas activas (pending/firing) con
      contexto de la regla (`rule_name`, `metric_name`, `entity`, `op`,
      `threshold`) y el estado (`since`, `resolve_from`, `checked_at`).
      Filtros opcionales `agent_id` (UUID valido, `400 invalid_agent_id`)
      y `state` (`pending|firing`, `400 invalid_alert_state`).
    - `GET /api/v1/alerts/history` — historial desc por `ts`: `{id,
      rule_id, rule_name, agent_id, from_state, to_state, ts}`. Filtros
      opcionales `agent_id`, `rule_id` (positivo), `from`/`to` epoch
      (`from <= to`) y `limit` (`DEFAULT_ALERT_HISTORY_LIMIT=50`,
      tope `MAX_ALERT_HISTORY_LIMIT=1000`, `400 invalid_limit`).
  - 10 tests unitarios nuevos (parsing/validacion de `AlertsQuery` y
    `HistoryQuery`); 87 en total.

**Validado en este entorno (bloques 5.1 + 5.2):**

- Build limpio (`cargo build`), `cargo clippy` y `cargo fmt` sin
  hallazgos; 77 tests en verde (62 en el bloque 5.1, 77 al cerrar 5.2).
- Bloque 5.1: postgres real + collector + reglas por API. Creacion
  201, listado, borrado (404 `unknown_rule` / `invalid_rule_id`);
  negativos: sin token -> 401, duplicado -> 400 `rule_already_exists`,
  op invalido/entity vacia/threshold no finito/for_secs negativo -> 400
  con sus codigos. Regla `ge 0.8` con muestra 0.95 -> el evaluador
  loguea la condicion sostenida; con for_secs=120 -> `meets_for=false`
  (caso PENDING); con valor 0.2 -> sin disparo.
- Bloque 5.2: ciclo completo en vivo con `OBS_ALERT_EVAL_INTERVAL_SECS=3`
  y `OBS_ALERT_RESOLVE_GRACE_SECS=15`:
  - Muestras con tramo de 60s -> `cpu-alta` (for 30) directo a FIRING y
    `cpu-lenta` (for 300) a PENDING, `since` = inicio del tramo,
    `resolve_from` NULL.
  - Valor bajo inyectado -> `cpu-lenta` (PENDING) se resuelve al
    instante; `cpu-alta` marca `resolve_from` y sigue FIRING durante la
    ventana de hysteresis, y solo -> RESOLVED al vencerla (`alerts`
    queda vacia). Logs: `alerta resuelta`,
    `alerta entrando en ventana de resolucion (hysteresis)`.
  - Camino PENDING->FIRING en vivo (regla `temp-alta` for 20 con
    metrica limpia): sample unico -> PENDING; segundo sample que
    extiende el tramo a 50s -> `alerta firing (pending -> firing)`.
  - Deduplicacion por (rule_id, agent_id): una fila por regla+agent en
    todo el recorrido.

**Validado en este entorno (bloque 5.3):**

- Mismo ambiente (interval 3s, grace 15s), reglas recreadas tras el
  recorrido completo de 5.1/5.2:
  - Cambio de hook de API: `GET /api/v1/alerts` y
    `GET /api/v1/alerts/history` sobre el ciclo en vivo. Activas con
    contexto de regla y `since`/`resolve_from` correctos
    (`cpu-alta` FIRING directo con tramo 40s > for 30; `cpu-lenta`
    PENDING con tramo 40s < for 300).
  - Idempotencia: tras 3 ciclos de Stay*, `alert_events` sigue en 2
    (los Stay* / StartResolving no emiten eventos).
  - Filtros: `?state=firing` -> 1, `?state=pending` -> 1,
    `?agent_id=<uuid>` -> 2; `state=resolved` -> 400
    `invalid_alert_state`; `history?rule_id=4` y `?from`/`?to`/
    `?limit=2` ok; `from>to` -> 400; sin token -> 401.
  - PENDING->FIRING en vivo con `temp-alta` (for 20) sobre metrica
    limpia: sample aislado -> `NULL->pending`; tramo extendido ->
    `pending->firing`; ambos como eventos de `alert_events`.
  - Hysteresis en el historial: valor bueno -> `cpu-lenta` resuelve al
    instante (`pending->resolved`) mientras `cpu-alta` marca
    `resolve_from` y sigue FIRING, y solo -> `firing->resolved` tras
    vencer la gracia. `temp-alta` idem.
  - Timeline final: **7 eventos** (3 creaciones, 1 promocion, 3
    resoluciones), uno por transicion real y sin duplicados (por regla:
    2/2/3 eventos, todos `DISTINCT (from_state, to_state)`); `alerts`
    vacia al final.
  - 87 tests en verde, build/clippy/fmt limpios.

## Fase 6 — detalle

**Subdivision en 3 bloques:**

- **Bloque 6.1 — Health checks HTTP/TCP (entregado):**
  - Migracion `0006_health_checks.sql`: tablas `health_checks` (`name`
    UNIQUE, `kind` `http|tcp` con CHECK, `target`, `interval_secs` >= 1,
    `timeout_secs` >= 1, `enabled`), `health_check_results` (una fila por
    corrida: `ts`, `ok`, `latency_ms`, `detail`, FK CASCADE, indice
    `(check_id, ts)` desc) y `health_check_states` (estado actual por PK
    `check_id`: `state` `up|down`, `since`, `last_checked_at`,
    `last_ok`, `last_latency_ms`, `last_detail`).
  - `src/health.rs`:
    - `parse_target` — valida el target segun `kind` y devuelve
      `ParsedTarget` (diagrama host/puerto/ruta): http
      `http://host[:puerto]/ruta` (el esquema `https://` se rechaza por
      ahora — TLS llega en la Fase 8 —, puerto 0 o > 65535 invalido),
      tcp `host:puerto` (default port 0 invalido).
    - `probe` — HTTP: GET minimal sobre `tokio::net::TcpStream` (sin
      dependencias nuevas): request `GET <path> HTTP/1.1` +
      `Host` y `Connection: close`, el check es ok si la respuesta es
      2xx/3xx; TCP: exito de conexion = ok. Timeout global
      (`timeout_secs`, default `OBS_HEALTH_DEFAULT_TIMEOUT_SECS`).
    - `next_state` — maquina de estados pura `up|down`: conserva `since`
      mientras no cambia y la renueva con `ts` en la transicion.
    - `spawn_health_runner`/`run_cycle` — task de tokio que cada
      `OBS_HEALTH_POLL_SECS` (default 1s) corre los checks habilitados
      y vencidos (ultima corrida + `interval_secs` <= ahora; checks sin
      corridas corren al arrancar). Cada corrida persiste en una
      transaccion: INSERT del resultado + UPSERT del estado.
  - `src/db.rs`: `list_enabled_checks`, `list_health_checks` (LEFT JOIN
    con estados -> vista con temporales `null` si nunca corrio),
    `get_health_check`, `create_health_check`, `delete_health_check`
    (CASCADE), `get_check_state`, `latest_check_times`,
    `apply_check_outcome` (transaccion), `list_health_results`.
  - API (mismo bearer token):
    - `POST /api/v1/health/checks` — `{name, kind, target,
      interval_secs, timeout_secs?, enabled?}`; 201 con el check;
      errores 400 con codigos `check_already_exists` (UNIQUE),
      `invalid_check_kind`, `invalid_check_interval` (1..=86400),
      `invalid_check_timeout` (1..=300), `invalid_check_target`;
      `timeout_secs` default desde `OBS_HEALTH_DEFAULT_TIMEOUT_SECS`.
    - `GET /api/v1/health/checks` — lista con estado derivado
      (`state`/`since`/`last_*`).
    - `DELETE /api/v1/health/checks/{check_id}` — 404 `unknown_check`,
      400 `invalid_check_id`.
    - `GET /api/v1/health/checks/{check_id}/results` — historial desc
      `{ts, ok, latency_ms, detail}`; `limit` 1..=1000
      (`DEFAULT_HEALTH_RESULTS_LIMIT=50`, `MAX_HEALTH_RESULTS_LIMIT=1000`);
      404 `unknown_check` si no existe.
  - `src/config.rs`: `OBS_HEALTH_POLL_SECS` (>= 1) y
    `OBS_HEALTH_DEFAULT_TIMEOUT_SECS` (1..=300), validados al arrancar
    (fail-fast, filosofia ADR-0002/0003); constantes de limites de
    historial. `src/query.rs`: `CheckResultsQuery` + `into_limit`.
  - Tests unitarios: parseo de targets, armado del request y parseo de
    status line HTTP, maquina up/down, validacion de drafts, config y
    query (22 nuevos; **109 en total**).
  - Checks con `enabled = false` no se corren y quedan sin estado
    (`state: null`). Los checks sin corridas todavia corren al tercer
    `interval_secs` desde su creacion (arranque inmediato).

**Validado en este entorno (bloque 6.1):**

- Postgres real + collector con `OBS_HEALTH_POLL_SECS=1` y target HTTP
  `python3 -m http.server` local (polo 8091):
  - Creacion 201 (http 200, http 404, tcp a puerto abierto 55432, tcp a
    puerto cerrado, `http-root` contra `/`). `tcp-disabled` con
    `enabled=false` -> sin estado (`state: null`) y sin corridas.
  - Estados: `http-root` (200) -> up, `http-up` (404) -> down con
    `detail: HTTP 404`, `pg-port` (55432) -> up "conectado",
    `http-refused`/`tcp-closed` -> down "Connection refused".
    `since` = hora de la transicion; `last_ok`/`last_latency_ms`/
    `last_detail` poblados.
  - Transiciones en vivo: apagando el servidor `http-root` -> down
    (refused), restaurandolo -> up, con `since` renovado en cada flip.
  - Historial: `?limit=4` devuelve las ultimas corridas desc (mixed
    ok/down con sus detalles); `?limit=9999` -> 400 `invalid_limit`.
  - Negativos: duplicado -> 400 `check_already_exists`; kind gopher ->
    400 `invalid_check_kind`; `https://` -> 400 `invalid_check_target`
    (menciona la Fase 8); interval 0 / 86401 -> 400
    `invalid_check_interval`; timeout 0 -> 400 `invalid_check_timeout`;
    delete/list de id 999 -> 404 `unknown_check`; `check_id=abc` -> 400
    `invalid_check_id`; sin token -> 401; payload sin `name`/campo ->
    400 `invalid_check` (missing field).
  - Delete con CASCADE: borrado el check 2, sus resultados y estado
    desaparecen del listado.
  - 109 tests en verde, build/clippy/fmt limpios.

- **Bloque 6.2 — WebSocket de eventos realtime (entregado):**
  - `src/events.rs`: `Event` enum serializado con `type` como tag
    (`alert_event` | `health_result`) + `EventBus` sobre
    `tokio::sync::broadcast<String>` (capacidad `OBS_WS_CHANNEL_CAPACITY`,
    default 256). Reenviar `String` evita re-serializar y mantiene los
    eventos consistentes con lo que se persisten en DB.
  - `src/health.rs` / `src/alerts.rs`: los runners reciben el `EventBus`;
    publican el evento solo despues de commitear en DB (misma atomicidad
    que antes). Alertas: eventos en cada transicion
    INACTIVE/PENDING->PENDING, PENDING->FIRING, FIRING->PENDING
    (reingreso), FIRING->RESOLVED. Health: una `health_result` por corrida
    (incluye `state`, `since`, `state_changed`). Los eventos no se
    persisten: el historial sigue en la REST API.
  - `src/routes.rs`: `GET /api/v1/events` con upgrade a WebSocket. Auth
    reutiliza el bearer token; ademas del header `Authorization` se acepta
    `?token=...` (un WebSocket de navegador no permite headers) y se
    valida antes del upgrade -> 401 sin auth. `handle_socket`: split
    sink/stream, responde pings, ignora textos del cliente, y ante
    `RecvError::Lagged` emite un aviso `{"type":"events_lagged",
    "dropped":n}` antes de seguir.
  - `src/auth.rs`: `check_bearer_str` comparte la comparacion constante de
    tiempo con `check_bearer`.
  - `src/config.rs`: `OBS_WS_CHANNEL_CAPACITY` (>= 1), validado al
    arrancar.
  - Tests: 11 nuevos (Event serialization, EventBus subscribe/lagged/capped
    capacity, `events_lagged` payload, auth str, ws capacity config).
    **120 en total**.

**Validado en este entorno (bloque 6.2):**

- Postgres real + collector de la validacion 6.1 + `http.server` (8091):
  - Cliente WS autenticado recibe `health_result` por corrida de cada
    check (~cada 3-4s), con `state`/`since`/`state_changed`, tanto de HTTP
    como TCP.
  - `curl` con handshake WS sin token -> 401; con token invalido -> 401;
    con token valido -> 101 (upgrade).
  - Reglas de alerta disparando desde samples inyectados: el `alert_event`
    (firing->resolved) llega al WS en vivo al arrancar el cliente.
  - Happy path WS: cliente conectado desde antes de la transicion recibe
    los eventos de alerta en orden junto con los health_result del mismo
    intervalo.
  - 120 tests en verde, build/clippy/fmt limpios.

- **Bloque 6.3 — Historial unificado + summary de salud + eventos de
  conectividad (entregado):**
  - Migracion `0007_connectivity_events.sql`: columna
    `agents.last_connectivity_state` (ultimo estado derivado observado
    por el runner; NULL hasta la primera pasada) y tabla
    `connectivity_events` (`id`, `agent_id` FK CASCADE, `from_state`
    nullable, `to_state` CHECK `online|degraded|offline`, `ts` default
    now(), indice `(agent_id, ts DESC)`).
  - `src/connectivity.rs`:
    - `detect_transitions` (pura): para cada agente compara el estado
      derivado (bloque 4.2, misma funcion `connectivity_state`) contra
      `last_connectivity_state`; si difiere -> `ConnectivityTransition`
      (from/to). La primera observacion (columna NULL) cuenta como
      transicion desde NULL, igual que la creacion de alerta en
      `alert_events`.
    - `spawn_connectivity_runner`/`run_cycle`: task de tokio que cada
      `OBS_CONNECTIVITY_POLL_SECS` (default 5) recorre los agents,
      detecta transiciones y las persiste y publica al bus. Sin cambios
      no escribe nada. `ts` del evento = hora del ciclo que lo detecto
      (filosofia last_seen, ADR-0003).
    - `apply_connectivity_transitions` (db.rs): en una transaccion
      INSERT del evento + UPDATE de `agents.last_connectivity_state`;
      los `connectivity_event` al WebSocket se publican solo despues del
      commit (misma atomicidad que alertas/health).
  - Historial unificado `GET /api/v1/events/history` (query API, mismo
    bearer token):
    - `src/query.rs`: `TimelineQuery` (`agent_id` + `limit`, default 50,
      max 1000 con `DEFAULT_EVENTS_HISTORY_LIMIT`/
      `MAX_EVENTS_HISTORY_LIMIT`) y `merge_timeline` (pura): concatena
      las cuatro fuentes, ordena por `ts` desc y corta a `limit`.
    - `src/db.rs`: `list_recent_health_results` (JOIN con checks para
      `check_name`), `list_recent_reboots` (con `agent_id`),
      `list_connectivity_events` (`agent_id` opcional por parametro
      `($1::uuid IS NULL OR agent_id = $1)`).
    - El handler arma `TimelineEntry {kind, ts, payload}` por fuente
      (`alert_event`, `health_result`, `reboot_event`,
      `connectivity_event`) con el mismo shape que el WebSocket y
      devuelve `{events, count}`. `agent_id` filtra alertas, reboots y
      conectividad; salud se trae acotada por `limit`.
  - Summary de salud `GET /api/v1/health/summary`:
    - `db.rs`: `list_agents_liveness` (last_seen), `count_check_states`
      (estados de checks), `count_alert_states` (pending/firing),
      `count_health_checks` (total).
    - El handler deriva la conectividad de los agents con la misma
      funcion del query API y agrega `{agents: {total, online, degraded,
      offline}, checks: {total, up, down, unknown}, alerts: {total,
      pending, firing}}`. `unknown` = checks definidos sin estado aun
      (`health_check_states` solo guarda up/down).
  - `src/events.rs`: variante `ConnectivityEvent` (tag
    `connectivity_event`) + constructor `Event::connectivity` + tests de
    serializacion (from NULL en la primera observacion).
  - `src/config.rs`: `OBS_CONNECTIVITY_POLL_SECS` (>= 1), fail-fast al
    arrancar; `state_limits()` movido aca y publico (lo comparten
    handlers de query y el runner). `src/state.rs`:
    `connectivity_state_from_str` y `ConnectivityState::as_str`.
  - Tests: 20 nuevos (connectivity transition detection, state parse,
    timeline query/merge, config poll validation, event serialization).
    **140 en total**.

**Validado en este entorno (bloque 6.3):**

- Postgres real + collector con `OBS_CONNECTIVITY_POLL_SECS=2`,
  `OBS_STATE_ONLINE_SECS=5` y `OBS_STATE_DEGRADED_SECS=15`:
  - Migracion 0007 aplicada en vivo (`_sqlx_migrations` version 7).
  - El agente stale existente (sin `last_connectivity_state`) -> primera
    observacion `NULL -> offline` en `connectivity_events` y por WS.
  - Cadena completa de transiciones en DB y en el cliente WebSocket:
    `offline -> online` (7s), `online -> degraded` (+6s) y
    `degraded -> offline` (+10s) con los umbrales cortos, en tiempo real
    y en orden; luego `offline -> online -> degraded` tomada viva tras un
    heartbeat.
  - `GET /api/v1/events/history` (sin filtro): timeline cruzando las
    fuentes por `ts` desc (health_result, connectivity_event y
    alert_event con `rule_name`/`from_state`/`to_state`); `?limit=1000`
    -> 434 eventos (424 health + 6 alert + 4 connectivity).
  - `GET /api/v1/events/history?agent_id=...&limit=2` filtra y acota.
  - `GET /api/v1/health/summary`: `{agents:{total,online,degraded,
    offline}, checks:{total,up,down,unknown}, alerts:{total,pending,
    firing}}` consistente con el estado real (1 agente offline tras
    vencer last_seen; http-root down / pg-port up; 0 alertas activas).
    Tras un heartbeat el agente pasa a un estado reciente reflejado en la
    agregacion.
  - Negativos: endpoints sin token -> 401; `limit=0` -> 400
    `invalid_limit`; `agent_id=nope` -> 400 `invalid_agent_id`.
  - 140 tests en verde, build/clippy/fmt limpios.

## Fase 7 — detalle

Dashboard web servido por el propio collector: **HTML+CSS+JS vanilla**
(una pagina + un CSS + un JS), sin framework, sin build step y sin
dependencias JS — el unico cambio de backend del bloque 7.1 es servir un
directorio estatico (`tower-http::services::ServeDir` + `fallback_service`
con SPA fallback a `index.html`). Consume la REST API y el WebSocket ya
existentes (Fases 4-6) con el mismo bearer token.

**Auth:** pantalla de login que pide el `OBS_COLLECTOR_TOKEN`; se guarda
en `sessionStorage` (`obs_token`) y se manda como `Authorization: Bearer`
en cada `fetch`; el WS lo pasa como `?token=`. 401 en cualquier request
devuelve al login.

**Bloques:**

- **Bloque 7.1 — Overview + skeleton (entregado):**
  - Backend: `OBS_DASHBOARD_DIR` (default `dashboard`, carpeta relativa al
    binario) montada con `ServeDir` con fallback SPA a `index.html`;
    `build_router` agrega `.fallback_service(...)` (las rutas `/api/*` y
    `/healthz` explicitas siguen ganando). `tower-http` gana la feature
    `fs`.
  - `collector/dashboard/index.html` + `app.js` + `style.css`:
    - Login (token) -> layout con sidebar (Overview / Host / Alertas e
      historicos — vistas 7.2 y 7.3 placeholder con "proximamente").
    - Overview: summary cards (`GET /api/v1/health/summary`): agents
      online/degraded/offline, checks up/down/unknown, alertas
      pending/firing; lista de agents con badge de estado y tiempos
      relativos (`GET /api/v1/agents`); timeline unificado
      (`GET /api/v1/events/history` limit 50) renderizado por `type`, con
      append en vivo de los eventos del WS (`/api/v1/events?token=`),
      deduplicacion por orden y refresco (debounced) del summary ante
      cualquier evento.
    - Helpers de tiempo relativo y formato de fechas; los eventos
      `events_lagged` del WS muestran un aviso en la cola del timeline.
  - Tests: config `OBS_DASHBOARD_DIR`, y un test que verifica que
    `dashboard/index.html` existe junto al manifest (evita commitear el
    backend sin el frontend). **143 en total** (3 nuevos).
  - Validado en vivo: collector sirviendo las 3 hojas + fallback SPA,
    login con token correcto/incorrecto (401 -> login), cards con datos
    reales (1 agente, http-root down / pg-port up), timeline en vivo con
    eventos de `connectivity_event`/`health_result`/`alert_event`
    agregandose al abrir (a traves del WS estando el overview montado).

- **Bloque 7.2 — Host page (pendiente):**
  Detalle por agent (ruta `/host.html?agent=...`): estado + `since`,
  metrica y ultimo valor de cada serie (`GET /api/v1/agents/{id}/metrics`),
  grafica simple de una serie elegida (`.../metrics/{metric}?limit=...`),
  timeline de conectividad (`/api/v1/events/history?agent_id=...`),
  reboots (`/api/v1/agents/{id}/reboots`) y alertas activas del host
  (`/api/v1/alerts?agent_id=...`).

- **Bloque 7.3 — Alertas e historicos (pendiente):**
  Vistas completas de gestion y lectura: rules (list/create/delete),
  checks (list/create/delete/results), alertas activas e historial de
  alertas, historial unificado completo con filtros, y check results.
  Formularios con validacion contra los codigos de error de la API.
