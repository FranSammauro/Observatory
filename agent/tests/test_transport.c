#include "transport.h"

#include <stdio.h>
#include <string.h>

static int g_failures = 0;

#define CHECK(cond, msg) do { \
    if (!(cond)) { \
        fprintf(stderr, "FAIL: %s (%s:%d)\n", msg, __FILE__, __LINE__); \
        g_failures++; \
    } \
} while (0)

static void test_parse_http_with_port(void)
{
    char host[256];
    uint16_t port;
    bool is_https;

    obs_status_t status = transport_parse_url("http://127.0.0.1:8080", host, sizeof(host),
                                                &port, &is_https);

    CHECK(status == OBS_OK, "parse should succeed");
    CHECK(strcmp(host, "127.0.0.1") == 0, "host");
    CHECK(port == 8080, "port");
    CHECK(is_https == false, "http scheme");
}

static void test_parse_https_default_port(void)
{
    char host[256];
    uint16_t port;
    bool is_https;

    obs_status_t status = transport_parse_url("https://collector.local", host, sizeof(host),
                                                &port, &is_https);

    CHECK(status == OBS_OK, "parse should succeed");
    CHECK(strcmp(host, "collector.local") == 0, "host");
    CHECK(port == 443, "default https port");
    CHECK(is_https == true, "https scheme");
}

static void test_parse_http_default_port_with_trailing_slash(void)
{
    char host[256];
    uint16_t port;
    bool is_https;

    obs_status_t status = transport_parse_url("http://example.com/", host, sizeof(host),
                                                &port, &is_https);

    CHECK(status == OBS_OK, "parse should succeed");
    CHECK(strcmp(host, "example.com") == 0, "host without trailing slash");
    CHECK(port == 80, "default http port");
}

static void test_parse_invalid_scheme(void)
{
    char host[256];
    uint16_t port;
    bool is_https;

    obs_status_t status = transport_parse_url("ftp://example.com", host, sizeof(host),
                                                &port, &is_https);
    CHECK(status == OBS_ERR_INVALID_ARG, "unsupported scheme should fail");
}

static void test_parse_invalid_port(void)
{
    char host[256];
    uint16_t port;
    bool is_https;

    obs_status_t status = transport_parse_url("http://example.com:99999", host, sizeof(host),
                                                &port, &is_https);
    CHECK(status == OBS_ERR_INVALID_ARG, "out-of-range port should fail");
}

static void test_parse_null_args(void)
{
    char host[256];
    uint16_t port;
    bool is_https;

    CHECK(transport_parse_url(NULL, host, sizeof(host), &port, &is_https) == OBS_ERR_INVALID_ARG,
           "null url rejected");
}

int main(void)
{
    test_parse_http_with_port();
    test_parse_https_default_port();
    test_parse_http_default_port_with_trailing_slash();
    test_parse_invalid_scheme();
    test_parse_invalid_port();
    test_parse_null_args();

    if (g_failures == 0) {
        printf("test_transport: all tests passed\n");
        return 0;
    }
    fprintf(stderr, "test_transport: %d failure(s)\n", g_failures);
    return 1;
}
