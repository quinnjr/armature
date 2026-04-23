#!/bin/bash
# Memory profiling script for Armature framework
# Detects memory leaks and allocation patterns
#
# Usage: ./scripts/memory-profile.sh [tool] [duration]
#   tool: valgrind, heaptrack, dhat, massif (default: dhat)
#   duration: seconds to run the workload (default: 30)
#
# Prerequisites:
#   - valgrind: apt install valgrind (Linux)
#   - heaptrack: apt install heaptrack heaptrack-gui (Linux)
#   - dhat: Built-in with --features dhat
#   - massif: Part of valgrind

set -e

TOOL=${1:-dhat}
DURATION=${2:-30}
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
REPORT_DIR="$PROJECT_DIR/memory-reports"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

cd "$PROJECT_DIR"

echo "🧠 Armature Memory Profiling"
echo "============================"
echo ""
echo "Tool:     $TOOL"
echo "Duration: ${DURATION}s"
echo "Output:   $REPORT_DIR"
echo ""

mkdir -p "$REPORT_DIR"

# Function to generate load
generate_load() {
    local port=$1
    local duration=$2
    local start_time=$(date +%s)
    local request_count=0

    echo "⏱️  Generating load for ${duration}s..."

    while [ $(($(date +%s) - start_time)) -lt $duration ]; do
        # Mix of different request types
        curl -s "http://localhost:$port/health" > /dev/null 2>&1 || true &
        curl -s "http://localhost:$port/json" > /dev/null 2>&1 || true &
        curl -s "http://localhost:$port/users/123" > /dev/null 2>&1 || true &
        curl -s -X POST "http://localhost:$port/api/users" \
            -H "Content-Type: application/json" \
            -d '{"name":"test","email":"test@example.com"}' > /dev/null 2>&1 || true &
        wait
        request_count=$((request_count + 4))

        if [ $((request_count % 200)) -eq 0 ]; then
            echo "   Requests: $request_count"
        fi
    done

    echo "✅ Generated $request_count requests"
}

