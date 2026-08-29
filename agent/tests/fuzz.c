/*
 * Harness de fuzzing minimo (Fase 8, bloque 8.3).
 *
 * Sin libFuzzer (clang) ni dependencias externas: un generador
 * deterministico xorshift64 arma el corpus (entradas interesantes
 * reales) y lo muta (bit flips, inserciones/borrados, bytes
 * "interesantes", truncados, lineas largas) contra los parsers puros
 * del agent:
 *
 *   - transport_parse_url            (parser de URL del transport)
 *   - transport_parse_status_line    (parser de la linea de status HTTP)
 *   - config_parse_text              (parser de la config clave=valor)
 *
 * No hay assertions por "resultado correcto": el parser decide. Lo que
 * el harness verifica es ausencia de crash/UB/leaks -- por eso se corre
 * bajo ASan + UBSan + LSan (make sanitize). Un fallo de invariante
 * (buffers fuera de rango, status sin escribir) tambien falla.
 *
 * Uso:
 *   tests/fuzz [iteraciones] [seed]
 * con defaults 200000 y 1. Mismo seed => misma secuencia (reproducible).
 */

#include "config.h"
#include "transport.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define DEFAULT_ITERATIONS 200000u
#define INPUT_MAX 384
#define HOST_BUF 256

static uint64_t g_state;

static uint64_t rnd(void)
{
    g_state ^= g_state << 13;
    g_state ^= g_state >> 7;
    g_state ^= g_state << 17;
    return g_state;
}

/* Bytes "interesantes" para byte flips: cerca de limites, NUL, \n, '=' */
static const unsigned char interesting_ascii[] = {
    0x00, '\n', '\r', ' ', '=', ':', '/', '.', '-', '+', 'e', 'E',
    '0', '1', '5', '9', '"', '{', '}', '[', ']', ',', '#',
};

static unsigned char interesting_byte(void)
{
    return interesting_ascii[rnd() % sizeof(interesting_ascii)];
}

/* Corpus inicial: entradas reales/golosas tomadas de los tests y
 * casos borde conocidos del transporte y la config. */
static const char *corpus[] = {
    /* URLs validas y borde */
    "http://127.0.0.1:8080",
    "http://127.0.0.1",
    "https://collector.local",
    "https://colector.local:443",
    "http://host",
    "http://host:80",
    "http://host:65535",
    "http://host:65536",
    "http://host:0",
    "http://host:-1",
    "http://:8080",
    "http://host:",
    "http:///path",
    "http://",
    "https://",
    "ftp://host:8080",
    "host:8080",
    "http://ho:st:8080",
    "http://host:8080/path",
    "http://host:8080/path?q=1",
    "http://host:99999999999999",
    "http://host:1844674407370955161722",
    /* config valida (formato real del /etc/observer/agent.conf) */
    "collector_url = http://127.0.0.1:8080\n"
    "agent_token = secret\n"
    "heartbeat_interval_secs = 5\n"
    "metrics_interval_secs = 10\n"
    "log_level = debug\n",
    "collector_url=http://mia.observatorio:8080\nagent_token=k7\n",
    /* lineas de status HTTP */
    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
    "HTTP/1.1 404 Not Found\r\n",
    "HTTP/1.1 200",
    "HTTP/1.0 100 Continue",
    "HTTP/2 200",
    "HTTPS/1.1 200 OK",
    "200 OK",
    "\r\nHTTP/1.1 200 OK\r\n",
    /* basura */
    "\xff\xfe\x00\x01",
    ">>>>>>",
};

static size_t corpus_len(void)
{
    return sizeof(corpus) / sizeof(corpus[0]);
}

/* Rellena `out` (INPUT_MAX+1 bytes, NUL terminado) con una entrada:
 * sin mutacion (corpus directo), mutada, o bytes aleatorios puros. */
