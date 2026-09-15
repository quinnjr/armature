#!/usr/bin/env bash
# scripts/bench-h1.sh
#
# Load-test armature-h1 against bare hyper over a real socket, side by side.
#
# The design doc compares the two. This script is what makes that comparison
# reproducible instead of a claim: it prints every version and command line it
# used, so a number reported from it can be re-derived or disputed.
#
# What it does NOT do is produce a publishable throughput figure on a laptop.
# Frequency scaling, other processes, and a loopback socket all move the result
# more than most code changes do. Treat it as a relative comparison run back to
# back on one machine, and read absolute per-request cost out of
# `cargo bench -p armature-h1` instead.
#
# Usage:
#   scripts/bench-h1.sh                 # both servers, defaults
#   DURATION=30s CONNECTIONS=200 scripts/bench-h1.sh
#   scripts/bench-h1.sh --only armature
#   scripts/bench-h1.sh --only hyper

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly REPO_ROOT
cd "$REPO_ROOT"

# ----------------------------------------------------------------------------
# Configuration
# ----------------------------------------------------------------------------

DURATION="${DURATION:-10s}"
CONNECTIONS="${CONNECTIONS:-64}"
# oha's default is one worker thread per connection, which oversubscribes the
# machine and turns the load generator into the bottleneck. Cap it at the core
# count unless told otherwise.
CLIENT_THREADS="${CLIENT_THREADS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
# Not configurable: the `hello` example hardcodes 127.0.0.1:8080, and it is a
# minimal example on purpose rather than a configurable test server.
readonly ARMATURE_PORT=8080
HYPER_PORT="${HYPER_PORT:-8081}"
ONLY=""

readonly HYPER_DIR="benches/comparison_servers/hyper_h1_server"
readonly HYPER_MANIFEST="${HYPER_DIR}/Cargo.toml"
# The baseline is its own workspace root (see its Cargo.toml), so its artifacts
# land under its own target directory rather than the repo's.
readonly HYPER_BIN="${HYPER_DIR}/target/release/hyper-h1-benchmark-server"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

while [[ $# -gt 0 ]]; do
    case "$1" in
        --only)
            ONLY="${2:-}"
            if [[ "$ONLY" != "armature" && "$ONLY" != "hyper" ]]; then
                echo "--only takes 'armature' or 'hyper'" >&2
                exit 2
            fi
            shift 2
            ;;
        -h|--help)
            sed -n '3,21p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

# ----------------------------------------------------------------------------
# Preflight
# ----------------------------------------------------------------------------

if ! command -v oha >/dev/null 2>&1; then
    echo -e "${RED}oha is not installed.${NC}" >&2
    echo "  cargo install oha" >&2
    echo >&2
    echo "oha rather than wrk or ab: it reports a latency distribution rather" >&2
    echo "than a mean, and a mean latency on a keep-alive HTTP benchmark hides" >&2
    echo "exactly the tail this crate exists to keep short." >&2
    exit 1
fi

for port in "$ARMATURE_PORT" "$HYPER_PORT"; do
    if command -v ss >/dev/null 2>&1 && ss -ltn 2>/dev/null | grep -q ":${port} "; then
        echo -e "${RED}port ${port} is already in use.${NC}" >&2
        echo "Set ARMATURE_PORT / HYPER_PORT to something free." >&2
        exit 1
    fi
done

# ----------------------------------------------------------------------------
# Build both servers first, so compile time cannot land inside a measurement
# window — and so the lockfile the version report reads from exists.
# ----------------------------------------------------------------------------

echo -e "${BLUE}Building both servers (release)...${NC}"
BUILD_ARMATURE="cargo build --release -p armature-h1 --example hello"
BUILD_HYPER="cargo build --release --manifest-path ${HYPER_MANIFEST}"
echo "  \$ ${BUILD_ARMATURE}"
[[ "$ONLY" == "hyper" ]] || $BUILD_ARMATURE
echo "  \$ ${BUILD_HYPER}"
[[ "$ONLY" == "armature" ]] || $BUILD_HYPER
echo

# ----------------------------------------------------------------------------
# Provenance — printed before any measurement, so the numbers below are
# attributable to a specific toolchain and dependency set.
# ----------------------------------------------------------------------------

hyper_version() {
    # From the lockfile of the baseline crate, not from its Cargo.toml: the
    # requirement is a caret range and the resolved version is what actually ran.
    # Lockfiles are gitignored in this repo, so this reads the one the build above
    # just produced.
    if [[ -f "${HYPER_DIR}/Cargo.lock" ]]; then
        awk '/^name = "hyper"$/{found=1; next} found && /^version = /{gsub(/[",]/,""); print $3; exit}' \
            "${HYPER_DIR}/Cargo.lock"
    else
        echo "unresolved (no lockfile)"
    fi
}

armature_h1_version() {
    awk '/^version = /{gsub(/[",]/,""); print $3; exit}' armature-h1/Cargo.toml
}

echo -e "${BOLD}armature-h1 vs hyper — HTTP/1.1 over loopback TCP${NC}"
echo
echo -e "${CYAN}Environment${NC}"
echo "  date:            $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
echo "  host:            $(uname -srm)"
echo "  cpus:            ${CLIENT_THREADS}"
if [[ -r /proc/cpuinfo ]]; then
    echo "  model:           $(awk -F': ' '/^model name/{print $2; exit}' /proc/cpuinfo)"
