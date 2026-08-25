CC ?= cc

CFLAGS_BASE = -std=c11 -Wall -Wextra -Wpedantic -Wconversion -Wshadow -Iinclude
CFLAGS ?= $(CFLAGS_BASE) -O2
LDFLAGS ?=

SRC = src/main.c \
      src/agent.c \
      src/config.c \
      src/logging.c \
      src/protocol.c \
      src/collectors/cpu.c \
      src/collectors/memory.c

LIB_SRC = src/agent.c \
          src/config.c \
          src/logging.c \
          src/protocol.c \
          src/collectors/cpu.c \
          src/collectors/memory.c

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
TEST_BINS = tests/test_cpu tests/test_memory tests/test_config

test: $(TEST_BINS)
	@echo "--- running unit tests ---"
	@for t in $(TEST_BINS); do ./$$t || exit 1; done

tests/test_cpu: tests/test_cpu.c $(LIB_OBJ)
	$(CC) $(TEST_CFLAGS) -o $@ tests/test_cpu.c $(LIB_OBJ) -lm

tests/test_memory: tests/test_memory.c $(LIB_OBJ)
	$(CC) $(TEST_CFLAGS) -o $@ tests/test_memory.c $(LIB_OBJ) -lm

tests/test_config: tests/test_config.c $(LIB_OBJ)
	$(CC) $(TEST_CFLAGS) -o $@ tests/test_config.c $(LIB_OBJ) -lm

clean:
	rm -f $(OBJ) $(BIN) $(TEST_BINS)
