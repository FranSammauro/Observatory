#ifndef OBSERVER_CONFIG_H
#define OBSERVER_CONFIG_H

#include "agent.h"
#include "logging.h"

/*
 * Configuracion del agente cargada desde un archivo de texto plano con
 * formato "clave = valor". No se usa ninguna libreria externa de parseo
 * para mantener el numero de dependencias al minimo.
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

void config_set_defaults(obs_config_t *config);
obs_status_t config_load(const char *path, obs_config_t *config);

/* Expuesto para el harness de fuzzing. */
void config_parse_text(const char *text, obs_config_t *config);

#endif /* OBSERVER_CONFIG_H */
