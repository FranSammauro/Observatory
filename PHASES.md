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
- [ ] **Fase 4 — Collector: query API + estados de conectividad**
      Endpoints GET, máquina de estados ONLINE/DEGRADED/OFFLINE, detección
      de reboot.
- [ ] **Fase 5 — Alert engine**
      Reglas declarativas, máquina de estados
      INACTIVE→PENDING→FIRING→RESOLVED, hysteresis, deduplicación,
      historial.
- [ ] **Fase 6 — Health checks + WebSocket**
      Checks HTTP/TCP, eventos realtime.
- [ ] **Fase 7 — Dashboard**
      Overview, host page, alertas, históricos.
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

- Query API (endpoints GET de series) y maquina de estados
  ONLINE/DEGRADED/OFFLINE (Fase 4 — ver ADR-0003, la DB ya mantiene
  last_seen para derivarla).
- Deteccion de reboot al comparar uptime entre muestras (Fase 4).
- Tokens per-agent y TLS (Fase 8).