static void generate_input(char *out)
{
    size_t n;
    unsigned int mode = (unsigned int)(rnd() % 4);

    if (mode == 3) {
        n = rnd() % (INPUT_MAX + 1);
        for (size_t i = 0; i < n; i++) {
            out[i] = (char)(rnd() & 0xff);
        }
    } else {
        const char *base = corpus[rnd() % corpus_len()];
        n = strlen(base);
        size_t cap = INPUT_MAX;
        if (n > cap) n = cap;
        memcpy(out, base, n);

        unsigned int mutations = 1u + (unsigned int)(rnd() % 8);
        for (unsigned int m = 0; m < mutations; m++) {
            unsigned int kind = (unsigned int)(rnd() % 5);
            if (n == 0) kind = 1;
            switch (kind) {
            case 0: /* bit flip en un byte */
                if (n > 0) {
                    size_t at = rnd() % n;
                    out[at] = (char)((unsigned char)out[at] ^ (1u << (rnd() % 8)));
                }
                break;
            case 1: /* byte interesante */
                if (n < cap) {
                    size_t at = (n > 0) ? rnd() % (n + 1) : 0;
                    memmove(out + at + 1, out + at, n - at);
                    out[at] = (char)interesting_byte();
                    n++;
                }
                break;
            case 2: /* borrar un byte */
                if (n > 0) {
                    size_t at = rnd() % n;
                    for (size_t i = at; i + 1 < n; i++) {
                        out[i] = out[i + 1];
                    }
                    n--;
                }
                break;
            case 3: /* sobreescribir un byte con uno interesante */
                if (n > 0) {
                    out[rnd() % n] = (char)interesting_byte();
                }
                break;
            case 4: /* truncar a la mitad */
                if (n > 0) {
                    n = n / 2;
                }
                break;
            }
        }
        if (n > 0 && rnd() % 16 == 0) {
            /* linea larga: duplicar el contenido hasta el tope */
            size_t grow = INPUT_MAX;
            if (grow < n) grow = n;
            for (size_t i = n; i < grow; i++) {
                out[i] = out[i % n];
            }
            n = grow;
        }
    }

    out[n] = '\0';
}

static void fuzz_url(const char *input)
{
    char host[HOST_BUF];
    uint16_t port;
    bool is_https;

    obs_status_t status = transport_parse_url(input, host, sizeof(host), &port, &is_https);

    if (status == OBS_OK) {
        /* El parser prometio un host NUL-terminado dentro del buffer:
         * debe quedar <= 255 chars y terminar en '\0'. */
        size_t len = strlen(host);
        if (len >= sizeof(host)) {
            fprintf(stderr, "HUECCO: host fuera de rango (%zu)\n", len);
            exit(1);
        }
        if (host[sizeof(host) - 1] != '\0') {
            fprintf(stderr, "HUECO: host no NUL terminado\n");
            exit(1);
        }
    }
    (void)port;
    (void)is_https;
}

static void fuzz_status_line(const char *input)
{
    int status_code = 0;

    (void)transport_parse_status_line(input, &status_code);
    (void)status_code;
}

static void fuzz_config(const char *input)
{
    obs_config_t cfg;

    config_set_defaults(&cfg);
    config_parse_text(input, &cfg);

    /* Invariante: snprintf nunca volco mas que el buffer (NUL dentro). */
    if (cfg.collector_url[OBS_MAX_LINE - 1] != '\0') {
        fprintf(stderr, "HUECO: collector_url desbordado\n");
        exit(1);
    }
    if (cfg.agent_token[OBS_MAX_LINE - 1] != '\0') {
        fprintf(stderr, "HUECO: agent_token desbordado\n");
        exit(1);
    }
}

int main(int argc, char **argv)
{
    unsigned int iterations = DEFAULT_ITERATIONS;
    if (argc > 1) {
        iterations = (unsigned int)strtoul(argv[1], NULL, 10);
    }
    uint64_t seed = (argc > 2) ? strtoull(argv[2], NULL, 10) : 1u;
    g_state = seed;

    /* Los parsers loguean (LOG_WARN_ / LOG_ERROR_) por consola; nada de
     * esa salida es un fallo del fuzz. Solo los exit(1) por invariante o
     * el crash del sanitizer cuentan como hallazgo. */

    char input[INPUT_MAX + 1];
    for (unsigned int i = 0; i < iterations; i++) {
        generate_input(input);
        fuzz_url(input);
        fuzz_status_line(input);
        fuzz_config(input);
    }

    printf("fuzz: ok (%u iteraciones, seed %llu)\n",
           iterations, (unsigned long long)seed);
    return 0;
}