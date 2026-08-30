# Observatory

Plataforma de observabilidad distribuida para hosts Linux. Un agente en C
recolecta metricas de bajo nivel directamente desde las interfaces del
kernel; un collector central en Rust las recibe, persiste y expone via
API REST, WebSocket y un dashboard web integrado.

```
Agent (C) ---HTTP/JSON---> Collector (Rust) ---> PostgreSQL
                                  |
                        REST + WebSocket ---> Dashboard
```

El proyecto no compite con Prometheus ni con OpenTelemetry. El objetivo
es construir desde cero una plataforma funcional para entender los
problemas reales del software distribuido que observa maquinas: protocol
design, liveness, alert state machines, cardinality control, transport
reliability. La hipotesis verificable: un agente Linux con un presupuesto
de recursos explicito puede correr en hardware muy limitado (Pentium M,
256 MB RAM, Alpine Linux) con overhead medible y documentado.

## Estructura

```
observatory/
├── agent/           Agente en C (ver agent/README.md)
├── collector/       Collector en Rust, migraciones SQL y dashboard
│   ├── src/
│   ├── migrations/
│   └── dashboard/   HTML/CSS/JS estatico servido por el collector
├── deploy/          docker-compose y configuracion de ejemplo
├── benchmarks/      Script de benchmark reproducible y resultados
├── docs/
│   └── adr/         Architecture Decision Records
├── PHASES.md        Historial detallado de desarrollo por fase
└── README.md
```

## Funcionalidades

**Agente**

Recoleccion sin herramientas externas (sin `top`, `free`, `df`, `ps`):
lectura directa de `/proc/stat`, `/proc/meminfo`, `/proc/diskstats`,
`/proc/net/dev`, `/proc/mounts` + `statvfs()`, `/proc/uptime`,
`/proc/<pid>/stat` y `/sys/class/thermal/`. Metricas basadas en deltas
con reloj monotonico. Heartbeat independiente de las metricas con
backoff exponencial por canal ante fallos de envio. Identidad persistente
generada desde `/dev/urandom`, no dependiente del hostname.

**Collector**

Ingestion con validacion estricta: version de protocolo, UUID del agente,
ventana temporal, cardinalidad de arrays y de claves de metricas. Registro
implicito de agentes: el primer payload valido crea la fila. Deteccion de
reboots comparando `system.uptime` entre muestras consecutivas dentro de
una transaccion serializada. Maquina de estados de conectividad
(ONLINE/DEGRADED/OFFLINE) derivada de `last_seen`, con historial de
transiciones.

**Alert engine**

Reglas declarativas (metric, entidad opcional, operador, umbral, duracion).
Maquina de estados INACTIVE -> PENDING -> FIRING -> RESOLVED. Hysteresis
configurable para evitar flapping. Deduplicacion por (regla, agente).
Historial de transiciones.

**Health checks**

Sondas HTTP y TCP configurables con intervalo y timeout propios. Estado
actual (up/down) derivado de la ultima corrida. Historial de resultados.

**Eventos en tiempo real**

WebSocket con autenticacion por token (header o query param para clientes
de navegador). Canal de broadcast con notificacion ante lag. Historial
unificado de alertas, health checks, reboots y eventos de conectividad
accesible por REST.

**Dashboard**

HTML/CSS/JS sin framework ni build step, servido por el propio collector.
Overview con tarjetas de estado, lista de agentes y timeline en vivo.
Pagina de host con series, grafica SVG, alertas activas y reboots.
Gestion de reglas y checks, historial de alertas, timeline filtrable.

**Hardening**

Rate limiting por IP (token bucket) sobre los endpoints de ingestion, con
respuesta 429 y cabecera Retry-After. TLS nativo con rustls: el collector
sirve HTTPS cuando se le proveen certificado y clave. El agente no
implementa TLS; los agentes remotos llegan via tunel dentro de la red
confiable. Fuzzing deterministico del agente (transport, config parser)
y del pipeline de ingestion del collector.

