# ADR-0003: Collector core — decisiones de ingestion (Fase 3)

## Estado

Aceptado

## Contexto

La Fase 3 construye el Collector central en Rust (Axum) que recibe los
payloads que el agent ya emite (ver ADR-0002 y `agent/src/protocol.c`):

- `POST /api/v1/metrics` — sample completo.
- `POST /api/v1/agents/heartbeat` — heartbeat liviano.

El protocolo del agent **no tiene endpoint de registro ni campo de
hostname**: cada payload solo lleva `protocol_version`, `agent_id`
(hex de 128 bits, 32 chars) y `timestamp` (segundos epoch), mas las
metricas. El Collector debe decidir, sin tocar el agent:

1. Como autenticar.
2. Como registrar/hacer el seguimiento de agentes.
3. Que rango temporal de `timestamp` aceptar.
4. Como modelar los datos en PostgreSQL (los arrays disk/network/
   filesystem tienen entidades con nombre).
5. Como tratar el heartbeat respecto a `last_seen`.

## Decisiones

### 1. Autenticacion: un unico bearer token compartido por env

V1 usa un **unico token compartido**, configurado en el Collector via la
env `OBS_COLLECTOR_TOKEN`. Cada agent envia `Authorization: Bearer
<token>` (el agent ya soporta `agent_token` en su config). El Collector:

- Requiere la env al arrancar; si falta, **falla ruidosamente** (exit 2),
  consistente con la filosofia del ADR-0002 ("fallar antes que degradar").
- Compara en tiempo casi-constante.
- No autentica a nadie sin token: si el header falta o no matchea -> 401.

Tokens por-agente o por-cliente se difieren hasta que haya un caso real
que los justifique (rotacion, multi-tenant). Para una plataforma personal
de V1 un secreto compartido es lo optimo: un solo lugar donde configurar,
y el agent ya queda definido.

### 2. Registro de agentes: implicito (upsert en el primer payload)

No hay endpoint de registro en el protocolo. Lo optimo es **registro
implicito**: el primer payload valido (heartbeat o metrics) crea la fila
en `agents` (`INSERT ... ON CONFLICT (agent_id) DO UPDATE SET
last_seen`), manteniendo `first_seen` intacto. Cero cambios en el agent,
cero flujo de registro a implementar/fragil.

Nota: los agents se identifican por un id aleatorio persistente
(`agent/identity.c`), no por hostname (informe seccion 20). El hostname
no se reporta en V1.

### 3. Validacion temporal: ventana acotada

`timestamp` se compara contra el reloj del servidor:

- Futuro tolerado: `OBS_INGEST_FUTURE_SKEW_SECS` (default **60s**) — cubre
  relojes de host ligeramente adelantados.
- Antiguedad maxima: `OBS_INGEST_MAX_AGE_SECS` (default **600s**) — el
  agent no tiene spool en V1 (informe seccion 27), asi que una muestra mas
  vieja que eso es ruido, no backlog legitimo.

Fuera de la ventana -> 400 `timestamp_out_of_range`. Acotar evita
persistir datos con relojes descompuestos y limita el apetito de
almacenamiento.

### 4. Modelo de datos: `metric_samples` normalizado nombre->valor

Se adopta el naming dotted-style del informe (probe-based, secciones 4-5
y 34). Tabla **normalizada por (agent_id, ts, metric_name, entity)**:

```
metric_samples (agent_id UUID, ts TIMESTAMPTZ, metric_name TEXT,
                entity TEXT NULL, value DOUBLE PRECISION)
```

- Escalares de `metrics` (ej. `system.cpu.user`): `entity = NULL`.
- Entradas de los arrays:
  - `disk[...]`     -> `disk.read_bytes_per_sec`, entity = `device`
  - `network[...]`  -> `network.rx_bytes_per_sec`, entity = `interface`
  - `filesystem[...]`-> `filesystem.utilization`, entity = `mountpoint`

Esto da consultas SQL directas por serie (`WHERE agent_id=$1 AND
metric_name=$2 [AND entity=$3] ORDER BY ts DESC`), cardinalidad acotada
por el agent (max 16 entradas por categoria, `OBS_MAX_*`), e indices
orientados al query API (Fase 4): `(agent_id, metric_name, entity, ts
DESC)` y `(ts DESC)`.

La cardinalidad del label de entidad es controlada hoy por los limites
del agent (`OBS_MAX_DISKS/INTERFACES/FILESYSTEMS = 16`); el Collector la
valida del lado del servidor (rechaza >16 entradas por array con 400
`too_many_entities`) para no confiar ciegamente en clientes. Tambien se
acota el objeto plano `metrics` (`MAX_METRIC_KEYS = 1024`, muy por
encima de las ~14 claves que emite el agent) para que un cliente
descompuesto no pueda inyectar miles de series por sample — `400
too_many_metrics`.

### 5. Heartbeat -> `agents.last_seen` (ya en Fase 3)

Cada heartbeat (y cada sample) hace `last_seen = ahora`. La **maquina de
estados ONLINE/DEGRADED/OFFLINE y la deteccion de reboot** (que compara
uptime entre muestras) quedan **explícitamente para la Fase 4**: esta
fase solo mantiene el dato crudo en `agents` para que Fase 4 pueda
derivar el estado sin reprocesar payloads.

## Consecuencias

**Positivas**

- Cero cambios en el agent; el Collector implementa el contrato que el
  agent ya emite.
- Un solo secreto, configurado en un solo lugar.
- Modelo simple de series que el query API de Fase 4 puede explotar con
  SQL plano; sin EAV ni JSONB todavia (si mas adelante hace falta una
  copia fiel del sample crudo, es una migracion aditiva, no un cambio de
  diseno).
- Migraciones embebidas (`sqlx::migrate!`), corren al arranque; no hace
  falta sqlx-cli para desplegar.

**Negativas**

- Un sample completo de un host con muchos dispositivos genera ~200 filas
  (14 escalares + 16 disk x 4 + 16 net x 6 + 16 fs x 3). Es aceptable para
  una plataforma personal; si el volumen creciera, el indice `(ts DESC)`
  y agregaciones en el query API mitigaran el impacto. Se inserte en una
  sola sentencia `INSERT ... SELECT ... UNNEST(...)` por sample.
- Token compartido: todos los agents comparten el mismo secreto. Si un
  host es comprometido, el token hay que rotarlo en todos. Aceptable en
  V1; per-agent tokens son una extension natural para Fase 8 (hardening).

## Consultar tambien

- ADR-0001 (por que el agent es C y el Collector Rust).
- ADR-0002 (transporte, auth header, TLS diferido a Fase 8).
- `agent/src/protocol.c` (serializacion exacta de los payloads).
- `PHASES.md` Fase 3/4.