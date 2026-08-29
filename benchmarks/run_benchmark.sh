#!/usr/bin/env bash
# Benchmark reproducible del Collector (Fase 8, bloque 8.3)
#
# Levanta el Collector (build release) contra una base PostgreSQL
# DEDICADA y efimera y lo golpea con N agents simulados
# (heartbeats + samples) durante T segundos, midiendo throughput,
# latencia y persistencia.
#
# La meta es una *referencia reproducible*, no un tuning fino: el script
# se diseno para poder correrse igual en el hardware de referencia
# (Pentium M + Alpine Linux) que en una workstation, y dejar la huella
# (fingerprint) del entorno donde se midio.
#
# Uso:
#   ./benchmarks/run_benchmark.sh [duration_secs] [n_agents] [interval_secs]
#   # defaults: 30 10 1
#
# Requiere:
#   - docker (contenedor `observer-postgres`) o una DATABASE_URL aparte
#   - curl, jq, git, cargo
# Variables (todas opcionales):
#   DATABASE_URL   postgres://observer:observer@127.0.0.1:55432/bench
#   BENCH_AGENTS, BENCH_DURATION_SECS, BENCH_INTERVAL_SECS, BENCH_PORT
#   RESULT_DIR (default benchmarks/results)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RESULT_DIR="${RESULT_DIR:-$ROOT/benchmarks/results}"
mkdir -p "$RESULT_DIR"

BENCH_DURATION_SECS="${1:-${BENCH_DURATION_SECS:-30}}"
BENCH_AGENTS="${2:-${BENCH_AGENTS:-10}}"
BENCH_INTERVAL_SECS="${3:-${BENCH_INTERVAL_SECS:-1}}"
BENCH_PORT="${BENCH_PORT:-18499}"
BENCH_TOKEN="bench-token"

DATABASE_URL="${DATABASE_URL:-postgres://observer:observer@127.0.0.1:55432/observer}"
BENCH_DB="${OBSERVER_BENCH_DB:-observer_bench}"

# Nombre del contenedor postgres de dev (deploy/docker-compose.yml)
PG_CT="${OBSERVER_PG_CONTAINER:-observer-postgres}"

log() { echo "[bench] $*"; }
die() { echo "[bench] ERROR: $*" >&2; exit 1; }

# ---------------------------------------------------------------- huella
HASH="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo noum)"
STAMP="$(date +%Y%m%d_%H%M%S)"
OUT="$RESULT_DIR/${HASH}_${STAMP}.txt"

FINGERPRINT="$(cat <<EOF
=== Observatory benchmark ===
commit:     $HASH
fecha:      $(date -u +%Y-%m-%dT%H:%M:%SZ)
Kernel:     $(uname -srmo)
CPU model:  $(awk -F: '/model name/{print $2;exit}' /proc/cpuinfo | sed 's/^ //')
CPU cores:  $(nproc --all 2>/dev/null || nproc)
CPU MHz:    $(awk -F: '/cpu MHz/{print $2;exit}' /proc/cpuinfo | sed 's/^ //')
Mem total:  $(awk '/MemTotal/{print $2" kB"}' /proc/meminfo)
Distro:     $( (cat /etc/os-release 2>/dev/null | awk -F= '/PRETTY_NAME/{print $2}' | tr -d '"') || echo unknown)
gcc:        $(gcc --version 2>/dev/null | head -1 || echo '-')
rustc:      $(rustc --version 2>/dev/null || echo '-')
cargo:      $(cargo --version 2>/dev/null || echo '-')
Params:     duration=${BENCH_DURATION_SECS}s agents=${BENCH_AGENTS} interval=${BENCH_INTERVAL_SECS}s
EOF
)"

log "fingerprint:
$FINGERPRINT"

# ------------------------------------------------------------------- DB
clean_db() {
    if docker exec "$PG_CT" dropdb -U observer --if-exists "$BENCH_DB" >/dev/null 2>&1; then
        log "DB '$BENCH_DB' borrada (previa)"
    fi
}

# limpieza: mata el collector y los agents en background (sin `kill 0`,
# que mataria a este propio shell y dejaria sin escribir el resultado).
cleanup() {
    [ -n "${COLLECTOR_PID:-}" ] && kill "$COLLECTOR_PID" >/dev/null 2>&1 || true
    for _j in $(seq 1 20); do
        jobs -rp | xargs -r kill >/dev/null 2>&1 || true
        sleep 0.1
    done
    clean_db
}
trap cleanup EXIT

clean_db
docker exec "$PG_CT" createdb -U observer "$BENCH_DB" \
    || die "no se pudo crear la DB '$BENCH_DB'"
log "DB dedicated '$BENCH_DB' creada ($BENCH_DB)"

BENCH_DATABASE_URL="postgres://observer:observer@127.0.0.1:55432/$BENCH_DB"

# ------------------------------------------------------------- collector
log "build collector (release)..."
(cd "$ROOT/collector" && cargo build --release) || die "fallo el build del collector"

log "arrancando collector en 127.0.0.1:$BENCH_PORT"
DATABASE_URL="$BENCH_DATABASE_URL" \
OBS_COLLECTOR_TOKEN="$BENCH_TOKEN" \
OBS_LISTEN_ADDR="127.0.0.1:$BENCH_PORT" \
OBS_RATE_LIMIT_ENABLED=false \
OBS_DASHBOARD_DIR="/nonexistent" \
RUST_LOG=warn \
"$ROOT/collector/target/release/observer-collector" &
COLLECTOR_PID=$!

