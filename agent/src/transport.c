#define _POSIX_C_SOURCE 200809L

#include "transport.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <errno.h>
#include <unistd.h>
#include <fcntl.h>
#include <poll.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <sys/time.h>
#include <netdb.h>

#define COMPONENT "transport"
#define OBS_MAX_HOST 256
#define OBS_MAX_RESPONSE 4096

/*
 * Parsea "http://host[:port]" o "https://host[:port]". Expuesto (no
 * static) para poder testearlo unitariamente sin abrir sockets.
 */
obs_status_t transport_parse_url(const char *base_url, char *host, size_t host_size,
                                  uint16_t *port, bool *is_https)
{
    if (!base_url || !host || host_size == 0 || !port || !is_https) {
        return OBS_ERR_INVALID_ARG;
    }

    const char *rest;
    if (strncmp(base_url, "https://", 8) == 0) {
        *is_https = true;
        rest = base_url + 8;
        *port = 443;
    } else if (strncmp(base_url, "http://", 7) == 0) {
        *is_https = false;
        rest = base_url + 7;
        *port = 80;
    } else {
        return OBS_ERR_INVALID_ARG;
    }

    if (*rest == '\0') {
        return OBS_ERR_INVALID_ARG;
    }

    const char *colon = strchr(rest, ':');
    const char *slash = strchr(rest, '/');

    size_t host_len;
    if (colon && (!slash || colon < slash)) {
        host_len = (size_t)(colon - rest);
        long parsed_port = strtol(colon + 1, NULL, 10);
        if (parsed_port <= 0 || parsed_port > 65535) {
            return OBS_ERR_INVALID_ARG;
        }
        *port = (uint16_t)parsed_port;
    } else if (slash) {
        host_len = (size_t)(slash - rest);
    } else {
        host_len = strlen(rest);
    }

    if (host_len == 0 || host_len >= host_size) {
        return OBS_ERR_INVALID_ARG;
    }

    memcpy(host, rest, host_len);
    host[host_len] = '\0';

    return OBS_OK;
}

static int connect_with_timeout(const char *host, uint16_t port, unsigned int timeout_secs)
{
    char port_str[8];
    snprintf(port_str, sizeof(port_str), "%u", port);

    struct addrinfo hints;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;

    struct addrinfo *result = NULL;
    int gai_status = getaddrinfo(host, port_str, &hints, &result);
    if (gai_status != 0) {
        LOG_ERROR_(COMPONENT, "DNS/address resolution failed for '%s': %s",
                    host, gai_strerror(gai_status));
        return -1;
    }

    int fd = -1;

    for (struct addrinfo *rp = result; rp != NULL; rp = rp->ai_next) {
        fd = socket(rp->ai_family, rp->ai_socktype, rp->ai_protocol);
        if (fd < 0) {
            continue;
        }

        int flags = fcntl(fd, F_GETFL, 0);
        fcntl(fd, F_SETFL, flags | O_NONBLOCK);

        int connect_rc = connect(fd, rp->ai_addr, rp->ai_addrlen);
        if (connect_rc == 0) {
            fcntl(fd, F_SETFL, flags);
            break;
        }

        if (errno != EINPROGRESS) {
            close(fd);
            fd = -1;
            continue;
        }

        struct pollfd pfd = { .fd = fd, .events = POLLOUT };
        int poll_rc = poll(&pfd, 1, (int)(timeout_secs * 1000));

        if (poll_rc <= 0) {
            /* timeout o error de poll */
            close(fd);
            fd = -1;
            continue;
        }

        int so_error = 0;
        socklen_t len = sizeof(so_error);
        getsockopt(fd, SOL_SOCKET, SO_ERROR, &so_error, &len);

        if (so_error != 0) {
            close(fd);
            fd = -1;
            continue;
        }

        fcntl(fd, F_SETFL, flags);
        break;
    }

    freeaddrinfo(result);
    return fd;
}

static obs_status_t send_all(int fd, const char *data, size_t len, unsigned int timeout_secs)
{
    struct timeval tv = { .tv_sec = timeout_secs, .tv_usec = 0 };
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));

    size_t sent = 0;
    while (sent < len) {
        ssize_t n = send(fd, data + sent, len - sent, 0);
        if (n < 0) {
            if (errno == EAGAIN || errno == EWOULDBLOCK) {
                LOG_ERROR_(COMPONENT, "write timeout after %us", timeout_secs);
            } else {
                LOG_ERROR_(COMPONENT, "send() failed: %s", strerror(errno));
            }
            return OBS_ERR_IO;
        }
        sent += (size_t)n;
    }

    return OBS_OK;
}

