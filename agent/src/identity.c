#define _POSIX_C_SOURCE 200809L

#include "identity.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>
#include <ctype.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/stat.h>

#define COMPONENT "identity"

static bool is_valid_hex_id(const char *s, size_t len)
{
    if (len != OBS_AGENT_ID_LEN) {
        return false;
    }
    for (size_t i = 0; i < len; i++) {
        if (!isxdigit((unsigned char)s[i])) {
            return false;
        }
    }
    return true;
}

static obs_status_t try_read_existing(const char *path, char *out, size_t out_size)
{
    if (out_size < OBS_AGENT_ID_LEN + 1) {
        return OBS_ERR_INVALID_ARG;
    }

    FILE *fp = fopen(path, "r");
    if (!fp) {
        return OBS_ERR_IO;
    }

    char buffer[OBS_MAX_LINE];
    obs_status_t status = OBS_ERR_PARSE;

    if (fgets(buffer, sizeof(buffer), fp)) {
        size_t len = strlen(buffer);
        while (len > 0 && (buffer[len - 1] == '\n' || buffer[len - 1] == '\r')) {
            buffer[len - 1] = '\0';
            len--;
        }
        if (is_valid_hex_id(buffer, len)) {
            snprintf(out, out_size, "%s", buffer);
            status = OBS_OK;
        }
    }

    fclose(fp);
    return status;
}

static obs_status_t generate_random_id(char *out, size_t out_size)
{
    if (out_size < OBS_AGENT_ID_LEN + 1) {
        return OBS_ERR_INVALID_ARG;
    }

    unsigned char raw[OBS_AGENT_ID_LEN / 2];

    FILE *urandom = fopen("/dev/urandom", "rb");
    if (!urandom) {
        LOG_ERROR_(COMPONENT, "could not open /dev/urandom");
        return OBS_ERR_IO;
    }

    size_t read_bytes = fread(raw, 1, sizeof(raw), urandom);
    fclose(urandom);

    if (read_bytes != sizeof(raw)) {
        LOG_ERROR_(COMPONENT, "short read from /dev/urandom");
        return OBS_ERR_IO;
    }

    for (size_t i = 0; i < sizeof(raw); i++) {
        snprintf(out + (i * 2), 3, "%02x", raw[i]);
    }

    return OBS_OK;
}

static obs_status_t persist_id(const char *path, const char *id)
{
    /* O_EXCL evita pisar un id existente por una carrera entre el chequeo
     * de lectura y la escritura (dos instancias del agent arrancando a la
     * vez). Si ya existe, quien gano la carrera es la fuente de verdad. */
    int fd = open(path, O_WRONLY | O_CREAT | O_EXCL, 0600);
    if (fd < 0) {
        LOG_WARN_(COMPONENT,
            "could not create '%s' (may already exist from a concurrent start)", path);
        return OBS_ERR_IO;
    }

    FILE *fp = fdopen(fd, "w");
    if (!fp) {
        close(fd);
        return OBS_ERR_IO;
    }

    fprintf(fp, "%s\n", id);
    fclose(fp);

    LOG_INFO_(COMPONENT, "generated new agent identity, persisted to '%s'", path);
    return OBS_OK;
}

obs_status_t identity_resolve(const char *path, char *out, size_t out_size)
{
    if (!path || !out || out_size < OBS_AGENT_ID_LEN + 1) {
        return OBS_ERR_INVALID_ARG;
    }

    if (try_read_existing(path, out, out_size) == OBS_OK) {
        return OBS_OK;
    }

    obs_status_t status = generate_random_id(out, out_size);
    if (status != OBS_OK) {
        return status;
    }

    obs_status_t persist_status = persist_id(path, out);
    if (persist_status != OBS_OK) {
        /* No pudimos persistir (p.ej. otra instancia gano la carrera, o
         * el directorio no existe / no hay permisos). Intentamos releer:
         * si alguien mas ya lo escribio, usamos ese id para no terminar
         * con dos identidades distintas para el mismo host. */
        if (try_read_existing(path, out, out_size) == OBS_OK) {
            return OBS_OK;
        }
        LOG_WARN_(COMPONENT,
            "could not persist identity to '%s', using in-memory id for this run only", path);
        /* out ya tiene el id generado en memoria; lo usamos igual para
         * esta ejecucion, aunque no sobreviva a un reinicio. */
    }

    return OBS_OK;
}
