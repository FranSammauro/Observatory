#ifndef OBSERVER_TRANSPORT_H
#define OBSERVER_TRANSPORT_H

#include "agent.h"

/*
 * Cliente HTTP minimo sobre sockets POSIX (informe tecnico, seccion 21
 * y 26). Sin dependencias externas, en linea con el resto del agent.
 *
 * IMPORTANTE: esta fase implementa HTTP en texto plano. TLS se agrega
 * en la Fase 8 (Hardening) - ver docs/adr/0002-transport-protocol.md.
 * No usar collector_url con https:// todavia; transport_post lo
 * rechaza explicitamente para no dar una falsa sensacion de seguridad.
 */

typedef struct {
    unsigned int connect_timeout_secs;
    unsigned int write_timeout_secs;
    unsigned int read_timeout_secs;
} transport_config_t;

typedef struct {
    int status_code;   /* codigo HTTP de la respuesta, si se pudo leer */
    bool got_response;  /* false si fallo antes de recibir una respuesta */
} transport_result_t;

/*
 * Parsea "http://host[:port]" o "https://host[:port]" en host/port/scheme.
 * Expuesto para tests unitarios (no requiere abrir sockets).
 */
obs_status_t transport_parse_url(const char *base_url, char *host, size_t host_size,
                                  uint16_t *port, bool *is_https);

/* Parsea la linea de status de una respuesta HTTP ("HTTP/1.1 200 OK...").
 * Expuesto para fuzz/tests; false si no es una linea de status valida. */
bool transport_parse_status_line(const char *response, int *status_code);

/*
 * Hace un POST a `base_url` + `path` con el body dado (se asume JSON) y
 * el token como `Authorization: Bearer <token>` (omitido si token es
 * NULL o vacio). base_url debe tener forma "http://host[:puerto]".
 *
 * Devuelve OBS_OK si se pudo completar el request HTTP (incluso si el
 * servidor respondio con un status >= 400 - eso se refleja en
 * result->status_code, no como error de transporte). Devuelve un
 * codigo de error si fallo la conexion, el parseo de la URL, o hubo un
 * timeout.
 */
obs_status_t transport_post(const transport_config_t *config,
                             const char *base_url,
                             const char *path,
                             const char *token,
                             const char *body,
                             transport_result_t *result);

#endif /* OBSERVER_TRANSPORT_H */
