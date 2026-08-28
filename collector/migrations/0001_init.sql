-- Collector core (Fase 3). Esquema inicial:
--   agents         -> identidad conocida del agent (id persistente, 128 bits)
--   metric_samples -> una fila por (agent, timestamp, metrica) con un
--                     entity NULL para escalares planos o el label del
--                     array (device / mountpoint / interface) para las
--                     metricas por-entidad. Modelo normalizado y plano de
--                     series con nombre dotted-style (system.cpu.user, ...).

CREATE TABLE IF NOT EXISTS agents (
    agent_id   UUID PRIMARY KEY,
    first_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS metric_samples (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    agent_id    UUID NOT NULL REFERENCES agents (agent_id) ON DELETE CASCADE,
    ts          TIMESTAMPTZ NOT NULL,
    metric_name TEXT NOT NULL,
    entity      TEXT,
    value       DOUBLE PRECISION NOT NULL
);

-- Lookup principal: series por agente + metrica (+ entidad), ordenadas
-- por tiempo descendente para el query API de Fase 4.
CREATE INDEX IF NOT EXISTS idx_metric_samples_lookup
    ON metric_samples (agent_id, metric_name, entity, ts DESC);

-- Barrido temporal global (ej. "estado del sistema entre T0 y T1").
CREATE INDEX IF NOT EXISTS idx_metric_samples_ts
    ON metric_samples (ts DESC);