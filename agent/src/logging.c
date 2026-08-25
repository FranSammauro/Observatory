#define _POSIX_C_SOURCE 200809L

#include "logging.h"

#include <stdio.h>
#include <stdarg.h>
#include <time.h>

static log_level_t g_min_level = LOG_INFO;

static const char *level_name(log_level_t level)
{
    switch (level) {
        case LOG_TRACE: return "TRACE";
        case LOG_DEBUG: return "DEBUG";
        case LOG_INFO:  return "INFO";
        case LOG_WARN:  return "WARN";
        case LOG_ERROR: return "ERROR";
        default:        return "?????";
    }
}

void log_init(log_level_t min_level)
{
    g_min_level = min_level;
}

void log_log(log_level_t level, const char *component, const char *fmt, ...)
{
    if (level < g_min_level) {
        return;
    }

    time_t now = time(NULL);
    struct tm tm_utc;
    gmtime_r(&now, &tm_utc);

    char timestamp[32];
    strftime(timestamp, sizeof(timestamp), "%Y-%m-%dT%H:%M:%SZ", &tm_utc);

    FILE *out = (level >= LOG_WARN) ? stderr : stdout;

    fprintf(out, "%s [%-5s] %-10s ", timestamp, level_name(level), component);

    va_list args;
    va_start(args, fmt);
    vfprintf(out, fmt, args);
    va_end(args);

    fprintf(out, "\n");
}
