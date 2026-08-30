# Historial de desarrollo

Observatory se construyo en fases progresivas, cada una con commits
independientes. Este documento registra que se entrego en cada fase,
que se valido y que quedo deliberadamente fuera de scope.

## Fase 1 — Agente: nucleo en C

Estructura del proyecto, Makefile con targets `all / debug / sanitize /
test / fuzz`, collectors de CPU (`/proc/stat`, calculo por delta entre
snapshots) y memoria (`/proc/meminfo`, prioridad a `MemAvailable`),
parser de configuracion sin dependencias externas, logger con niveles,
serializador JSON manual del payload, 3 suites de tests unitarios.

Validado: build limpio bajo `-Wall -Wextra -Wpedantic -Wconversion
-Wshadow`, 3/3 suites en verde, prueba con AddressSanitizer y
UndefinedBehaviorSanitizer contra `/proc` real.

## Fase 2 — Agente: collectors restantes y transporte

Collectors de disco (`/proc/diskstats`), red (`/proc/net/dev`),
filesystem (`/proc/mounts` + `statvfs()`), uptime (`/proc/uptime`),
procesos (conteo agregado por estado, sin cardinalidad por PID) y
temperatura (`/sys/class/thermal/`, opcional). Cliente HTTP sobre
sockets POSIX sin dependencias externas, con timeouts explicitos de
connect/write/read. Backoff exponencial con jitter por canal de envio.
Identidad persistente generada desde `/dev/urandom` con permisos 0600
y manejo de condicion de carrera entre instancias via `O_EXCL`. 8 suites
de tests nuevas; 11/11 en total.

Validado: build limpio, 11/11 suites, prueba de integracion contra un
mock collector HTTP con AddressSanitizer activo confirmando el flujo
completo de envio y el backoff real ante fallos de conexion.

## Fase 3 — Collector: ingestion y autenticacion

Proyecto Rust (Axum, SQLx, Tokio). Migraciones embebidas aplicadas al
arrancar. Endpoints `POST /api/v1/metrics` y `POST /api/v1/agents/heartbeat`.
Autenticacion por bearer token con comparacion en tiempo constante.
Registro implicito de agentes. Validacion de ingestion: version de
protocolo, UUID del agente, ventana temporal, cardinalidad de arrays y
claves de metricas. Deteccion de reboots por caida de `system.uptime`
en una transaccion serializada. 17 tests unitarios.

Validado: `cargo build` y `cargo test` limpios, prueba de integracion
con PostgreSQL real y agente real corriendo 8 segundos.

## Fase 4 — Collector: query API y estado de conectividad

Endpoints GET de la API de lectura: lista de agentes, detalle, series
disponibles, serie temporal con filtros (`entity`, `from`, `to`, `limit`),
reboots. Estado ONLINE/DEGRADED/OFFLINE derivado de `last_seen` como
funcion pura; umbrales configurables y validados al arrancar. Serializado
en la respuesta como `state` y `last_seen_age_secs`. 23 tests nuevos;
48 en total.

Validado: build limpio, 48/48 tests, prueba de integracion verificando
estados derivados con valores reales de `last_seen`, casos negativos de
la API (401, 400, 404).

## Fase 5 — Alert engine

Reglas declarativas con operadores `ge/gt/le/lt`. Funcion pura de
evaluacion de series (`evaluate_series`) que determina si la condicion
se sostiene y desde cuando. Maquina de estados INACTIVE -> PENDING ->
FIRING -> RESOLVED con hysteresis configurable. Deduplicacion por
(regla, agente) via clave primaria de la tabla `alerts`. Historial de
transiciones en `alert_events`. API de gestion y consulta de reglas y
alertas activas. 39 tests nuevos; 87 en total.

Validado: build limpio, 87/87 tests, ciclo completo en vivo con los
caminos PENDING->FIRING, hysteresis (StartResolving->StayResolving->
Resolved) y resolucion inmediata de alertas PENDING verificados en DB.

## Fase 6 — Health checks, WebSocket e historial unificado

Sondas HTTP y TCP con intervalo y timeout propios. Estado `up/down`
derivado de la ultima corrida; historial de resultados. Canal de
broadcast sobre `tokio::sync::broadcast` para el WebSocket de eventos
en tiempo real. Autenticacion del WebSocket por cabecera o query param.
Notificacion de lag (`events_lagged`) a suscriptores lentos. Runner de
eventos de conectividad que materializa el historial de transiciones
ONLINE/DEGRADED/OFFLINE. Timeline unificado de las cuatro fuentes de
eventos. Endpoint de summary de salud. 53 tests nuevos; 140 en total.

Validado: build limpio, 140/140 tests, prueba de integracion con
transiciones de conectividad en vivo, cliente WebSocket recibiendo
`health_result` y `alert_event` en tiempo real, timeline unificado con
434 eventos reales.

## Fase 7 — Dashboard

UI estatica (HTML/CSS/JS sin framework ni build step) servida por el
propio collector via `tower_http::services::ServeDir` con SPA fallback.
Overview con tarjetas de estado (agentes, checks, alertas), lista de
agentes con badge de conectividad y timeline en vivo via WebSocket.
Pagina de host con detalle del agente, tabla de series con ultimo valor,
grafica SVG generada sin dependencias, timeline del host y reboots.
Vista de gestion: reglas, checks, alertas activas, historial de alertas
e historial unificado con filtros. Token guardado en `sessionStorage`;
401 en cualquier endpoint devuelve al login. 3 tests nuevos (existencia
del bundle en disco); 143 en total.

Validado: build limpio, 143/143 tests y `node --check` sobre los tres
archivos JS, flujo completo de login, navegacion y actualizacion en vivo
verificados en navegador.

## Fase 8 — Hardening

**Rate limiting**: token bucket por IP sobre los endpoints de ingestion.
Middleware anclado con `route_layer` exclusivamente sobre `POST /api/v1/metrics`
y `POST /api/v1/agents/heartbeat`. Respuesta 429 con cabecera `Retry-After`.
Configurable o desactivable via variables de entorno; validacion al arrancar.

**TLS**: el collector puede servir HTTPS nativo (rustls, provider aws-lc-rs)
cuando se le proveen `OBS_TLS_CERT` y `OBS_TLS_KEY`. Sin esas variables
sirve HTTP plano. Shutdown graceful compartido entre ambos modos. El agente
no implementa TLS; ver ADR-0002.

**Fuzzing y benchmark**: `make sanitize` en el agente compila el binario,
los 11 tests y el harness de fuzzing bajo AddressSanitizer, UndefinedBehaviorSanitizer
y LeakSanitizer, y ejecuta 200.000 iteraciones sobre los parsers de
transport y config. El collector incluye un test de fuzzing deterministico
de 50.000 iteraciones sobre el pipeline completo de ingestion. El script
`benchmarks/run_benchmark.sh` levanta una instancia efimera de PostgreSQL
y el collector en release, corre N agentes simulados y reporta throughput,
latencias y fingerprint del entorno. 18 tests nuevos; 158 en total.

Validado: build limpio, 158/158 tests, rate limiting verificado en vivo
(200/200/429 con burst=2), TLS verificado con certificado autofirmado
(`curl --cacert`), fuzzing sin hallazgos de sanitizers.
