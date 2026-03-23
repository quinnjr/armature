#!/bin/bash
# Benchmark Armature Micro-Framework vs Actix-web vs Axum
#
# Prerequisites:
# - oha (cargo install oha)
# - Build all servers first
#
# Usage: ./scripts/benchmark-comparison.sh

set -e

# Configuration
REQUESTS=100000
CONCURRENCY=100
DURATION=10  # seconds for rate-based tests

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${BLUE}╔══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║       Armature vs Actix vs Axum Benchmark Suite              ║${NC}"
echo -e "${BLUE}╚══════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Check for oha
if ! command -v oha &> /dev/null; then
    echo -e "${RED}Error: 'oha' is not installed. Install with: cargo install oha${NC}"
    exit 1
fi

# Build servers
echo -e "${YELLOW}Building servers...${NC}"
cargo build --release --example micro_benchmark_server 2>/dev/null
(cd benches/comparison_servers/actix_server && cargo build --release 2>/dev/null)
(cd benches/comparison_servers/axum_server && cargo build --release 2>/dev/null)

# Function to start a server and wait for it
start_server() {
    local name=$1
    local cmd=$2
    local port=$3

    echo -e "${BLUE}Starting $name on port $port...${NC}"
    eval "$cmd" &
    sleep 2

    # Check if server is running
    if ! curl -s "http://127.0.0.1:$port/health" > /dev/null; then
        echo -e "${RED}Failed to start $name${NC}"
        return 1
    fi
}

# Function to stop all servers
stop_servers() {
    pkill -f "micro_benchmark_server" 2>/dev/null || true
    pkill -f "actix_server" 2>/dev/null || true
    pkill -f "axum_server" 2>/dev/null || true
    sleep 1
}

# Function to run benchmark
run_benchmark() {
    local name=$1
    local port=$2
    local endpoint=$3
    local method=${4:-GET}
    local body=${5:-""}

    local url="http://127.0.0.1:$port$endpoint"

    if [ "$method" = "POST" ] && [ -n "$body" ]; then
        oha -n $REQUESTS -c $CONCURRENCY -m POST -H "Content-Type: application/json" -d "$body" --no-tui "$url" 2>/dev/null
    else
        oha -n $REQUESTS -c $CONCURRENCY --no-tui "$url" 2>/dev/null
    fi
}

# Results storage
declare -A RESULTS

echo ""
echo -e "${YELLOW}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${YELLOW}                    BENCHMARK RESULTS                           ${NC}"
echo -e "${YELLOW}═══════════════════════════════════════════════════════════════${NC}"
echo ""

# Stop any existing servers
stop_servers

# ============================================
# TEST 1: Hello World (Plaintext)
# ============================================
echo -e "${GREEN}▶ Test 1: Hello World (Plaintext) - GET /${NC}"
echo ""

# Armature
start_server "Armature" "./target/release/examples/micro_benchmark_server" 3000
echo -e "${BLUE}Armature:${NC}"
run_benchmark "Armature" 3000 "/"
stop_servers

# Actix
start_server "Actix" "./benches/comparison_servers/actix_server/target/release/actix_server" 3001
echo -e "${BLUE}Actix-web:${NC}"
run_benchmark "Actix" 3001 "/"
stop_servers

# Axum
start_server "Axum" "./benches/comparison_servers/axum_server/target/release/axum_server" 3002
echo -e "${BLUE}Axum:${NC}"
run_benchmark "Axum" 3002 "/"
stop_servers

echo ""
echo -e "${GREEN}▶ Test 2: JSON Response - GET /json${NC}"
echo ""

# Armature
start_server "Armature" "./target/release/examples/micro_benchmark_server" 3000
echo -e "${BLUE}Armature:${NC}"
run_benchmark "Armature" 3000 "/json"
stop_servers

# Actix
start_server "Actix" "./benches/comparison_servers/actix_server/target/release/actix_server" 3001
echo -e "${BLUE}Actix-web:${NC}"
run_benchmark "Actix" 3001 "/json"
stop_servers

# Axum
start_server "Axum" "./benches/comparison_servers/axum_server/target/release/axum_server" 3002
echo -e "${BLUE}Axum:${NC}"
run_benchmark "Axum" 3002 "/json"
stop_servers

echo ""
echo -e "${GREEN}▶ Test 3: Path Parameter - GET /users/123${NC}"
echo ""

# Armature
start_server "Armature" "./target/release/examples/micro_benchmark_server" 3000
echo -e "${BLUE}Armature:${NC}"
run_benchmark "Armature" 3000 "/users/123"
stop_servers

# Actix
start_server "Actix" "./benches/comparison_servers/actix_server/target/release/actix_server" 3001
echo -e "${BLUE}Actix-web:${NC}"
run_benchmark "Actix" 3001 "/users/123"
stop_servers

# Axum
start_server "Axum" "./benches/comparison_servers/axum_server/target/release/axum_server" 3002
echo -e "${BLUE}Axum:${NC}"
run_benchmark "Axum" 3002 "/users/123"
stop_servers

echo ""
echo -e "${GREEN}▶ Test 4: POST JSON Body - POST /api/users${NC}"
echo ""

POST_BODY='{"name":"Test User","email":"test@example.com"}'

# Armature
start_server "Armature" "./target/release/examples/micro_benchmark_server" 3000
echo -e "${BLUE}Armature:${NC}"
run_benchmark "Armature" 3000 "/api/users" "POST" "$POST_BODY"
stop_servers

# Actix
start_server "Actix" "./benches/comparison_servers/actix_server/target/release/actix_server" 3001
echo -e "${BLUE}Actix-web:${NC}"
run_benchmark "Actix" 3001 "/api/users" "POST" "$POST_BODY"
stop_servers

# Axum
start_server "Axum" "./benches/comparison_servers/axum_server/target/release/axum_server" 3002
echo -e "${BLUE}Axum:${NC}"
run_benchmark "Axum" 3002 "/api/users" "POST" "$POST_BODY"
stop_servers

echo ""
echo -e "${YELLOW}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${YELLOW}                    BENCHMARK COMPLETE                          ${NC}"
echo -e "${YELLOW}═══════════════════════════════════════════════════════════════${NC}"
echo ""
echo -e "${GREEN}All tests completed. Review the results above.${NC}"
echo ""
echo "Note: Results may vary based on system load and configuration."
echo "Run multiple times for consistent results."

