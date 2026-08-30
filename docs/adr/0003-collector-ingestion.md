# ADR-0003: Modelo de ingestion y persistencia del collector

## Estado

Aceptado.

## Contexto

Al disenar el collector hay cinco decisiones interrelacionadas que
determinan el modelo de datos, el comportamiento ante fallos y la
semantica de las metricas almacenadas.

## Decisiones

**1. Registro implicito de agentes**

No hay un endpoint de registro separado en el protocolo del agente. El
primer heartbeat o sample valido crea la fila en `agents` con un INSERT
ON CONFLICT. Esto simplifica el protocolo y el agente: no hay que gestionar
el estado "registrado / no registrado" ni manejar fallos de registro.

**2. Semantica de `last_seen`**

`agents.last_seen` usa la hora de arribo al servidor (`now()` en SQL), no
el timestamp que reporta el agente. Para determinar si un agente esta vivo
importa "cuando lo oi yo", no "que hora tiene el host". Un host con el
reloj corrido 5 minutos no debe aparecer como offline estando activo.

**3. Modelo de almacenamiento de metricas**

Una fila por `(agent_id, timestamp, metric_name, entity, value)` en la
tabla `metric_samples`. Los escalares tienen `entity = NULL`; las metricas
por dispositivo o interfaz tienen el identificador correspondiente como
`entity`. Este modelo es mas consultable que JSONB (los indices sobre
columnas tipadas son mas eficientes) sin ser mas complejo de mantener que
un esquema columnar especializado. Si el volumen lo justifica en el futuro,
la migracion a TimescaleDB es incremental sobre esta misma tabla.

**4. Deteccion de reboots**

`system.uptime` es monotonamente creciente desde el arranque. Una caida de
uptime entre dos muestras consecutivas del mismo agente indica un reboot.
La deteccion ocurre dentro de una transaccion que incluye un lock del
agente (`SELECT FOR UPDATE`) para serializar ingestiones concurrentes y
evitar que dos workers lean el mismo uptime anterior y dupliquen el evento.

**5. Estado de conectividad como funcion derivada**

El estado ONLINE/DEGRADED/OFFLINE no se almacena como un campo; se calcula
al leer a partir de `agents.last_seen`. Una funcion pura sobre la edad de
`last_seen` devuelve el estado actual. El historial de transiciones si se
persiste (tabla `connectivity_events`), generado por un runner periodico
que compara el estado derivado contra el ultimo estado registrado.
