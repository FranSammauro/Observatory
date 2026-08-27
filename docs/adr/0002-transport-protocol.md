# ADR-0002: Protocolo de transporte del agent

## Estado

Aceptado (parcial - ver "Pendiente" abajo)

## Contexto

El agent necesita enviar metricas y heartbeats al Collector de forma
periodica (informe tecnico, secciones 21-27). El transporte debe:

- tener timeouts explicitos de connect/write/read (nunca bloquear
  indefinidamente);
- reintentar con backoff exponencial + jitter ante fallos;
- eventualmente viajar sobre TLS en produccion (seccion 23).

## Alternativas consideradas para el cliente HTTP

- **libcurl**: la opcion mas comun para HTTP en C. Trae manejo de TLS,
  redirects, keep-alive, etc. "gratis".
- **Sockets POSIX crudos + parser HTTP/1.1 manual**: sin dependencias,
  control total sobre timeouts, footprint minimo.

## Decision

Sockets POSIX crudos (`transport.c`), sin libcurl ni ninguna libreria
HTTP externa.

Esto es consistente con la filosofia general del agent (informe
secciones 6.1 y 73.2: pocas dependencias, no reinventar por sport pero
tampoco traer un dependency tree grande para un cliente HTTP simple que
solo necesita hacer POST con un body JSON conocido).

## TLS: diferido a Fase 8 (Hardening)

Esta fase (Fase 2) implementa el transporte en **texto plano
(`http://`)**. `transport_post()` rechaza explicitamente URLs
`https://` con un error claro (`OBS_ERR_UNAVAILABLE` + log), en vez de
degradar silenciosamente a texto plano bajo una URL que promete TLS -
eso seria peor que fallar ruidosamente.

Implementar TLS desde cero en C sobre sockets crudos (sin OpenSSL/
libressl como dependencia) esta fuera de alcance razonable para este
componente. La Fase 8 (Hardening, ver `PHASES.md`) evaluara entre:

1. Enlazar contra OpenSSL/LibreSSL directamente (agrega una dependencia,
   pero es la mas estandar para TLS en C).
2. Terminar TLS en un reverse proxy / stunnel delante del Collector, y
   dejar el agent hablando HTTP en plano solo dentro de una red
   confiable o un tunel (mockeando parcialmente informe seccion 23,
   que asume TLS end-to-end).

La decision entre esas dos opciones se toma en Fase 8, con mas contexto
sobre el entorno real de despliegue.

## Consecuencias

**Positivas**

- Cero dependencias nuevas para el agent.
- Control total y explicito sobre connect/write/read timeouts
  (`setsockopt(SO_SNDTIMEO/SO_RCVTIMEO)`, `poll()` para el connect).
- El parser de URL (`transport_parse_url`) y la logica de request/
  response son testeables unitariamente sin abrir sockets.

**Negativas**

- Parser HTTP/1.1 minimo hecho a mano: no soporta chunked
  transfer-encoding, redirects, ni keep-alive (se usa
  `Connection: close` en cada request deliberadamente, para simplificar
  la lectura de la respuesta).
- Sin TLS hasta Fase 8 - **no usar en una red no confiable todavia**.
  El default de `collector_url` se cambio a `http://127.0.0.1:8080`
  (antes `https://...`) para reflejar el estado real de esta fase.
