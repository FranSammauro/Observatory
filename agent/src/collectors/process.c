#define _POSIX_C_SOURCE 200809L

#include "collectors/process.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>
#include <ctype.h>
#include <dirent.h>

#define COMPONENT "collector.process"
#define PROC_ROOT "/proc"

char process_normalize_state(char raw_state)
{
    switch (raw_state) {
        case 'R': return 'R'; /* running */
        case 'S': /* interruptible sleep */
        case 'D': /* uninterruptible sleep (I/O) */
            return 'S';
        case 'T': /* stopped (signal) */
        case 't': /* tracing stop */
            return 'T';
        case 'Z': return 'Z'; /* zombie */
        default:  return '?'; /* idle kernel threads, dead, etc. - contados en total pero no en un bucket */
    }
}

static bool is_pid_dir(const char *name)
{
    if (name[0] == '\0') {
        return false;
    }
    for (const char *c = name; *c; c++) {
        if (!isdigit((unsigned char)*c)) {
            return false;
        }
    }
    return true;
}

static bool read_pid_state(const char *pid_str, char *state_out)
{
    char path[OBS_MAX_PATH];
    snprintf(path, sizeof(path), "%s/%s/stat", PROC_ROOT, pid_str);

    FILE *fp = fopen(path, "r");
    if (!fp) {
        return false; /* proceso pudo haber terminado entre el listado y la lectura */
    }

    char line[OBS_MAX_LINE];
    bool ok = false;

    if (fgets(line, sizeof(line), fp)) {
        /* Formato: "<pid> (<comm>) <state> ...". <comm> puede contener
         * espacios y parentesis, asi que buscamos el ULTIMO ')' antes de
         * asumir donde empieza el campo de estado. */
        char *last_paren = strrchr(line, ')');
        if (last_paren && *(last_paren + 1) != '\0') {
            char *cursor = last_paren + 1;
            while (*cursor == ' ') {
                cursor++;
            }
            if (*cursor != '\0') {
                *state_out = *cursor;
                ok = true;
            }
        }
    }

    fclose(fp);
    return ok;
}

obs_status_t process_collect(process_metrics_t *out)
{
    if (!out) {
        return OBS_ERR_INVALID_ARG;
    }

    memset(out, 0, sizeof(*out));

    DIR *dir = opendir(PROC_ROOT);
    if (!dir) {
        LOG_ERROR_(COMPONENT, "failed to open '%s'", PROC_ROOT);
        return OBS_ERR_IO;
    }

    struct dirent *entry;
    while ((entry = readdir(dir)) != NULL) {
        if (!is_pid_dir(entry->d_name)) {
            continue;
        }

        char state = '\0';
        if (!read_pid_state(entry->d_name, &state)) {
            continue;
        }

        out->total++;

        switch (process_normalize_state(state)) {
            case 'R': out->running++; break;
            case 'S': out->sleeping++; break;
            case 'T': out->stopped++; break;
            case 'Z': out->zombie++; break;
            default: break; /* contado en total, sin bucket especifico */
        }
    }

    closedir(dir);
    return OBS_OK;
}
