# ADR-0002: Protocolo de transporte del agente

## Estado

Aceptado.

## Contexto

El agente necesita enviar metricas y heartbeats al collector con timeouts
explicitos, reintentos ante fallos y, en entornos de produccion, cifrado
del transporte.

## Decision

**HTTP/1.1 plano sobre sockets POSIX** en el agente, sin libcurl ni ninguna
otra dependencia externa. El agente hace POST con body JSON y cabecera
`Authorization: Bearer`. Los timeouts de connect/write/read se controlan
con `setsockopt(SO_SNDTIMEO/SO_RCVTIMEO)` y `poll()`.

El agente **no implementa TLS** y rechaza explicitamente URLs `https://`
con un error claro en lugar de degradar silenciosamente a texto plano.

**TLS se termina en el collector**: el collector puede servir HTTPS nativo
(rustls) para el dashboard y los clientes de navegador. Los agentes en
redes confiables se conectan en texto plano directamente; los agentes
remotos llegan a traves de un tunel (stunnel, WireGuard, SSH port
forwarding) y el agente no necesita saber nada de TLS.

## Justificacion

Implementar TLS desde cero sobre sockets crudos en C, sin enlazar contra
OpenSSL, esta fuera del alcance razonable de este componente. Enlazar
contra OpenSSL agrega una dependencia mayor que contradice la filosofia del
agente. La alternativa de tunel es mas simple, mas flexible y mas segura
porque separa las responsabilidades: el agente hace lo que hace bien
(recolectar y enviar), y el transporte seguro lo resuelve infraestructura
existente.

## Consecuencias

El binario del agente no tiene dependencias dinamicas mas alla de la libc.
El default de `collector_url` es `http://`; cualquier intento de usar
`https://` falla en el arranque con un mensaje explicito. La arquitectura
de red recomendada para entornos publicos es `agente -> tunel -> collector`.
