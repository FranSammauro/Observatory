-- Alert engine (Fase 5, bloque 5.1): reglas declarativas.
--
-- Una regla describe QUE observar (metrica + entidad opcional), la
-- CONDICION (operador + umbral) y cuanto debe sostenerse antes de contar
-- como alerta (`for_secs`). `entity` es NULL para metricas escalares o el
-- label exacto (device/interface/mountpoint) para las por-entidad.
-- `for_secs` lo consume la maquina de estados del bloque 5.2; aca solo se
-- almacena.

CREATE TABLE IF NOT EXISTS alert_rules (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    metric_name TEXT NOT NULL,
    entity      TEXT,
    op          TEXT NOT NULL CHECK (op IN ('ge', 'gt', 'le', 'lt')),
    threshold   DOUBLE PRECISION NOT NULL,
    for_secs    BIGINT NOT NULL DEFAULT 0 CHECK (for_secs >= 0),
    enabled     BOOLEAN NOT NULL DEFAULT true,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Barrido de reglas habilitadas que hace el evaluador cada ciclo.
CREATE INDEX IF NOT EXISTS idx_alert_rules_enabled ON alert_rules (enabled);