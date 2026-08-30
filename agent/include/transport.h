#ifndef OBSERVER_TRANSPORT_H
#define OBSERVER_TRANSPORT_H

#include "agent.h"

/*
 * Cliente HTTP minimo sobre sockets POSIX, sin dependencias externas.
 * Soporta HTTP plano unicamente; collector_url con esquema https:// es
 * rechazado explicitamente para evitar una falsa sensacion de seguridad.
 * TLS se termina en el propio collector; los agentes se conectan en texto
 * plano dentro de la red local o a traves de un tunel.
 */

typedef struct {
    unsigned int connect_timeout_secs;
    unsigned int write_timeout_secs;
    unsigned int read_timeout_secs;
} transport_config_t;

typedef struct {
    int status_code;
    bool got_response;
} transport_result_t;

/* Expuesto para tests unitarios: parsea la URL sin abrir sockets. */
obs_status_t transport_parse_url(const char *base_url, char *host, size_t host_size,
                                  uint16_t *port, bool *is_https);

obs_status_t transport_post(const transport_config_t *config,
                             const char *base_url,
                             const char *path,
                             const char *token,
                             const char *body,
                             transport_result_t *result);

#endif /* OBSERVER_TRANSPORT_H */
