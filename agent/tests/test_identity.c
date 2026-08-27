#define _POSIX_C_SOURCE 200809L

#include "identity.h"
#include "logging.h"

#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/stat.h>

static int g_failures = 0;

#define CHECK(cond, msg) do { \
    if (!(cond)) { \
        fprintf(stderr, "FAIL: %s (%s:%d)\n", msg, __FILE__, __LINE__); \
        g_failures++; \
    } \
} while (0)

static void test_generates_and_persists(void)
{
    const char *path = "/tmp/observer_test_agent_id_a";
    unlink(path);

    char id[64];
    obs_status_t status = identity_resolve(path, id, sizeof(id));

    CHECK(status == OBS_OK, "identity_resolve should succeed");
    CHECK(strlen(id) == OBS_AGENT_ID_LEN, "id has expected length");

    struct stat st;
    CHECK(stat(path, &st) == 0, "id file was created");
    CHECK((st.st_mode & 0777) == 0600, "id file has 0600 permissions");

    unlink(path);
}

static void test_reuses_existing_id(void)
{
    const char *path = "/tmp/observer_test_agent_id_b";
    unlink(path);

    char first_id[64];
    CHECK(identity_resolve(path, first_id, sizeof(first_id)) == OBS_OK, "first resolve succeeds");

    char second_id[64];
    CHECK(identity_resolve(path, second_id, sizeof(second_id)) == OBS_OK, "second resolve succeeds");

    CHECK(strcmp(first_id, second_id) == 0, "second call reuses the persisted id");

    unlink(path);
}

static void test_ignores_corrupt_existing_file(void)
{
    const char *path = "/tmp/observer_test_agent_id_c";
    FILE *fp = fopen(path, "w");
    CHECK(fp != NULL, "could create corrupt file");
    if (fp) {
        fprintf(fp, "not-valid-hex!!\n");
        fclose(fp);
    }

    char id[64];
    obs_status_t status = identity_resolve(path, id, sizeof(id));

    /* El archivo existe pero con contenido invalido -> no se puede
     * persistir un id nuevo encima (O_EXCL falla), asi que el agent
     * debe seguir funcionando con un id en memoria para esta corrida. */
    CHECK(status == OBS_OK, "should not fail even if existing file is corrupt");
    CHECK(strlen(id) == OBS_AGENT_ID_LEN, "still produces a valid-looking id");

    unlink(path);
}

static void test_null_args(void)
{
    char id[64];
    CHECK(identity_resolve(NULL, id, sizeof(id)) == OBS_ERR_INVALID_ARG, "null path rejected");
    CHECK(identity_resolve("/tmp/x", NULL, sizeof(id)) == OBS_ERR_INVALID_ARG, "null out rejected");
    CHECK(identity_resolve("/tmp/x", id, 4) == OBS_ERR_INVALID_ARG, "too-small buffer rejected");
}

int main(void)
{
    log_init(LOG_ERROR); /* silenciar warnings esperados durante el test */

    test_generates_and_persists();
    test_reuses_existing_id();
    test_ignores_corrupt_existing_file();
    test_null_args();

    if (g_failures == 0) {
        printf("test_identity: all tests passed\n");
        return 0;
    }
    fprintf(stderr, "test_identity: %d failure(s)\n", g_failures);
    return 1;
}
