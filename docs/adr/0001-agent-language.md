# ADR-0001: Lenguaje del agente

## Estado

Aceptado.

## Contexto

El agente debe ejecutarse en hosts Linux con recursos potencialmente muy
limitados. El hardware de referencia experimental es un Pentium M con
256 MB de RAM corriendo Alpine Linux. El agente necesita:

- interactuar directamente con interfaces del kernel (`/proc`, `/sys`,
  `statvfs()`, sockets POSIX);
- mantener un footprint de memoria y binario minimo;
- funcionar sin privilegios de root para las metricas basicas;
- compilar con un toolchain estandar sin dependencias de sistema.

## Alternativas consideradas

**Rust**: garantias de memory safety en tiempo de compilacion y ergonomia
alta, pero el runtime y el tamano de binario tipico son mayores que en C,
incluso con perfiles agresivos de optimizacion.

**Go**: runtime con garbage collector, footprint de memoria base
significativo. El objetivo de menos de 5 MB de RSS en reposo lo
descarta para el hardware de referencia.

**C**: sin runtime, sin garbage collector, acceso directo a POSIX/Linux
sin capas intermedias, binario pequeno. El costo es la responsabilidad
manual de la gestion de memoria, mitigado con politicas explicitas
(buffers de tamano fijo, limites en `agent.h`, compilacion con
`-Wall -Wextra -Wpedantic -Wconversion -Wshadow` y builds de
AddressSanitizer/UndefinedBehaviorSanitizer como parte del flujo de
desarrollo normal).

## Decision

C11 para el agente.

## Consecuencias

El collector, que resuelve un problema diferente (concurrencia, networking,
persistencia, evaluacion de reglas), usa Rust. La separacion es
intencional: cada componente usa el lenguaje mas adecuado para su problema,
no el mismo lenguaje por conveniencia.
