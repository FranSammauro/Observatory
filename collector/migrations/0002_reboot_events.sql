-- Deteccion de reboot (Fase 4, bloque 4.3). Una fila por reboot
-- detectado: cuando una muestra de system.uptime llega con un valor menor
-- que el ultimo conocido del mismo agent (caida mayor a
-- OBS_REBOOT_MIN_UPTIME_DROP_SECS) se registra el evento en ingestion.
--   uptime_before -> valor previo conocido
--   uptime_after  -> valor que revelo el reboot (tras arrancar)
--   sample_ts     -> timestamp del agent de esa muestra
--   detected_at   -> hora de arribo (filosofia last_seen, ADR-0003)

CREATE TABLE IF NOT EXISTS reboot_events (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    agent_id      UUID NOT NULL REFERENCES agents (agent_id) ON DELETE CASCADE,
    detected_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    sample_ts     TIMESTAMPTZ NOT NULL,
    uptime_before DOUBLE PRECISION NOT NULL,
    uptime_after  DOUBLE PRECISION NOT NULL
);

-- Consulta por agent (timeline de reboots de un host, query API 4.3).
CREATE INDEX IF NOT EXISTS idx_reboot_events_agent
    ON reboot_events (agent_id, detected_at DESC);