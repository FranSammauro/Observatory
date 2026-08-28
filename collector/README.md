# observer-collector

Collector central en **Rust (Axum)** que recibe los payloads que emite el
agent C (`observer-agent`), los valida, los autentica por bearer token, y
los persiste en PostgreSQL.

> Estado actual: **Fase 4** (en curso) — query API de lectura (bloque 4.1)
> entregado: endpoints GET de agentes/series. Pendiente de la Fase 4: la
> maquina de estados ONLINE/DEGRADED/OFFLINE y la deteccion de reboot
> (bloques 4.2/4.3, ver [`../PHASES.md`](../PHASES.md)).

## Arquitectura

```
Agent (C) --POST /api/v1/metrics--> Collector (Rust/Axum) --> PostgreSQL
          --POST /api/v1/agents/heartbeat-->          ^
                          (Authorization: Bearer <token>)

Dashboard  --GET /api/v1/...--> Collector --> PostgreSQL
                          (Authorization: Bearer <token>)
```

Endpoints:

Ingestion (Fase 3):

- `POST /api/v1/metrics` — sample completo de metricas del agent.
- `POST /api/v1/agents/heartbeat` — heartbeat liviano (mas frecuente).

Query API (Fase 4, bloque 4.1) — de solo lectura, mismo bearer token:

- `GET /api/v1/agents` — agentes registrados (`first_seen`, `last_seen`),
  ordenados por actividad descendente.
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

Infra:

- `GET /healthz` — chequeo de salud (incluye ping a la DB).

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
| `RUST_LOG` | `info` | nivel de log (tracing) |

## Build y tests

```sh
cargo build            # debug
cargo build --release  # release (LTO + codegen-units=1)
cargo test             # 25 tests unitarios (sin DB)
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

## Limites de cardinalidad

El Collector valida del lado del servidor lo que el agent ya acota
(`OBS_MAX_DISKS/INTERFACES/FILESYSTEMS = 16`): arrays con mas de 16
entradas -> `400 too_many_entities`; el objeto `metrics` con mas de
1024 claves -> `400 too_many_metrics` (el agent emite ~14). Los valores
no finitos (NaN/Inf) no pueden llegar a la DB: `serde_json` los rechaza
a nivel de parsing (`400 invalid_json`).

## Proximos pasos (Fase 4, bloques pendientes)

- Maquina de estados ONLINE/DEGRADED/OFFLINE derivada de `last_seen`
  (bloque 4.2).
- Deteccion de reboot (uptime decreciente entre muestras) (bloque 4.3).

Ver [`../PHASES.md`](../PHASES.md).