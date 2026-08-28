-- Historial de transiciones de alertas (Fase 5, bloque 5.3).
--
-- Una fila por transicion REAL de estado de la maquina del bloque 5.2
-- para un (rule, agent):
--   creacion    : INACTIVE -> pending | firing   (from_state NULL)
--   promocion   : pending  -> firing
--   resolucion  : pending|firing -> resolved
--
-- Idempotencia: el evaluador solo escribe eventos en los pasos que
-- cambian de estado (Stay* y la ventana de hysteresis no emiten nada),
-- y el estado actual persistido en `alerts` garantiza que al reiniciar
-- no se re-emite una transicion ya registrada.
--
--   ts -> hora de arribo del ciclo (filosofia last_seen, ADR-0003).

CREATE TABLE IF NOT EXISTS alert_events (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    rule_id    BIGINT NOT NULL REFERENCES alert_rules (id) ON DELETE CASCADE,
    agent_id   UUID NOT NULL,
    from_state TEXT CHECK (from_state IN ('pending', 'firing')),
    to_state   TEXT NOT NULL CHECK (to_state IN ('pending', 'firing', 'resolved')),
    ts         TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Timeline por agent y por regla (query API del bloque 5.3).
CREATE INDEX IF NOT EXISTS idx_alert_events_agent ON alert_events (agent_id, ts DESC);
CREATE INDEX IF NOT EXISTS idx_alert_events_rule ON alert_events (rule_id, ts DESC);