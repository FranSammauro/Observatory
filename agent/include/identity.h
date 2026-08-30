#ifndef OBSERVER_IDENTITY_H
#define OBSERVER_IDENTITY_H

#include "agent.h"

/*
 * Identidad persistente del agente. El identificador no depende del
 * hostname para evitar colisiones al renombrar maquinas o clonar
 * imagenes. Se genera una vez con entropia de /dev/urandom y se persiste
 * en disco con permisos 0600.
 */

#define OBS_AGENT_ID_LEN 32

obs_status_t identity_resolve(const char *path, char *out, size_t out_size);

#endif /* OBSERVER_IDENTITY_H */
