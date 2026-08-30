# observer-agent

Agente de monitoreo en C para hosts Linux. Recolecta metricas del sistema
directamente desde las interfaces del kernel sin depender de herramientas
externas como `top`, `free` o `df`, y las envia periodicamente al collector
central.

## Metricas recolectadas

| Fuente | Metricas |
|---|---|
| `/proc/stat` | CPU: utilizacion, user, system, iowait (delta entre snapshots) |
| `/proc/meminfo` | Memoria y swap: total, disponible, utilizacion |
| `/proc/diskstats` | I/O por disco: bytes/s, operaciones/s (discos completos, sin particiones) |
| `/proc/net/dev` | Trafico por interfaz: rx/tx bytes/s, paquetes/s, errores acumulados |
| `/proc/mounts` + `statvfs()` | Filesystem: total, disponible, utilizacion por punto de montaje |
| `/proc/uptime` | Uptime del sistema en segundos |
| `/proc/<pid>/stat` | Conteo agregado de procesos por estado |
| `/sys/class/thermal/` | Temperatura (opcional; ausencia no es error) |

Las metricas basadas en delta (CPU, disco, red) requieren dos lecturas
para calcular la tasa. El primer ciclo establece el punto de referencia;
los datos aparecen a partir del segundo intervalo.

## Build

```sh
make              # release con -O2
make debug        # -O0 -g
make sanitize     # AddressSanitizer + UndefinedBehaviorSanitizer + LeakSanitizer
make test         # compila y ejecuta los 11 test suites
make fuzz         # harness deterministico sobre transport y config parser
make clean
```

Requiere un compilador C11 (`gcc` o `clang`) y `make`. Sin dependencias
externas.

## Configuracion

Archivo de texto plano con formato `clave = valor`. Si el archivo no
existe al arrancar, el agente usa valores por defecto y continua.

```ini
collector_url = http://collector.local:8080
agent_token   = <token>
agent_id_path = /etc/observer/agent-id

metrics_interval_secs   = 10
heartbeat_interval_secs = 5

connect_timeout_secs = 3
write_timeout_secs   = 3
read_timeout_secs    = 5

log_level = info
```

`collector_url` debe usar `http://`. El agente no implementa TLS; ver
`docs/adr/0002-transport-protocol.md` para la justificacion y la
arquitectura de red recomendada.

## Identidad del agente

Al arrancar por primera vez, el agente genera un identificador de 128 bits
desde `/dev/urandom` y lo persiste en `agent_id_path` con permisos `0600`.
Reinicios posteriores reutilizan el mismo identificador, de modo que la
identidad del host no depende del hostname ni de la direccion IP.

## Transporte y reintentos

El agente mantiene dos canales independientes programados con
`CLOCK_MONOTONIC`: uno para las metricas y otro para el heartbeat. Ante un
fallo de envio en cualquiera de los dos, se aplica backoff exponencial con
jitter (1s, 2s, 4s, 8s, 16s, 30s, 30s...) sin bloquear al otro canal. No
se almacena la muestra fallida; el siguiente ciclo envia datos frescos.

## Estructura del codigo

```
agent/
├── src/
│   ├── main.c                    Loop principal, scheduling, envio
│   ├── config.c                  Parser de configuracion
│   ├── logging.c                 Logger con niveles TRACE..ERROR
│   ├── protocol.c                Serializador JSON del payload
│   ├── transport.c               Cliente HTTP sobre sockets POSIX
│   ├── retry.c                   Backoff exponencial con jitter
│   ├── identity.c                Identidad persistente del agente
│   ├── agent.c                   Utilidades comunes
│   └── collectors/
│       ├── cpu.c
│       ├── memory.c
│       ├── disk.c
│       ├── network.c
│       ├── filesystem.c
│       ├── uptime.c
│       ├── process.c
│       └── temperature.c
├── include/                      Headers correspondientes
├── tests/                        Test suites unitarios (sin framework externo)
├── Makefile
└── README.md
```

## Tests

Cada suite es un binario standalone que devuelve codigo de salida distinto
de cero ante cualquier fallo, apto para integracion directa con CI.

| Suite | Cobertura |
|---|---|
| `test_cpu` | Parseo de `/proc/stat`, calculo de delta, deteccion de counter reset |
| `test_memory` | Parseo de `/proc/meminfo`, fallback de MemAvailable a MemFree |
| `test_config` | Carga de archivo, overrides parciales, claves desconocidas |
| `test_disk` | Filtrado de particiones, parseo de `/proc/diskstats`, contrato de primer ciclo |
| `test_network` | Parseo de `/proc/net/dev`, filtrado de loopback |
| `test_filesystem` | Filtrado de pseudo-filesystems, parseo de `/proc/mounts` |
| `test_uptime` | Parseo de `/proc/uptime` |
| `test_process` | Normalizacion de estados de proceso, coleccion real contra `/proc` |
| `test_retry` | Crecimiento exponencial, clamp al maximo, reproducibilidad con misma seed |
| `test_transport` | Parser de URL (esquemas, puertos, errores) sin abrir sockets |
| `test_identity` | Generacion, persistencia (permisos 0600), reuso de id existente, archivo corrupto |
