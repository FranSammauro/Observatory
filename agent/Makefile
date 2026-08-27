CC ?= cc

CFLAGS_BASE = -std=c11 -Wall -Wextra -Wpedantic -Wconversion -Wshadow -Iinclude
CFLAGS ?= $(CFLAGS_BASE) -O2
LDFLAGS ?=

LIB_SRC = src/agent.c \
          src/config.c \
          src/logging.c \
          src/protocol.c \
          src/transport.c \
          src/retry.c \
          src/identity.c \
          src/collectors/cpu.c \
          src/collectors/memory.c \
          src/collectors/disk.c \
          src/collectors/network.c \
          src/collectors/filesystem.c \
          src/collectors/uptime.c \
          src/collectors/process.c \
          src/collectors/temperature.c

SRC = src/main.c $(LIB_SRC)

OBJ = $(SRC:.c=.o)
LIB_OBJ = $(LIB_SRC:.c=.o)

BIN = observer-agent

.PHONY: all clean test debug sanitize

all: $(BIN)

$(BIN): $(OBJ)
	$(CC) $(CFLAGS) -o $@ $(OBJ) $(LDFLAGS)

%.o: %.c
	$(CC) $(CFLAGS) -c -o $@ $<

debug: CFLAGS = $(CFLAGS_BASE) -g -O0
debug: clean $(BIN)

sanitize: CFLAGS = $(CFLAGS_BASE) -g -O0 -fsanitize=address,undefined
sanitize: LDFLAGS += -fsanitize=address,undefined
sanitize: clean $(BIN)

# --- tests ---
TEST_CFLAGS = $(CFLAGS_BASE) -g -O0
TEST_BINS = tests/test_cpu tests/test_memory tests/test_config \
            tests/test_disk tests/test_network tests/test_filesystem \
            tests/test_uptime tests/test_process tests/test_retry \
            tests/test_transport tests/test_identity

test: $(TEST_BINS)
	@echo "--- running unit tests ---"
	@for t in $(TEST_BINS); do ./$$t || exit 1; done

tests/test_%: tests/test_%.c $(LIB_OBJ)
	$(CC) $(TEST_CFLAGS) -o $@ $< $(LIB_OBJ) -lm

clean:
	rm -f $(OBJ) $(BIN) $(TEST_BINS)
