# Personal Observability Platform

Plataforma distribuida de observabilidad para hosts Linux: agentes ligeros
en C que recolectan métricas de bajo nivel, y un collector central en Rust
que las recibe, persiste, evalúa reglas de alerta y las expone vía API
REST + WebSocket.

```
Agent (C) --HTTPS/JSON--> Collector (Rust) --> PostgreSQL
                                |
                                +--> API REST + WebSocket --> Dashboard
```

No busca competir con Prometheus/Grafana/Zabbix/OpenTelemetry Collector.
El objetivo es construir desde cero una plataforma pequeña para entender
qué problemas aparecen al diseñar software distribuido que observa
máquinas reales — con una hipótesis técnica comprobable de fondo: un
agente Linux con presupuesto de recursos explícito, corriendo en hardware
extremadamente limitado (Pentium M, 256 MB RAM, Alpine Linux), con
benchmarks documentados.

Ver [`PHASES.md`](PHASES.md) para el estado del desarrollo por fases, y
[`docs/adr/`](docs/adr/) para las decisiones de arquitectura.

## Estructura

```
personal-observability/
├── agent/          # Agent en C (ver agent/README.md)
├── collector/
│   ├── dashboard/    # UI estatica del dashboard (Fase 7, servida por el collector)
├── deploy/           # Docker, systemd, config de Alpine
├── docs/
│   └── adr/          # Architecture Decision Records
├── benchmarks/       # Resultados de benchmarks (Fase 8+)
├── PHASES.md
└── README.md
```

## Estado actual

**Fases 1–3 completas**: agent en C con todos los collectors del sistema
(CPU, memoria, disk, network, filesystem, uptime, procesos, temperatura
opcional), transporte HTTP real sobre sockets POSIX, heartbeat
independiente, retry con backoff+jitter, e identidad persistente — y ya
el **Collector central en Rust** (Axum) que recibe esos payloads: registro
implicito de agentes, ingestion con validación, autenticación por bearer
token y persistencia en PostgreSQL. Ver [`agent/README.md`](agent/README.md)
y [`collector/README.md`](collector/README.md). Decisiones de ingestion:
[`docs/adr/0003-collector-ingestion.md`](docs/adr/0003-collector-ingestion.md).

Ya del dashboard (Fase 7), entregado completo: **7.1 overview skeleton**
(login bearer, summary cards, lista de agents y timeline en vivo por WS),
**7.2 host page** (detalle por agent, series con grafica SVG, timeline
del host y reboots) y **7.3 alertas e historicos** (gestion de rules y
checks, alertas activas, historial de alertas e historial unificado con
filtros), servidos por el propio collector. Del camino hacia producción
falta hardening/TLS/benchmark (Fase 8). Ya entregadas: query API +
estados de conectividad (Fase 4), alert engine (Fase 5) y health checks
+ eventos realtime + historial unificado + summary (Fase 6).

TLS todavía no está implementado (queda para la Fase 8 de hardening) —
ver [`docs/adr/0002-transport-protocol.md`](docs/adr/0002-transport-protocol.md).
