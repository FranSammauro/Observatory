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
├── collector/       # Collector en Rust (Fase 3+)
├── dashboard/        # Dashboard (Fase 7+)
├── deploy/           # Docker, systemd, config de Alpine
├── docs/
│   └── adr/          # Architecture Decision Records
├── benchmarks/       # Resultados de benchmarks (Fase 8+)
├── PHASES.md
└── README.md
```

## Estado actual

**Fase 2 completa**: agent en C con todos los collectors del sistema
(CPU, memoria, disk, network, filesystem, uptime, procesos, temperatura
opcional), transporte HTTP real sobre sockets POSIX, heartbeat
independiente, retry con backoff+jitter, e identidad persistente. Ver
[`agent/README.md`](agent/README.md) para build, uso y tests.

TLS todavía no está implementado (queda para la Fase 8 de hardening) —
ver [`docs/adr/0002-transport-protocol.md`](docs/adr/0002-transport-protocol.md).
