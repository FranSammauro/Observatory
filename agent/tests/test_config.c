#include "config.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>
#include <unistd.h>

static int g_failures = 0;

#define CHECK(cond, msg) do { \
    if (!(cond)) { \
        fprintf(stderr, "FAIL: %s (%s:%d)\n", msg, __FILE__, __LINE__); \
        g_failures++; \
    } \
} while (0)

static void test_defaults(void)
{
    obs_config_t config;
    config_set_defaults(&config);

    CHECK(config.heartbeat_interval_secs == 5, "default heartbeat interval");
    CHECK(config.metrics_interval_secs == 10, "default metrics interval");
    CHECK(strlen(config.agent_token) == 0, "default token is empty");
}

static void test_load_from_file(void)
{
    const char *path = "/tmp/observer_test_config.conf";
    FILE *fp = fopen(path, "w");
    CHECK(fp != NULL, "could create temp config file");
    if (!fp) return;

    fprintf(fp,
        "# comentario\n"
        "collector_url = https://collector.local:8443\n"
        "agent_token=super-secret-token\n"
        "metrics_interval_secs = 30\n"
        "log_level = debug\n"
        "\n"
        "unknown_key = ignored\n");
    fclose(fp);

    obs_config_t config;
    config_set_defaults(&config);
    obs_status_t status = config_load(path, &config);

    CHECK(status == OBS_OK, "config_load should succeed");
    CHECK(strcmp(config.collector_url, "https://collector.local:8443") == 0, "collector_url overridden");
    CHECK(strcmp(config.agent_token, "super-secret-token") == 0, "agent_token overridden");
    CHECK(config.metrics_interval_secs == 30, "metrics_interval_secs overridden");
    CHECK(config.log_level == LOG_DEBUG, "log_level overridden");
    /* heartbeat no estaba en el archivo -> debe conservar el default */
    CHECK(config.heartbeat_interval_secs == 5, "heartbeat_interval_secs keeps default");

    unlink(path);
}

static void test_load_missing_file_keeps_defaults(void)
{
    obs_config_t config;
    config_set_defaults(&config);
    obs_status_t status = config_load("/tmp/does_not_exist_observer.conf", &config);

    CHECK(status == OBS_ERR_IO, "missing file returns OBS_ERR_IO");
    CHECK(config.metrics_interval_secs == 10, "defaults preserved on missing file");
}

int main(void)
{
    log_init(LOG_ERROR); /* silenciar logs durante el test */

    test_defaults();
    test_load_from_file();
    test_load_missing_file_keeps_defaults();

    if (g_failures == 0) {
        printf("test_config: all tests passed\n");
        return 0;
    }

    fprintf(stderr, "test_config: %d failure(s)\n", g_failures);
    return 1;
}
