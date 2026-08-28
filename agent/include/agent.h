#ifndef OBSERVER_AGENT_H
#define OBSERVER_AGENT_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

/* Protocol / build versioning */
#define AGENT_VERSION      "0.2.0-phase2"
#define PROTOCOL_VERSION   1

/* Sizing limits (see docs/adr/0001-agent-language.md — budget-driven design) */
#define OBS_MAX_LINE           512
#define OBS_MAX_PATH            256
#define OBS_MAX_HOSTNAME         256
#define OBS_MAX_JSON_BUFFER     16384
#define OBS_MAX_INTERFACES        16
#define OBS_MAX_FILESYSTEMS       16

/* Error classification (see informe §56) */
typedef enum {
    OBS_OK = 0,
    OBS_ERR_IO,
    OBS_ERR_PARSE,
    OBS_ERR_PERMISSION,
    OBS_ERR_UNAVAILABLE,
    OBS_ERR_INVALID_ARG,
    OBS_ERR_OVERFLOW
} obs_status_t;

const char *obs_status_str(obs_status_t status);

#endif /* OBSERVER_AGENT_H */
