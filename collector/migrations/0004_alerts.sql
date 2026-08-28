-- Maquina de estados de alertas (Fase 5, bloque 5.2).
--
-- Una fila por alerta ACTIVA (pending o firing), deduplicada por
-- (rule_id, agent_id). INACTIVE y RESOLVED no se persisten: INACTIVE es
-- la ausencia de fila, y al resolverse la fila se borra (el detalle de
-- las transiciones llegara al historial del bloque 5.3).
--
--   since        -> inicio del tramo continuo actual de la condicion
--   resolve_from -> hysteresis: cuando la condicion dejo de sostenerse.
--                   La alerta sigue FIRING hasta que la ventana vence
--                   (OBS_ALERT_RESOLVE_GRACE_SECS), para no flapear;
--                   si la condicion se reestablece, se limpia.
--   checked_at   -> ultimo ciclo en que el evaluador toco la fila

CREATE TABLE IF NOT EXISTS alerts (
    rule_id      BIGINT NOT NULL REFERENCES alert_rules (id) ON DELETE CASCADE,
    agent_id     UUID NOT NULL,
    state        TEXT NOT NULL CHECK (state IN ('pending', 'firing')),
    since        TIMESTAMPTZ NOT NULL,
    resolve_from TIMESTAMPTZ,
    checked_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (rule_id, agent_id)
);

-- Barrido de alertas activas de un agent (detalle del host).
CREATE INDEX IF NOT EXISTS idx_alerts_agent ON alerts (agent_id);