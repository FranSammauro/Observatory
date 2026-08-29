-- Health checks proactivos (Fase 6, bloque 6.1).
--
-- Checks HTTP/TCP que el collector ejecuta periodica y proactivamente
-- contra targets externos, registrando cada corrida (resultado) y el
-- estado actual deduplicado por check (para el WS de la Fase 6 y para
-- no recalcular la transicion up/down).
--
--   ts -> hora de arribo del ciclo (filosofia last_seen, ADR-0003).

CREATE TABLE IF NOT EXISTS health_checks (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    kind          TEXT NOT NULL CHECK (kind IN ('http', 'tcp')),
    target        TEXT NOT NULL,
    interval_secs BIGINT NOT NULL CHECK (interval_secs >= 1),
    timeout_secs  BIGINT NOT NULL CHECK (timeout_secs >= 1),
    enabled       BOOLEAN NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Una fila por corrida del check. Ok/latencia/detalle de esa ejecucion.
CREATE TABLE IF NOT EXISTS health_check_results (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    check_id    BIGINT NOT NULL REFERENCES health_checks (id) ON DELETE CASCADE,
    ts          TIMESTAMPTZ NOT NULL,
    ok          BOOLEAN NOT NULL,
    latency_ms  BIGINT NOT NULL CHECK (latency_ms >= 0),
    detail      TEXT NOT NULL
);

-- Timeline por check.
CREATE INDEX IF NOT EXISTS idx_health_results_check ON health_check_results (check_id, ts DESC);

-- Estado actual de un check: UP/DOWN desde cuando, y el ultimo outcome.
-- `since` se conserva mientras el estado no cambia; la transicion
-- (up<->down) lo reinicia. Es lo que consume el WebSocket (6.2) y el
-- detalle de la list.
CREATE TABLE IF NOT EXISTS health_check_states (
    check_id         BIGINT PRIMARY KEY REFERENCES health_checks (id) ON DELETE CASCADE,
    state            TEXT NOT NULL CHECK (state IN ('up', 'down')),
    since            TIMESTAMPTZ NOT NULL,
    last_checked_at  TIMESTAMPTZ NOT NULL,
    last_ok          BOOLEAN NOT NULL,
    last_latency_ms  BIGINT NOT NULL CHECK (last_latency_ms >= 0),
    last_detail      TEXT NOT NULL
);