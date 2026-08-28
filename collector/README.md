# observer-collector

Collector central en **Rust (Axum)** que recibe los payloads que emite el
agent C (`observer-agent`), los valida, los autentica por bearer token, y
los persiste en PostgreSQL.

> Estado actual: **Fase 3** — ingestion + registro implicito de agentes +
> validacion + autenticacion. El query API (GET), la maquina de estados
> ONLINE/DEGRADED/OFFLINE y la deteccion de reboot son **Fase 4** (ver
> [`../PHASES.md`](../PHASES.md)).

## Arquitectura

```
Agent (C) --POST /api/v1/metrics--> Collector (Rust/Axum) --> PostgreSQL
          --POST /api/v1/agents/heartbeat-->          ^
                          (Authorization: Bearer <token>)
```

Endpoints:

- `POST /api/v1/metrics` — sample completo de metricas del agent.
- `POST /api/v1/agents/heartbeat` — heartbeat liviano (mas frecuente).
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
cargo test             # 16 tests unitarios (sin DB)
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

## Proximos pasos (Fase 4+)

- Query API: endpoints GET de series temporales.
- Maquina de estados ONLINE/DEGRADED/OFFLINE.
- Deteccion de reboot (uptime decreciente entre muestras).

Ver [`../PHASES.md`](../PHASES.md).