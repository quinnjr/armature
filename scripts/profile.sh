#!/bin/bash
# Profile the Armature framework
# Usage: ./scripts/profile.sh [duration_seconds]

set -e

DURATION=${1:-30}
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

echo "🔬 Armature Profiling Script"
echo "============================"
echo ""

# Check for required tools
if ! command -v curl &> /dev/null; then
    echo "❌ curl is required but not installed."
    exit 1
fi

# Build in release mode with debug symbols
echo "📦 Building with profiling enabled..."
cargo build --example profiling_server --release 2>&1 | grep -v "warning:"

# Start the server in background, capturing its own stdout/stderr to a temp
# file. The server binds to an OS-assigned ephemeral port (127.0.0.1:0) and
# prints the port it actually got (e.g. "Server: http://localhost:PORT") only
# to its own stdout, so that's where we have to read it from.
echo ""
echo "🚀 Starting profiling server..."
SERVER_LOG="$(mktemp -t armature-profile-server.XXXXXX.log)"
cleanup() {
    if [ -n "${SERVER_PID:-}" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
        kill -INT "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    rm -f "$SERVER_LOG"
}
trap cleanup EXIT

cargo run --example profiling_server --release > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!

# Wait for the server to print its bound port. Poll instead of a fixed sleep
# since build/startup time varies, and bail out early (with the captured log)
# if the server process dies before it gets there.
PORT=""
for _ in $(seq 1 30); do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "❌ Server process exited before it started listening. Log:"
        grep -v "warning:" "$SERVER_LOG" || true
        exit 1
    fi
    PORT=$(grep -o "localhost:[0-9]*" "$SERVER_LOG" 2>/dev/null | tail -1 | cut -d: -f2)
    if [ -n "$PORT" ]; then
        break
    fi
    sleep 0.5
done

if [ -z "$PORT" ]; then
    echo "⚠️  Could not detect the server's port from its output after 15s; falling back to 3000"
    PORT=3000
fi

echo "📡 Server running on port $PORT"
echo ""
echo "⏱️  Generating load for ${DURATION} seconds..."
echo ""

# Generate load
START_TIME=$(date +%s)
REQUEST_COUNT=0

while [ $(($(date +%s) - START_TIME)) -lt $DURATION ]; do
    curl -s "http://localhost:$PORT/tasks" > /dev/null &
    curl -s "http://localhost:$PORT/tasks/1" > /dev/null &
    curl -s "http://localhost:$PORT/compute/light" > /dev/null &
    curl -s "http://localhost:$PORT/compute/heavy/500" > /dev/null &
    wait
    REQUEST_COUNT=$((REQUEST_COUNT + 4))

    if [ $((REQUEST_COUNT % 100)) -eq 0 ]; then
        ELAPSED=$(($(date +%s) - START_TIME))
        echo "   Requests: $REQUEST_COUNT, Elapsed: ${ELAPSED}s"
    fi
done

echo ""
echo "✅ Load generation complete: $REQUEST_COUNT requests"
echo ""

# Stop the server gracefully
echo "🛑 Stopping server..."
kill -INT $SERVER_PID 2>/dev/null || true
wait $SERVER_PID 2>/dev/null || true

sleep 1

# Check for output files
if [ -f "flamegraph-profile.svg" ]; then
    echo ""
    echo "📊 Results:"
    echo "   Flamegraph: $(pwd)/flamegraph-profile.svg"
    echo ""
    echo "Open the SVG file in a browser to explore the CPU profile."
    echo "Wider bars = more CPU time spent in that function."
else
    echo "⚠️  Flamegraph not generated. The server may have exited unexpectedly."
    exit 1
fi

echo ""
echo "🎉 Profiling complete!"

