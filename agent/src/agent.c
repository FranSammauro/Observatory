#include "agent.h"

const char *obs_status_str(obs_status_t status)
{
    switch (status) {
        case OBS_OK:               return "OK";
        case OBS_ERR_IO:           return "ERR_IO";
        case OBS_ERR_PARSE:        return "ERR_PARSE";
        case OBS_ERR_PERMISSION:   return "ERR_PERMISSION";
        case OBS_ERR_UNAVAILABLE:  return "ERR_UNAVAILABLE";
        case OBS_ERR_INVALID_ARG:  return "ERR_INVALID_ARG";
        case OBS_ERR_OVERFLOW:     return "ERR_OVERFLOW";
        default:                   return "ERR_UNKNOWN";
    }
}
