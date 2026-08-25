#ifndef OBSERVER_CONFIG_H
#define OBSERVER_CONFIG_H

#include "agent.h"
#include "logging.h"

/*
 * Configuracion del agent, cargada desde un archivo de texto plano
 * con formato `clave = valor` (una entrada por linea, '#' para comentarios).
 *
 * No usamos una libreria TOML/YAML externa a proposito: mantener el
 * agent con pocas dependencias es un requisito explicito del diseno
 * (informe tecnico, seccion 6.1 y 73.2 - anti premature optimization).
 */
typedef struct {
    char collector_url[OBS_MAX_LINE];
    char agent_token[OBS_MAX_LINE];
    char agent_id_path[OBS_MAX_PATH];

    unsigned int heartbeat_interval_secs;
    unsigned int metrics_interval_secs;

    unsigned int connect_timeout_secs;
    unsigned int write_timeout_secs;
    unsigned int read_timeout_secs;

    log_level_t log_level;
} obs_config_t;

/* Carga defaults razonables (ver informe, seccion 24 y 26). */
void config_set_defaults(obs_config_t *config);

/* Parsea un archivo de configuracion y sobreescribe los defaults. */
obs_status_t config_load(const char *path, obs_config_t *config);

#endif /* OBSERVER_CONFIG_H */