## Inicio rapido

```sh
# Configurar el entorno
cp deploy/.env.example deploy/.env
# Editar deploy/.env: establecer OBS_COLLECTOR_TOKEN

# Levantar PostgreSQL y el collector
docker compose -f deploy/docker-compose.yml up -d

# Compilar el agente
cd agent && make

# Configurar el agente
cat > /etc/observer/agent.conf << 'CONF'
collector_url = http://collector.local:8080
agent_token = <el valor de OBS_COLLECTOR_TOKEN>
agent_id_path = /etc/observer/agent-id
metrics_interval_secs = 10
heartbeat_interval_secs = 5
CONF

./observer-agent /etc/observer/agent.conf
```

El dashboard queda disponible en `http://collector.local:8080`. El token
configurado en `OBS_COLLECTOR_TOKEN` es la contrasena del login.

## Configuracion del collector

Todas las variables tienen valores por defecto que permiten correr en
desarrollo sin configuracion adicional, excepto `DATABASE_URL` y
`OBS_COLLECTOR_TOKEN`, que son obligatorias.

| Variable | Default | Descripcion |
|---|---|---|
| `DATABASE_URL` | (requerida) | URL de conexion a PostgreSQL |
| `OBS_COLLECTOR_TOKEN` | (requerida) | Token bearer compartido |
| `OBS_LISTEN_ADDR` | `0.0.0.0:8080` | Direccion de escucha |
| `OBS_TLS_CERT` | (vacio) | Ruta al certificado PEM para HTTPS |
| `OBS_TLS_KEY` | (vacio) | Ruta a la clave privada PEM para HTTPS |
| `OBS_STATE_ONLINE_SECS` | `15` | Segundos desde ultimo heartbeat para ONLINE |
| `OBS_STATE_DEGRADED_SECS` | `60` | Segundos para DEGRADED |
| `OBS_ALERT_EVAL_INTERVAL_SECS` | `15` | Frecuencia del evaluador de alertas |
| `OBS_ALERT_LOOKBACK_SECS` | `300` | Ventana de evaluacion en segundos |
| `OBS_ALERT_RESOLVE_GRACE_SECS` | `60` | Hysteresis de resolucion |
| `OBS_RATE_LIMIT_ENABLED` | `true` | Activar rate limiting por IP |
| `OBS_RATE_LIMIT_RATE` | `20.0` | Tasa sostenida (req/s) |
| `OBS_RATE_LIMIT_BURST` | `50.0` | Rafaga maxima |
| `OBS_DASHBOARD_DIR` | `dashboard` | Directorio de la UI estatica |

Ver `deploy/.env.example` para la lista completa.

## Decisiones de arquitectura

Los ADRs en `docs/adr/` documentan las decisiones tecnicas relevantes:

- `0001-agent-language.md` — C para el agente: acceso directo a POSIX/Linux,
  overhead minimo, sin runtime.
- `0002-transport-protocol.md` — HTTP plano en el agente, TLS terminado en
  el collector. Rechazo explicito de `https://` en el agente.
- `0003-collector-ingestion.md` — Modelo de datos, registro implicito de
  agentes, semantica de `last_seen` y deteccion de reboots.

## Benchmarks

`benchmarks/run_benchmark.sh` levanta una instancia efimera de PostgreSQL
y el collector en modo release, corre N agentes simulados durante 30
segundos y reporta throughput, latencia (p50/p95/p99) y distribucion de
codigos HTTP, junto con el fingerprint del entorno (kernel, CPU, memoria,
toolchain, commit).

Referencia en la maquina de desarrollo (30s, 10 agentes):
19.3 req/s, p99 9.1ms, p50 5.2ms, 100% HTTP 200.

Correr el script en el hardware objetivo (Pentium M + Alpine Linux)
produce la referencia experimental documentada en `benchmarks/results/`.