fi
echo "  rustc:           $(rustc --version)"
echo "  cargo:           $(cargo --version)"
echo "  oha:             $(oha --version)"
echo "  armature-h1:     $(armature_h1_version)"
echo "  hyper:           $(hyper_version)"
echo "  profile:         release"
echo
echo -e "${CYAN}Load${NC}"
echo "  duration:        ${DURATION}"
echo "  connections:     ${CONNECTIONS}"
echo "  client threads:  ${CLIENT_THREADS}"
echo "  keep-alive:      on (oha default)"
echo

SERVER_PID=""
cleanup() {
    if [[ -n "$SERVER_PID" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

# Wait for a server to accept connections. A fixed sleep either wastes time or
# measures a server that was still binding when the load started.
wait_for_port() {
    local port="$1" name="$2"
    local waited=0
    while ((waited < 100)); do
        if (exec 3<>"/dev/tcp/127.0.0.1/${port}") 2>/dev/null; then
            exec 3<&- 2>/dev/null || true
            return 0
        fi
        sleep 0.1
        # Not `((waited++))`: post-increment evaluates to the old value, so the
        # first iteration would return 1 and `set -e` would kill the script.
        waited=$((waited + 1))
    done
    echo -e "${RED}${name} never started listening on ${port}.${NC}" >&2
    return 1
}

# ----------------------------------------------------------------------------
# One measurement run.
# ----------------------------------------------------------------------------

# run_one <display name> <port> <output file> <printed env prefix> <command...>
#
# The env prefix is printed rather than applied: the caller exports what it needs.
# It exists so the echoed command line is complete enough to paste, which is the
# whole point of echoing it.
run_one() {
    local name="$1" port="$2" out="$3" env_prefix="$4"
    shift 4
    local -a start_cmd=("$@")

    echo -e "${BOLD}${name}${NC}"
    echo "  server:  \$ ${env_prefix}${start_cmd[*]}"

    "${start_cmd[@]}" >"${out}.server.log" 2>&1 &
    SERVER_PID=$!
    wait_for_port "$port" "$name"

    local url="http://127.0.0.1:${port}/"
    local -a oha_cmd=(
        oha
        -z "$DURATION"
        -c "$CONNECTIONS"
        -t "${CLIENT_THREADS}"
        --no-tui
        "$url"
    )
    echo "  client:  \$ ${oha_cmd[*]}"
    echo

    # A short unmeasured warm-up: the first requests on a fresh process pay page
    # faults and pool growth that have nothing to do with steady-state cost.
    oha -z 2s -c "$CONNECTIONS" -t "${CLIENT_THREADS}" --no-tui "$url" >/dev/null 2>&1 || true

    "${oha_cmd[@]}" | tee "$out"

    cleanup
    SERVER_PID=""
    echo
}

RESULTS_DIR="$(mktemp -d)"
trap 'cleanup; rm -rf "$RESULTS_DIR"' EXIT INT TERM

ARMATURE_OUT="${RESULTS_DIR}/armature.txt"
HYPER_OUT="${RESULTS_DIR}/hyper.txt"

if [[ "$ONLY" != "hyper" ]]; then
    run_one "armature-h1 (thread-per-core, SO_REUSEPORT)" "$ARMATURE_PORT" "$ARMATURE_OUT" \
        "" ./target/release/examples/hello
fi

if [[ "$ONLY" != "armature" ]]; then
    # Exported rather than passed through `env`, so the printed command line is
    # the one that ran.
    export PORT="$HYPER_PORT"
    run_one "hyper (tokio multi-thread, task per connection)" "$HYPER_PORT" "$HYPER_OUT" \
        "PORT=${HYPER_PORT} " "$HYPER_BIN"
fi

# ----------------------------------------------------------------------------
# Side by side
# ----------------------------------------------------------------------------

# oha's summary lines, e.g. "  Requests/sec: 123456.7890".
field() {
    local file="$1" label="$2"
    [[ -f "$file" ]] || { echo "n/a"; return; }
    awk -v label="$label" '
        index($0, label) { sub(/.*:[[:space:]]*/, ""); print; exit }
    ' "$file"
}

if [[ -z "$ONLY" ]]; then
    echo -e "${CYAN}Side by side${NC}"
    printf '  %-24s %20s %20s\n' "" "armature-h1" "hyper"
    printf '  %-24s %20s %20s\n' "requests/sec" \
        "$(field "$ARMATURE_OUT" 'Requests/sec')" \
        "$(field "$HYPER_OUT" 'Requests/sec')"
    printf '  %-24s %20s %20s\n' "total data" \
        "$(field "$ARMATURE_OUT" 'Total data')" \
        "$(field "$HYPER_OUT" 'Total data')"
    printf '  %-24s %20s %20s\n' "slowest" \
        "$(field "$ARMATURE_OUT" 'Slowest')" \
        "$(field "$HYPER_OUT" 'Slowest')"
    printf '  %-24s %20s %20s\n' "fastest" \
        "$(field "$ARMATURE_OUT" 'Fastest')" \
        "$(field "$HYPER_OUT" 'Fastest')"
    printf '  %-24s %20s %20s\n' "average" \
        "$(field "$ARMATURE_OUT" 'Average')" \
        "$(field "$HYPER_OUT" 'Average')"
    echo
    echo -e "${YELLOW}Read the p99 in the latency distribution above, not just the mean.${NC}"
    echo "Thread-per-core trades head-of-line blocking within a connection for"
    echo "zero cross-core coordination; that tradeoff shows up in the tail, which"
    echo "is where a throughput headline hides it. See armature-h1/README.md."
    echo
fi

echo -e "${GREEN}Done.${NC}"