# espera a que el collector escuche
for _ in $(seq 1 50); do
    if curl -sf "http://127.0.0.1:$BENCH_PORT/healthz" >/dev/null 2>&1; then break; fi
    sleep 0.2
done
curl -sf "http://127.0.0.1:$BENCH_PORT/healthz" >/dev/null 2>&1 \
    || die "el collector no arranco en $BENCH_PORT"
log "collector listo. DB='$BENCH_DB'"

# ------------------------------------------------------- agents virtuales
TMP="$(mktemp -d)"
STATS_FILE="$TMP/stats"

send_one() {  # $1 = tipo (heartbeat|metrics), $2 = agent_id (UUID valid)
    local now; now="$(date +%s)"
    local body url
    if [ "$1" = "metrics" ]; then
        url="http://127.0.0.1:$BENCH_PORT/api/v1/metrics"
        body="{\"protocol_version\":1,\"agent_id\":\"$2\",\"timestamp\":$now,\"metrics\":{\"system.cpu.utilization\":0.12,\"system.uptime\":1234}}"
    else
        url="http://127.0.0.1:$BENCH_PORT/api/v1/agents/heartbeat"
        body="{\"protocol_version\":1,\"agent_id\":\"$2\",\"timestamp\":$now}"
    fi
    curl -s -o /dev/null -w "%{http_code} %{time_total}\n" \
         -X POST "$url" -H "Authorization: Bearer $BENCH_TOKEN" \
         -H "Content-Type: application/json" -d "$body" >> "$STATS_FILE"
}

run_agent() {  # $1 = index
    # UUID valido (32 hex) unico por agente: 12 hex de ratio + 16 de ran
    local id seed
    id="$(cat /proc/sys/kernel/random/uuid | tr -d '-')"
    local end; end=$(( $(date +%s) + BENCH_DURATION_SECS ))
    while [ "$(date +%s)" -lt "$end" ]; do
        send_one heartbeat "$id"
        send_one metrics   "$id"
        sleep "$BENCH_INTERVAL_SECS"
    done
}

log "lanzando ${BENCH_AGENTS} agents por ${BENCH_DURATION_SECS}s (intervalo ${BENCH_INTERVAL_SECS}s)..."
START_TS=$(date +%s)
AGENT_PIDS=()
for i in $(seq 1 "$BENCH_AGENTS"); do
    run_agent "$i" &
    AGENT_PIDS+=("$!")
done
# esperar SOLO a los agents (el collector corre indefinido; no esperarlo)
for pid in "${AGENT_PIDS[@]}"; do
    wait "$pid" 2>/dev/null || true
done
END_TS=$(date +%s)
log "carga terminada"

# ------------------------------------------------------------ resultados
REQ_TOTAL=$(wc -l < "$STATS_FILE")
DURATION=$(( END_TS - START_TS )); [ "$DURATION" -lt 1 ] && DURATION=1
RPS=$(awk -v r="$REQ_TOTAL" -v d="$DURATION" 'BEGIN{printf "%.2f", r/d}')
[ -z "$RPS" ] && RPS=NaN

# percentiles de latencia (p99/p95/p50): campo 2 de cada linea, ordenado
if [ -s "$STATS_FILE" ]; then
    mapfile -t LATARR < <(awk '{print $2}' "$STATS_FILE" | sort -n)
    N=${#LATARR[@]}
    pct() { local i=$(( (N * $1 + 50) / 100 )); [ "$i" -lt 1 ] && i=1; echo "${LATARR[$((i-1))]}"; }
    LAT99=$(pct 99); LAT95=$(pct 95); LAT50=$(pct 50)
else
    LAT99="-"; LAT95="-"; LAT50="-"
fi

# distribucion HTTP: 1=status, 2=latencia
HTTP_COUNTS="$(awk '{c[$1]++} END{for (k in c) printf "%s %d\n", k, c[k]}' "$STATS_FILE" | sort -n)"

# persistencia: filas en metric_samples del periodo
ROWS=$(docker exec "$PG_CT" psql -U observer -d "$BENCH_DB" -tAc \
       "SELECT count(*) FROM metric_samples" 2>/dev/null || echo "n/a")
DISTINCT_AGENTS=$(docker exec "$PG_CT" psql -U observer -d "$BENCH_DB" -tAc \
       "SELECT count(DISTINCT agent_id) FROM metric_samples" 2>/dev/null || echo "n/a")

{
    echo "$FINGERPRINT"
    echo ""
    echo "=== Resultados ==="
    echo "Requests totales:      $REQ_TOTAL"
    echo "Duracion (respcarga):  ${DURATION}s"
    echo "Throughput:            ${RPS} req/s"
    echo "Latencia p99:          ${LAT99}s"
    echo "Latencia p95:          ${LAT95}s"
    echo "Latencia p50:          ${LAT50}s"
    echo ""
    echo "Distribucion HTTP:"
    echo "$HTTP_COUNTS" | awk '{printf "  HTTP %s: %d\n", $1, $2}'
    echo ""
    echo "Persistencia (postgres):"
    echo "  filas en metric_samples: $ROWS"
    echo "  agents distintos:        $DISTINCT_AGENTS"
} > "$OUT"

# dejar fingerprint en la salida tambien para el REPORTE
cat "$OUT"

# deja una copia como "latest" para el reporte rapido
cp "$OUT" "$RESULT_DIR/latest.txt"
rm -rf "$TMP"
log "resultado guardado en $OUT"
