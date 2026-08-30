# collector

Servicio HTTP en Rust que recibe telemetria de los agentes, la persiste
en PostgreSQL y expone una API REST, un canal WebSocket de eventos en
tiempo real y un dashboard web estatico.

## Stack

- Axum 0.8 — framework HTTP
- SQLx 0.7 — acceso a PostgreSQL con migraciones embebidas
- Tokio — runtime async
- rustls (via axum-server) — TLS nativo opcional
- tower-http — limite de body y trazado de requests

## Setup local

```sh
# Con docker-compose (desde la raiz del repo)
cp deploy/.env.example deploy/.env
# Editar OBS_COLLECTOR_TOKEN
docker compose -f deploy/docker-compose.yml up -d

# O con PostgreSQL local ya corriendo
export DATABASE_URL=postgres://observer:observer@localhost/observer
export OBS_COLLECTOR_TOKEN=mi-token-secreto
cargo build --release
./target/release/collector
```

Las migraciones se aplican automaticamente al arrancar. No se requiere
`sqlx-cli` para el despliegue.

## Endpoints

### Ingestion (agentes)

| Metodo | Path | Descripcion |
|---|---|---|
| `POST` | `/api/v1/metrics` | Sample completo de metricas |
| `POST` | `/api/v1/agents/heartbeat` | Heartbeat de liveness |

Ambos requieren `Authorization: Bearer <token>`. El agente se registra
implicitamente al primer payload valido.

### Query API

| Metodo | Path | Descripcion |
|---|---|---|
| `GET` | `/api/v1/agents` | Lista de agentes con estado derivado |
| `GET` | `/api/v1/agents/{id}` | Detalle de un agente |
| `GET` | `/api/v1/agents/{id}/metrics` | Series disponibles del agente |
| `GET` | `/api/v1/agents/{id}/metrics/{metric}` | Serie temporal con filtros opcionales |
| `GET` | `/api/v1/agents/{id}/reboots` | Timeline de reboots detectados |

### Alertas

| Metodo | Path | Descripcion |
|---|---|---|
| `POST` | `/api/v1/alerts/rules` | Crear regla de alerta |
| `GET` | `/api/v1/alerts/rules` | Listar reglas |
| `DELETE` | `/api/v1/alerts/rules/{id}` | Borrar regla |
| `GET` | `/api/v1/alerts` | Alertas activas (filtro por agente y estado) |
| `GET` | `/api/v1/alerts/history` | Historial de transiciones |

### Health checks

| Metodo | Path | Descripcion |
|---|---|---|
| `POST` | `/api/v1/health/checks` | Crear check HTTP o TCP |
| `GET` | `/api/v1/health/checks` | Listar checks con estado actual |
| `DELETE` | `/api/v1/health/checks/{id}` | Borrar check |
| `GET` | `/api/v1/health/checks/{id}/results` | Historial de resultados |
| `GET` | `/api/v1/health/summary` | Estado agregado de la plataforma |

### Eventos y sistema

| Metodo | Path | Descripcion |
|---|---|---|
| `GET` | `/api/v1/events` | WebSocket de eventos en tiempo real |
| `GET` | `/api/v1/events/history` | Timeline unificado de eventos |
| `GET` | `/healthz` | Liveness y readiness |

## Autenticacion

Token bearer compartido configurado en `OBS_COLLECTOR_TOKEN`. La
comparacion se realiza en tiempo constante para evitar timing attacks.
El WebSocket acepta el token tambien como query param `?token=` porque
los clientes de navegador no pueden enviar cabeceras en el handshake.

## Alert engine

Las reglas son declarativas: `metric_name`, `entity` (opcional, para
metricas por dispositivo o interfaz), operador (`ge`, `gt`, `le`, `lt`),
umbral y duracion minima (`for_secs`).

```json
{
  "name": "cpu-alta",
  "metric_name": "system.cpu.utilization",
  "op": "ge",
  "threshold": 0.90,
  "for_secs": 300
}
```

El evaluador corre cada `OBS_ALERT_EVAL_INTERVAL_SECS` sobre una ventana
de `OBS_ALERT_LOOKBACK_SECS`. Una alerta FIRING no se resuelve
inmediatamente al caer la condicion; espera `OBS_ALERT_RESOLVE_GRACE_SECS`
para evitar flapping.

## Estructura del codigo

```
collector/src/
├── main.rs         Entry point, configuracion, TLS, graceful shutdown
├── config.rs       Variables de entorno con validacion al arrancar
├── routes.rs       Router Axum, handlers HTTP y WebSocket
├── db.rs           Capa de acceso a PostgreSQL
├── models.rs       Deserializacion y validacion de payloads
├── alerts.rs       Motor de alertas: evaluacion y maquina de estados
├── health.rs       Health checks HTTP/TCP: sondas y scheduler
├── connectivity.rs Runner de eventos de conectividad
├── events.rs       Canal de broadcast para el WebSocket
├── auth.rs         Verificacion del bearer token
├── ratelimit.rs    Token bucket por IP
├── reboot.rs       Deteccion de reboots por caida de uptime
├── state.rs        Maquina de estados ONLINE/DEGRADED/OFFLINE
├── query.rs        Parsing y validacion de parametros de consulta
├── validation.rs   Validacion de timestamps
└── error.rs        Tipo de error unificado para los handlers
```

## Tests

```sh
cargo test
```

158 tests unitarios cubriendo validacion de payloads, maquina de estados
de alertas, deteccion de reboots, parsing de parametros de consulta,
autenticacion, rate limiting, health checks, eventos de conectividad,
serializacion de eventos WebSocket y validacion de toda la configuracion.
Incluye un test de fuzzing deterministico de 50.000 iteraciones sobre el
pipeline de ingestion.