case $TOOL in
    dhat)
        echo "📦 Building with DHAT profiler..."
        cargo build --example memory_profile_server --release --features memory-profiling 2>&1 | grep -v "^warning:" || true

        echo ""
        echo "🚀 Starting server with DHAT profiler..."
        DHAT_OUT_FILE="$REPORT_DIR/dhat-$TIMESTAMP.json" \
        ./target/release/examples/memory_profile_server &
        SERVER_PID=$!
        sleep 2

        generate_load 3000 $DURATION

        echo ""
        echo "🛑 Stopping server..."
        kill -INT $SERVER_PID 2>/dev/null || true
        wait $SERVER_PID 2>/dev/null || true

        echo ""
        echo "📊 DHAT Results:"
        if [ -f "$REPORT_DIR/dhat-$TIMESTAMP.json" ]; then
            echo "   Report: $REPORT_DIR/dhat-$TIMESTAMP.json"
            echo ""
            echo "   View with: dhat-viewer $REPORT_DIR/dhat-$TIMESTAMP.json"
            echo "   Or online: https://nnethercote.github.io/dh_view/dh_view.html"
        else
            echo "   ⚠️  DHAT output not found"
        fi
        ;;

    valgrind)
        if ! command -v valgrind &> /dev/null; then
            echo "❌ valgrind not found. Install with: apt install valgrind"
            exit 1
        fi

        echo "📦 Building release binary..."
        cargo build --example benchmark_server --release 2>&1 | grep -v "^warning:" || true

        echo ""
        echo "🚀 Starting server under Valgrind (this will be slow)..."
        valgrind --leak-check=full \
                 --show-leak-kinds=all \
                 --track-origins=yes \
                 --verbose \
                 --log-file="$REPORT_DIR/valgrind-$TIMESTAMP.log" \
                 ./target/release/examples/benchmark_server &
        SERVER_PID=$!

        # Valgrind is slow, wait longer for startup
        sleep 10

        # Shorter duration for valgrind due to slowdown
        generate_load 3000 $((DURATION / 3))

        echo ""
        echo "🛑 Stopping server..."
        kill -INT $SERVER_PID 2>/dev/null || true
        wait $SERVER_PID 2>/dev/null || true

        echo ""
        echo "📊 Valgrind Results:"
        echo "   Log: $REPORT_DIR/valgrind-$TIMESTAMP.log"
        echo ""
        echo "   Summary:"
        grep -E "definitely lost|indirectly lost|possibly lost|still reachable" \
             "$REPORT_DIR/valgrind-$TIMESTAMP.log" 2>/dev/null | tail -10 || true
        ;;

    massif)
        if ! command -v valgrind &> /dev/null; then
            echo "❌ valgrind not found. Install with: apt install valgrind"
            exit 1
        fi

        echo "📦 Building release binary..."
        cargo build --example benchmark_server --release 2>&1 | grep -v "^warning:" || true

        echo ""
        echo "🚀 Starting server under Massif heap profiler..."
        valgrind --tool=massif \
                 --massif-out-file="$REPORT_DIR/massif-$TIMESTAMP.out" \
                 --time-unit=B \
                 --detailed-freq=10 \
                 ./target/release/examples/benchmark_server &
        SERVER_PID=$!

        sleep 10

        generate_load 3000 $((DURATION / 3))

        echo ""
        echo "🛑 Stopping server..."
        kill -INT $SERVER_PID 2>/dev/null || true
        wait $SERVER_PID 2>/dev/null || true

        echo ""
        echo "📊 Massif Results:"
        echo "   Output: $REPORT_DIR/massif-$TIMESTAMP.out"
        echo ""
        echo "   View with: ms_print $REPORT_DIR/massif-$TIMESTAMP.out"

        if command -v ms_print &> /dev/null; then
            echo ""
            echo "   Peak memory usage:"
            ms_print "$REPORT_DIR/massif-$TIMESTAMP.out" 2>/dev/null | head -50 || true
        fi
        ;;

    heaptrack)
        if ! command -v heaptrack &> /dev/null; then
            echo "❌ heaptrack not found. Install with: apt install heaptrack"
            exit 1
        fi

        echo "📦 Building release binary..."
        cargo build --example benchmark_server --release 2>&1 | grep -v "^warning:" || true

        echo ""
        echo "🚀 Starting server under Heaptrack..."
        heaptrack -o "$REPORT_DIR/heaptrack-$TIMESTAMP" \
                  ./target/release/examples/benchmark_server &
        SERVER_PID=$!

        sleep 5

        generate_load 3000 $DURATION

        echo ""
        echo "🛑 Stopping server..."
        kill -INT $SERVER_PID 2>/dev/null || true
        wait $SERVER_PID 2>/dev/null || true

        echo ""
        echo "📊 Heaptrack Results:"
        HEAPTRACK_FILE=$(ls -t "$REPORT_DIR"/heaptrack-$TIMESTAMP*.zst 2>/dev/null | head -1)
        if [ -n "$HEAPTRACK_FILE" ]; then
            echo "   Output: $HEAPTRACK_FILE"
            echo ""
            echo "   Analyze with: heaptrack_print $HEAPTRACK_FILE"
            echo "   GUI: heaptrack_gui $HEAPTRACK_FILE"

            if command -v heaptrack_print &> /dev/null; then
                echo ""
                echo "   Summary:"
                heaptrack_print "$HEAPTRACK_FILE" 2>/dev/null | head -30 || true
            fi
        else
            echo "   ⚠️  Heaptrack output not found"
        fi
        ;;

    *)
        echo "❌ Unknown tool: $TOOL"
        echo "Available tools: dhat, valgrind, massif, heaptrack"
        exit 1
        ;;
esac

echo ""
echo "🎉 Memory profiling complete!"
echo ""
echo "All reports saved to: $REPORT_DIR"
ls -la "$REPORT_DIR"/*$TIMESTAMP* 2>/dev/null || true

