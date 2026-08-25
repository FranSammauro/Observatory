# ADR-0001: Lenguaje del agent

## Estado

Aceptado

## Contexto

El agente debe ejecutarse en hosts Linux con recursos potencialmente muy
limitados (hardware de referencia experimental: Pentium M, 256 MB RAM,
Alpine Linux), interactuar directamente con interfaces del sistema
(`/proc`, `/sys`, `statvfs()`, sockets), y mantener un footprint de
memoria y binario mínimo.

## Alternativas consideradas

- **Rust**: buena seguridad de memoria y ergonomía, pero el runtime y el
  tamaño de binario típico (aún con `opt-level=z` y sin panics) son mayores
  que en C, y el objetivo explícito de este componente es explorar
  interacción directa y minimalista con el kernel de Linux.
- **Go**: runtime con garbage collector y goroutines, footprint de memoria
  base más alto de lo deseable para el hardware de referencia; menos
  control fino sobre allocations.
- **C**: acceso directo a APIs POSIX/Linux sin capas intermedias, sin
  runtime, binario pequeño, control total sobre memoria.

## Decisión

Usar C (estándar C11) para el agent.

## Consecuencias

**Positivas**

- Overhead de runtime mínimo.
- Acceso directo a `/proc`, `/sys`, `statvfs()` sin bindings intermedios.
- Binario pequeño, apto para el hardware de referencia.

**Negativas**

- Carga manual de la responsabilidad de memory safety (sin borrow checker
  ni GC): mitigado con límites de tamaño explícitos en todos los buffers
  (`OBS_MAX_*` en `agent.h`), `-Wall -Wextra -Wpedantic -Wconversion
  -Wshadow`, y build con AddressSanitizer/UndefinedBehaviorSanitizer
  (`make sanitize`) como parte del flujo de desarrollo.
- Más código boilerplate para cosas que en otros lenguajes serían triviales
  (parsing, formateo de strings) — aceptable dado el alcance acotado del
  agent (collect → serialize → send).

El Collector, en cambio, usa Rust — ver el informe técnico completo para la
justificación de esa separación de responsabilidades por lenguaje.
