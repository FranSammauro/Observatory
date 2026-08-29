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

# --- tests / fuzzing ---
TEST_CFLAGS = $(CFLAGS_BASE) -g -O0
TEST_BINS = tests/test_cpu tests/test_memory tests/test_config \
            tests/test_disk tests/test_network tests/test_filesystem \
            tests/test_uptime tests/test_process tests/test_retry \
            tests/test_transport tests/test_identity
FUZZ_BIN = tests/fuzz

.PHONY: all clean test debug sanitize fuzz

all: $(BIN)

$(BIN): $(OBJ)
	$(CC) $(CFLAGS) -o $@ $(OBJ) $(LDFLAGS)

%.o: %.c
	$(CC) $(CFLAGS) -c -o $@ $<

debug: CFLAGS = $(CFLAGS_BASE) -g -O0
debug: clean $(BIN)

# --- sanitizers (Fase 8, bloque 8.3) ---
# ASan (memoria) + UBSan (undefined) + LSan (leaks, vía ASAN_OPTIONS)
# sobre el binario, los unit tests y el harness de fuzzing. La deteccion
# de leaks la pide ASan en Linux al salir; detect_leaks=1 la fuerza.
#   make sanitize            # build sanitizado + corre tests + fuzz
#   ASAN_OPTIONS=detect_leaks=1 ./tests/fuzz 200000 42   # run a mano
SANITIZE = -g -O0 -fsanitize=address,undefined -fno-omit-frame-pointer
SANITIZE_LDFLAGS = -fsanitize=address,undefined

# Cocina con variables por-target: sanitize recompila el binario y los
# tests con los sanitizers habilitados. Como su prereq fuerza a
# recompilar siempre (target `force` phony), los flags toman efecto
# aunque un binario viejo exista.
sanitize: CFLAGS = $(CFLAGS_BASE) $(SANITIZE)
sanitize: TEST_CFLAGS = $(CFLAGS_BASE) $(SANITIZE)
sanitize: LDFLAGS += $(SANITIZE_LDFLAGS)
# `clean` como prereq phony borra los .o y bins; al noexistir, make los
# reconstruye aqui mismo ya con los flags sanitizados (los flags
# por-target aplican a los prereqs no anidados con su propio override).
sanitize: clean $(BIN) $(TEST_BINS) $(FUZZ_BIN)
	@echo "--- running unit tests under ASan+UBSan+LSan ---"
	@for t in $(TEST_BINS); do ASAN_OPTIONS=detect_leaks=1 ./$$t || exit 1; done
	@echo "--- running fuzz harness under ASan+UBSan+LSan ---"
	@ASAN_OPTIONS=detect_leaks=1 ./$(FUZZ_BIN)

# --- fuzzing (Fase 8, bloque 8.3) ---
# Harness deterministico (tests/fuzz.c): corpus + mutaciones xorshift,
# sin clang/libFuzzer. Alias del binario con run default.
fuzz: $(FUZZ_BIN)
	./$(FUZZ_BIN) 200000

# --- tests ---
test: $(TEST_BINS)
	@echo "--- running unit tests ---"
	@for t in $(TEST_BINS); do ./$$t || exit 1; done

tests/test_%: tests/test_%.c $(LIB_OBJ)
	$(CC) $(TEST_CFLAGS) -o $@ $< $(LIB_OBJ) -lm $(LDFLAGS)

tests/fuzz: tests/fuzz.c $(LIB_OBJ)
	$(CC) $(TEST_CFLAGS) -o $@ $< $(LIB_OBJ) -lm $(LDFLAGS)

clean:
	rm -f $(OBJ) $(BIN) $(TEST_BINS) $(FUZZ_BIN)
