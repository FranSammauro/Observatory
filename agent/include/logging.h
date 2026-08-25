#ifndef OBSERVER_LOGGING_H
#define OBSERVER_LOGGING_H

/*
 * Logging minimalista, sin dependencias externas.
 *
 * Reglas (informe tecnico, seccion 55):
 *  - Nunca loguear: token, password, Authorization header, secrets.
 *  - Cada linea incluye: timestamp, level, component.
 */

typedef enum {
    LOG_TRACE = 0,
    LOG_DEBUG,
    LOG_INFO,
    LOG_WARN,
    LOG_ERROR
} log_level_t;

/* Inicializa el logger. min_level filtra que se imprime. */
void log_init(log_level_t min_level);

/* Log con componente explicito, printf-style. */
void log_log(log_level_t level, const char *component, const char *fmt, ...);

#define LOG_TRACE_(component, ...) log_log(LOG_TRACE, component, __VA_ARGS__)
#define LOG_DEBUG_(component, ...) log_log(LOG_DEBUG, component, __VA_ARGS__)
#define LOG_INFO_(component, ...)  log_log(LOG_INFO,  component, __VA_ARGS__)
#define LOG_WARN_(component, ...)  log_log(LOG_WARN,  component, __VA_ARGS__)
#define LOG_ERROR_(component, ...) log_log(LOG_ERROR, component, __VA_ARGS__)

#endif /* OBSERVER_LOGGING_H */
