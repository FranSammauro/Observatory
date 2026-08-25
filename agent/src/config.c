#include "config.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>
#include <ctype.h>
#include <stdlib.h>

#define COMPONENT "config"

static char *trim(char *s)
{
    while (isspace((unsigned char)*s)) {
        s++;
    }
    if (*s == '\0') {
        return s;
    }
    char *end = s + strlen(s) - 1;
    while (end > s && isspace((unsigned char)*end)) {
        *end = '\0';
        end--;
    }
    return s;
}

void config_set_defaults(obs_config_t *config)
{
    memset(config, 0, sizeof(*config));

    snprintf(config->collector_url, sizeof(config->collector_url),
              "https://127.0.0.1:8443");
    config->agent_token[0] = '\0';
    snprintf(config->agent_id_path, sizeof(config->agent_id_path),
              "/etc/observer/agent-id");

    /* Heartbeat independiente de metrics, ver informe seccion 24 */
    config->heartbeat_interval_secs = 5;
    config->metrics_interval_secs = 10;

    config->connect_timeout_secs = 3;
    config->write_timeout_secs = 3;
    config->read_timeout_secs = 5;

    config->log_level = LOG_INFO;
}

static obs_status_t apply_kv(obs_config_t *config, const char *key, const char *value)
{
    if (strcmp(key, "collector_url") == 0) {
        snprintf(config->collector_url, sizeof(config->collector_url), "%s", value);
    } else if (strcmp(key, "agent_token") == 0) {
        snprintf(config->agent_token, sizeof(config->agent_token), "%s", value);
    } else if (strcmp(key, "agent_id_path") == 0) {
        snprintf(config->agent_id_path, sizeof(config->agent_id_path), "%s", value);
    } else if (strcmp(key, "heartbeat_interval_secs") == 0) {
        config->heartbeat_interval_secs = (unsigned int)strtoul(value, NULL, 10);
    } else if (strcmp(key, "metrics_interval_secs") == 0) {
        config->metrics_interval_secs = (unsigned int)strtoul(value, NULL, 10);
    } else if (strcmp(key, "connect_timeout_secs") == 0) {
        config->connect_timeout_secs = (unsigned int)strtoul(value, NULL, 10);
    } else if (strcmp(key, "write_timeout_secs") == 0) {
        config->write_timeout_secs = (unsigned int)strtoul(value, NULL, 10);
    } else if (strcmp(key, "read_timeout_secs") == 0) {
        config->read_timeout_secs = (unsigned int)strtoul(value, NULL, 10);
    } else if (strcmp(key, "log_level") == 0) {
        if (strcmp(value, "trace") == 0) config->log_level = LOG_TRACE;
        else if (strcmp(value, "debug") == 0) config->log_level = LOG_DEBUG;
        else if (strcmp(value, "info") == 0) config->log_level = LOG_INFO;
        else if (strcmp(value, "warn") == 0) config->log_level = LOG_WARN;
        else if (strcmp(value, "error") == 0) config->log_level = LOG_ERROR;
        else {
            LOG_WARN_(COMPONENT, "unknown log_level '%s', keeping current", value);
        }
    } else {
        LOG_WARN_(COMPONENT, "unknown config key '%s' (ignored)", key);
    }

    return OBS_OK;
}

obs_status_t config_load(const char *path, obs_config_t *config)
{
    FILE *fp = fopen(path, "r");
    if (!fp) {
        LOG_WARN_(COMPONENT, "could not open config file '%s', using defaults", path);
        return OBS_ERR_IO;
    }

    char line[OBS_MAX_LINE];
    int line_no = 0;

    while (fgets(line, sizeof(line), fp)) {
        line_no++;

        char *trimmed = trim(line);
        if (trimmed[0] == '\0' || trimmed[0] == '#') {
            continue;
        }

        char *eq = strchr(trimmed, '=');
        if (!eq) {
            LOG_WARN_(COMPONENT, "%s:%d: malformed line (missing '='), skipping",
                       path, line_no);
            continue;
        }

        *eq = '\0';
        char *key = trim(trimmed);
        char *value = trim(eq + 1);

        if (key[0] == '\0') {
            LOG_WARN_(COMPONENT, "%s:%d: empty key, skipping", path, line_no);
            continue;
        }

        apply_kv(config, key, value);
    }

    fclose(fp);
    LOG_INFO_(COMPONENT, "loaded configuration from '%s'", path);
    return OBS_OK;
}
