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
- [ ] **Fase 3 — Collector core (Rust)**
      Proyecto Axum, migraciones PostgreSQL (`agents`, `metric_samples`),
      registro de agentes, ingestion con validación, autenticación por
      bearer token.
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
