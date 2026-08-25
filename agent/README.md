# observer-agent

Agente de observabilidad en C para hosts Linux, escrito con un presupuesto
de recursos explícito: pocas dependencias, sin llamadas a binarios externos
(`top`, `free`, `ps`, ...), lectura directa de `/proc` y `/sys`.

> Estado actual: **Fase 1** — collectors de CPU y memoria, config parsing,
> logging, y serialización del payload a JSON. El transporte HTTPS real, el
> heartbeat, y el resto de collectors (disk, network, filesystem, uptime,
> procesos) llegan en la Fase 2. Ver [`../PHASES.md`](../PHASES.md).

## Por qué C

Ver [`docs/adr/0001-agent-language.md`](../docs/adr/0001-agent-language.md).
En resumen: el agente necesita interactuar directamente con interfaces
POSIX/Linux y mantener un footprint mínimo (objetivo de diseño: < 5 MB RSS,
< 1% CPU promedio en el hardware de referencia), y eso pesa más que la
comodidad de un runtime más pesado.

## Build

```sh
make            # build de release (-O2)
make debug      # build de debug (-O0 -g)
make sanitize   # build con AddressSanitizer + UndefinedBehaviorSanitizer
make test       # compila y corre los tests unitarios
make clean
```

Requiere un compilador C11 (`cc`/`gcc`/`clang`) y `make`. Sin dependencias
externas.

## Uso

```sh
./observer-agent [ruta/a/agent.conf]
```

Si no se pasa ruta, busca `/etc/observer/agent.conf`. Si el archivo no
existe, corre con valores por defecto (ver `config_set_defaults` en
`src/config.c`) y lo indica por log — no es un error fatal.

En esta fase, el agente imprime cada muestra como una línea JSON por
`stdout` en vez de enviarla al Collector (eso es Fase 2):

```json
{"protocol_version":1,"agent_id":"...","timestamp":1787695105,"metrics":{"system.cpu.utilization":0.0010,...}}
```

La primera muestra de CPU siempre se descarta (`system.cpu.*` no aparece):
la utilización se calcula por delta entre dos snapshots de `/proc/stat`
(ver sección 9 del informe técnico), así que la primera lectura solo sirve
para establecer el punto de partida.

## Configuración

Formato plano `clave = valor`, una entrada por línea, `#` para comentarios:

```ini
# /etc/observer/agent.conf
collector_url = https://collector.local:8443
agent_token = <token>
agent_id_path = /etc/observer/agent-id

metrics_interval_secs = 10
heartbeat_interval_secs = 5

connect_timeout_secs = 3
write_timeout_secs = 3
read_timeout_secs = 5

log_level = info
```

Se eligió deliberadamente no usar una librería TOML/YAML externa: el
formato es simple y esto mantiene el binario liviano y con pocas
dependencias.

## Estructura

```
agent/
├── src/
│   ├── main.c              # loop principal
│   ├── config.c            # parser de configuración
│   ├── agent.c             # utilidades comunes (status codes)
│   ├── protocol.c          # serialización JSON del payload
│   ├── logging.c           # logger con niveles
│   └── collectors/
│       ├── cpu.c           # /proc/stat, delta-based utilization
│       └── memory.c        # /proc/meminfo
├── include/                 # headers correspondientes
├── tests/                   # tests unitarios (sin framework externo)
├── Makefile
└── README.md
```

## Tests

`make test` compila y corre `tests/test_cpu`, `tests/test_memory` y
`tests/test_config`. Son binarios standalone (sin framework externo) que
usan un macro `CHECK()` simple y devuelven código de salida != 0 si algo
falla — pensado para poder engancharlos directo a CI.

Casos cubiertos:

- **CPU**: parseo de línea válida/inválida, argumentos nulos, cálculo de
  delta/utilización, detección conceptual de counter reset.
- **Memoria**: parseo con y sin `MemAvailable` (fallback a `MemFree`),
  `MemTotal` ausente (debe fallar), argumentos nulos.
- **Config**: defaults, carga desde archivo (incluyendo override parcial y
  claves desconocidas), archivo inexistente (debe conservar defaults).

## Próximos pasos (Fase 2)

- `transport.c`: cliente HTTP con connect/write/read timeouts y retry con
  backoff exponencial + jitter (informe, secciones 26–27).
- Collectors de disk, network, filesystem, uptime y process count.
- Heartbeat como canal independiente de las métricas.
- Generación/persistencia real de `agent_id` (actualmente hay un
  placeholder si `/etc/observer/agent-id` no existe).