static obs_status_t recv_response(int fd, char *buffer, size_t buffer_size,
                                    unsigned int timeout_secs, size_t *out_len)
{
    struct timeval tv = { .tv_sec = timeout_secs, .tv_usec = 0 };
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));

    size_t total = 0;
    while (total < buffer_size - 1) {
        ssize_t n = recv(fd, buffer + total, buffer_size - 1 - total, 0);
        if (n < 0) {
            if (errno == EAGAIN || errno == EWOULDBLOCK) {
                LOG_ERROR_(COMPONENT, "read timeout after %us", timeout_secs);
                return OBS_ERR_IO;
            }
            LOG_ERROR_(COMPONENT, "recv() failed: %s", strerror(errno));
            return OBS_ERR_IO;
        }
        if (n == 0) {
            /* servidor cerro la conexion (Connection: close en el request) */
            break;
        }
        total += (size_t)n;
    }

    buffer[total] = '\0';
    *out_len = total;
    return OBS_OK;
}

/*
 * Parsea la linea de status HTTP de la respuesta ("HTTP/1.1 200 OK...")
 * en el codigo de status. Expuesto (no static) para fuzz/tests.
 */
bool transport_parse_status_line(const char *response, int *status_code)
{
    if (!response || !status_code) {
        return false;
    }
    /* "HTTP/1.1 200 OK\r\n..." */
    return sscanf(response, "HTTP/%*d.%*d %d", status_code) == 1;
}

obs_status_t transport_post(const transport_config_t *config,
                             const char *base_url,
                             const char *path,
                             const char *token,
                             const char *body,
                             transport_result_t *result)
{
    if (!config || !base_url || !path || !body || !result) {
        return OBS_ERR_INVALID_ARG;
    }

    result->got_response = false;
    result->status_code = 0;

    char host[OBS_MAX_HOST];
    uint16_t port;
    bool is_https;

    obs_status_t parse_status = transport_parse_url(base_url, host, sizeof(host), &port, &is_https);
    if (parse_status != OBS_OK) {
        LOG_ERROR_(COMPONENT, "invalid collector_url '%s'", base_url);
        return parse_status;
    }

    if (is_https) {
        /* El agente no implementa TLS. Ver docs/adr/0002-transport-protocol.md.
         * Se rechaza explicitamente en lugar de degradar silenciosamente. */
        LOG_ERROR_(COMPONENT,
            "collector_url usa https:// pero el agente no implementa TLS. "
            "Usar http:// dentro de la red de confianza.");
        return OBS_ERR_UNAVAILABLE;
    }

    int fd = connect_with_timeout(host, port, config->connect_timeout_secs);
    if (fd < 0) {
        LOG_ERROR_(COMPONENT, "could not connect to %s:%u", host, port);
        return OBS_ERR_UNAVAILABLE;
    }

    char request[OBS_MAX_JSON_BUFFER + 512];
    int has_token = (token && token[0] != '\0');

    int written = snprintf(request, sizeof(request),
        "POST %s HTTP/1.1\r\n"
        "Host: %s\r\n"
        "User-Agent: observer-agent/%s\r\n"
        "Content-Type: application/json\r\n"
        "%s%s%s"
        "Content-Length: %zu\r\n"
        "Connection: close\r\n"
        "\r\n"
        "%s",
        path, host, AGENT_VERSION,
        has_token ? "Authorization: Bearer " : "",
        has_token ? token : "",
        has_token ? "\r\n" : "",
        strlen(body), body);

    if (written < 0 || (size_t)written >= sizeof(request)) {
        close(fd);
        return OBS_ERR_OVERFLOW;
    }

    obs_status_t send_status = send_all(fd, request, (size_t)written, config->write_timeout_secs);
    if (send_status != OBS_OK) {
        close(fd);
        return send_status;
    }

    char response[OBS_MAX_RESPONSE];
    size_t response_len = 0;
    obs_status_t recv_status = recv_response(fd, response, sizeof(response),
                                               config->read_timeout_secs, &response_len);
    close(fd);

    if (recv_status != OBS_OK) {
        return recv_status;
    }

    if (response_len == 0) {
        LOG_ERROR_(COMPONENT, "empty response from collector");
        return OBS_ERR_IO;
    }

    int status_code = 0;
    if (!transport_parse_status_line(response, &status_code)) {
        LOG_ERROR_(COMPONENT, "could not parse HTTP status line from response");
        return OBS_ERR_PARSE;
    }

    result->got_response = true;
    result->status_code = status_code;

    return OBS_OK;
}
