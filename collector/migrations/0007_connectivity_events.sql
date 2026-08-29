-- Eventos de conectividad (Fase 6, bloque 6.3).
--
-- La maquina ONLINE/DEGRADED/OFFLINE (bloque 4.2) es derivada:
-- se recalcula al leer a partir de agents.last_seen. El bloque 6.3 le
-- agrega historial: un runner periodico detecta cuando el estado
-- derivado de un agent cambia con respecto al ultimo persistido y
-- registra la transicion en `connectivity_events` (la misma "historia"
-- que alerts/health/reboots para el timeline unificado).
--
--   agents.last_connectivity_state -> ultimo estado observado por el
--       runner (ONLINE/DEGRADED/OFFLINE); NULL hasta la primera pasada.
--   connectivity_events            -> una fila por transicion: desde que
--       estado y hacia cual, con ts = hora del ciclo que lo detecto
--       (filosofia last_seen, ADR-0003).

ALTER TABLE agents
    ADD COLUMN last_connectivity_state TEXT;

CREATE TABLE IF NOT EXISTS connectivity_events (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    agent_id    UUID NOT NULL REFERENCES agents (agent_id) ON DELETE CASCADE,
    from_state  TEXT,
    to_state    TEXT NOT NULL CHECK (to_state IN ('online', 'degraded', 'offline')),
    ts          TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Timeline por agent (mismo patron que alert_events/reboot_events).
CREATE INDEX IF NOT EXISTS idx_connectivity_events_agent
    ON connectivity_events (agent_id, ts DESC);