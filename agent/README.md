# observer-agent

Agente de observabilidad en C para hosts Linux, escrito con un presupuesto
de recursos explícito: pocas dependencias, sin llamadas a binarios externos
(`top`, `free`, `ps`, ...), lectura directa de `/proc` y `/sys`.

> Estado actual: **Fase 2** — resto de collectors (disk, network,
> filesystem, uptime, process, temperature), transporte HTTP real sobre
> sockets POSIX, heartbeat como canal independiente de las métricas,
> retry con backoff exponencial + jitter, e identidad persistente del
> agent. Ver [`../PHASES.md`](../PHASES.md).

## Por qué C

Ver [`docs/adr/0001-agent-language.md`](../docs/adr/0001-agent-language.md).
En resumen: el agente necesita interactuar directamente con interfaces
POSIX/Linux y mantener un footprint mínimo (objetivo de diseño: < 5 MB RSS,
< 1% CPU promedio en el hardware de referencia), y eso pesa más que la
comodidad de un runtime más pesado.

## Transporte: HTTP plano por ahora, TLS en Fase 8

Ver [`docs/adr/0002-transport-protocol.md`](../docs/adr/0002-transport-protocol.md).
`collector_url` debe empezar con `http://` en esta fase — `https://` es
rechazado explícitamente (con un error claro) en vez de degradar
silenciosamente a texto plano. **No usar sobre una red no confiable
todavía.**

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

El agent corre en foreground (para ejecutarlo como daemon, envolverlo en
un unit de systemd — pendiente para una fase posterior). Cada
`metrics_interval_secs` recolecta una muestra completa y la envía por
`POST {collector_url}/api/v1/metrics`; cada `heartbeat_interval_secs`
(canal independiente, ver informe sección 24) envía un heartbeat liviano
por `POST {collector_url}/api/v1/agents/heartbeat`. Ambos incluyen
`Authorization: Bearer {agent_token}` si está configurado.

Si un envío falla (Collector caído, timeout, respuesta no-2xx), el
agent no bloquea el resto del loop: aplica backoff exponencial + jitter
(1s, 2s, 4s, 8s, 16s, 30s, 30s...) antes del próximo intento **de ese
canal**, y sigue recolectando/enviando el otro canal normalmente. En el
próximo intento se envía una muestra fresca, no la vieja (informe
sección 27: sin buffer local en V1).

## Configuración

Formato plano `clave = valor`, una entrada por línea, `#` para comentarios:

```ini
# /etc/observer/agent.conf
collector_url = http://collector.local:8080
agent_token = <token>
agent_id_path = /etc/observer/agent-id

metrics_interval_secs = 10
heartbeat_interval_secs = 5

connect_timeout_secs = 3
write_timeout_secs = 3
read_timeout_secs = 5

log_level = info
```

`collector_url` debe usar `http://` en esta fase (ver nota de TLS más
arriba). Se eligió deliberadamente no usar una librería TOML/YAML
externa: el formato es simple y esto mantiene el binario liviano y con
pocas dependencias.

## Identidad del agent

El agent NO depende del hostname para identificarse (informe sección
20): la primera vez que corre, genera un id aleatorio de 128 bits
(vía `/dev/urandom`) y lo persiste en `agent_id_path` (default
`/etc/observer/agent-id`) con permisos `0600`. Corridas siguientes
reutilizan ese mismo id. Si el archivo no se puede crear/leer (permisos,
directorio inexistente), el agent sigue funcionando con un id generado
en memoria solo para esa ejecución — no es un error fatal, pero no
sobrevive a un reinicio.

## Estructura

```
agent/
├── src/
│   ├── main.c              # loop principal (scheduling monotonic, retry)
│   ├── config.c            # parser de configuración
│   ├── agent.c              # utilidades comunes (status codes)
│   ├── protocol.c           # serialización JSON (sample + heartbeat)
│   ├── transport.c          # cliente HTTP sobre sockets POSIX
│   ├── retry.c               # backoff exponencial + jitter
│   ├── identity.c            # identidad persistente del agent
│   ├── logging.c            # logger con niveles
│   └── collectors/
│       ├── cpu.c            # /proc/stat, delta-based utilization
│       ├── memory.c         # /proc/meminfo
│       ├── disk.c            # /proc/diskstats, tasas de I/O
│       ├── network.c        # /proc/net/dev, tasas por interfaz
│       ├── filesystem.c     # /proc/mounts + statvfs()
│       ├── uptime.c         # /proc/uptime
│       ├── process.c        # /proc/<pid>/stat, conteo agregado
│       └── temperature.c    # /sys/class/thermal (opcional)
├── include/                  # headers correspondientes
├── tests/                    # tests unitarios (sin framework externo)
├── Makefile
└── README.md
```

## Tests

`make test` compila y corre 11 suites (`test_cpu`, `test_memory`,
`test_config`, `test_disk`, `test_network`, `test_filesystem`,
`test_uptime`, `test_process`, `test_retry`, `test_transport`,
`test_identity`). Son binarios standalone (sin framework externo) que
usan un macro `CHECK()` simple y devuelven código de salida != 0 si algo
falla — pensado para poder engancharlos directo a CI.

Casos cubiertos (además de lo ya descripto en Fase 1):

- **Disk/Network**: parseo de `/proc/diskstats` y `/proc/net/dev`,
  filtrado de particiones/loopback, contrato de "primera lectura sin
  delta".
- **Filesystem**: filtrado de filesystems pseudo/virtuales, parseo de
  `/proc/mounts`, integración liviana contra el sistema real.
- **Uptime/Process**: parseo, y ejecución real contra `/proc` del
  contenedor (siempre hay al menos un proceso corriendo).
- **Retry**: crecimiento exponencial del backoff, clamp al máximo,
  reproducibilidad con la misma seed, manejo del caso seed=0.
- **Transport**: parseo de URL (`http://`/`https://`, puertos
  explícitos/default, errores de esquema/puerto inválido) — sin abrir
  sockets.
- **Identity**: generación + persistencia con permisos `0600`, reuso de
  un id existente, manejo de archivo corrupto, argumentos inválidos.

Validado además con una prueba de integración real (agent real +
mock collector HTTP en Python) confirmando el flujo completo
`collect → serialize → POST → retry` con backoff correcto ante fallos
de conexión, y con `make sanitize` (ASan + UBSan) sin hallazgos tras
ejercitar los paths de red reales.

## Próximos pasos (Fase 3+)

- Collector en Rust: registro de agentes, ingestion con validación,
  autenticación por bearer token, persistencia en PostgreSQL.
- TLS en el transporte del agent (Fase 8, ver ADR-0002).
- Generación de un unit de systemd para correr el agent como daemon.
