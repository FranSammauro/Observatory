#ifndef OBSERVER_IDENTITY_H
#define OBSERVER_IDENTITY_H

#include "agent.h"

/*
 * Identidad persistente del agent (informe tecnico, seccion 20).
 *
 * El identificador NO depende exclusivamente del hostname: se genera un
 * identificador aleatorio de alta entropia la primera vez que el agent
 * corre, y se persiste en disco (`agent_id_path`, por defecto
 * /etc/observer/agent-id) con permisos restrictivos para que sobreviva a
 * reinicios y a cambios de hostname.
 */

#define OBS_AGENT_ID_LEN 32  /* 16 bytes random -> 32 hex chars */

/*
 * Devuelve el agent_id en `out` (buffer de al menos OBS_AGENT_ID_LEN+1
 * bytes). Si `path` ya existe y contiene un id valido, lo reutiliza.
 * Si no existe, genera uno nuevo, lo persiste en `path` con permisos
 * 0600, y lo devuelve.
 */
obs_status_t identity_resolve(const char *path, char *out, size_t out_size);

#endif /* OBSERVER_IDENTITY_H */
