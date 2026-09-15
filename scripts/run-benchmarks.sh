#!/bin/bash
# scripts/run-benchmarks.sh
# Comprehensive benchmark runner for Armature
#
# Every criterion benchmark now lives in the crate it measures (armature-core,
# armature-jwt, armature-cache, ...), so each suite below is a set of
# `-p <crate> --bench <target>` invocations rather than root-package benches.
# The root package keeps only the two pattern benches (database access shapes,
# allocation timing) — see `--patterns`.

# Exit immediately if a command exits with a non-zero status
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Benchmark suites: "icon|label|package|bench-target"
H1_SUITE=(
    "🧵|H1 Parse|armature-h1|parse"
    "✍|H1 Write|armature-h1|write"
    "🔁|H1 E2E|armature-h1|e2e"
)

CORE_SUITE=(
    "🔧|Core|armature-core|core"
    "🧭|Router|armature-core|router"
    "🧱|Arena|armature-core|arena"
    "📦|Body|armature-core|body"
    "🧾|JSON|armature-core|json"
    "⚡|Micro|armature-core|micro"
    "🚰|Pipeline|armature-core|pipeline"
    "🛡|Resilience|armature-core|resilience"
    "🔍|SIMD Parser|armature-core|simd_parser"
    "📈|Internal Overhead|armature-core|internal_overhead"
)

SECURITY_SUITE=(
    "🔒|JWT|armature-jwt|jwt"
    "🔑|Auth|armature-auth|auth"
)

VALIDATION_SUITE=(
    "✅|Validation|armature-validation|validation"
)

DATA_SUITE=(
    "💾|Cache|armature-cache|cache"
    "⏰|Cron|armature-cron|cron"
    "📬|Queue|armature-queue|queue"
    "🚦|Rate Limit|armature-ratelimit|ratelimit"
    "🗄|Storage|armature-storage|storage"
    "🌐|HTTP Client|armature-http-client|http_client"
)

# Root-package benches. These measure access/allocation *shapes*, not any
# Armature crate — they are NOT the cross-framework comparison, which lives in
# benches/comparison_servers/ and runs via the `http-benchmark` bin.
# Neither imports an `armature_*` symbol (criterion + crossbeam + std only), so
# they build under the root's default feature set; no `--features` needed.
PATTERNS_SUITE=(
    "🗃|Database Patterns|armature-framework|database_benchmarks"
    "🧮|Memory Patterns|armature-framework|memory_benchmarks"
)

# Function to display usage
usage() {
    echo -e "${CYAN}Armature Benchmark Runner${NC}"
    echo "========================="
    echo ""
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  -a, --all              Run all benchmarks"
    echo "  -c, --core             Run armature-core benchmarks only"
    echo "  -s, --security         Run armature-jwt/armature-auth benchmarks only"
    echo "  -v, --validation       Run armature-validation benchmarks only"
    echo "  -d, --data             Run data/infra crate benchmarks only"
    echo "                         (cache, cron, queue, ratelimit, storage, http-client)"
    echo "      --h1               Run armature-h1 benchmarks only (parse, write, e2e)"
    echo "      --patterns         Run root-package pattern benchmarks only"
    echo "                         (database access shapes, allocation timing)"
    echo "                         NOT a cross-framework comparison — for that see"
    echo "                         the http-benchmark runner in benches/README.md"
    echo "  -b, --baseline NAME    Save results as baseline"
    echo "      --compare NAME     Compare with baseline"
    echo "  -o, --open             Open HTML report after running"
    echo "  -q, --quick            Quick run (fewer samples)"
    echo "  -h, --help             Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0 --all               # Run all benchmarks"
    echo "  $0 --core --open       # Run core benchmarks and open report"
    echo "  $0 --baseline main     # Save as baseline 'main'"
    echo "  $0 --compare main      # Compare with baseline 'main'"
    echo "  $0 -a -b v0.1.0        # Run all and save as v0.1.0"
    exit 1
}

# Default values
RUN_ALL=false
RUN_CORE=false
RUN_SECURITY=false
RUN_VALIDATION=false
RUN_DATA=false
RUN_H1=false
RUN_PATTERNS=false
BASELINE=""
COMPARE=""
OPEN_REPORT=false
QUICK=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -a|--all)
            RUN_ALL=true
            shift
            ;;
        -c|--core)
            RUN_CORE=true
            shift
            ;;
        -s|--security)
            RUN_SECURITY=true
            shift
            ;;
        -v|--validation)
            RUN_VALIDATION=true
            shift
            ;;
        -d|--data)
            RUN_DATA=true
            shift
            ;;
        --h1)
            RUN_H1=true
            shift
            ;;
        --patterns)
            RUN_PATTERNS=true
            shift
            ;;
        -p)
            # Everywhere else in this repo `-p` means "package" (cargo's flag).
            # It used to mean --compare here; refuse rather than silently
            # comparing against a baseline named after a crate.
            echo -e "${RED}-p is not a package selector here. Use --compare NAME, or --core/--h1/... to pick a suite.${NC}"
            exit 1
            ;;
        -b|--baseline)
            BASELINE="$2"
            shift 2
            ;;
        --compare)
            COMPARE="$2"
            shift 2
            ;;
        -o|--open)
            OPEN_REPORT=true
            shift
            ;;
        -q|--quick)
            QUICK=true
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            usage
            ;;
    esac
done

# If no specific benchmark selected, show usage
if [[ "$RUN_ALL" == false && "$RUN_CORE" == false && "$RUN_SECURITY" == false && "$RUN_VALIDATION" == false && "$RUN_DATA" == false && "$RUN_H1" == false && "$RUN_PATTERNS" == false ]]; then
    usage
fi

echo -e "${CYAN}╔════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║   Armature Benchmark Suite Runner     ║${NC}"
echo -e "${CYAN}╔════════════════════════════════════════╗${NC}"
echo ""

# Build benchmark options (passed through to criterion, after `--`)
BENCH_OPTS=()
if [[ "$QUICK" == true ]]; then
    # criterion's own default is 100 samples with 3s warm-up + 5s measurement,
    # so a plain `--sample-size 100` would be a no-op. These are genuinely quick.
    BENCH_OPTS+=(--sample-size 20 --warm-up-time 1 --measurement-time 2)
    echo -e "${YELLOW}⚡ Quick mode enabled (20 samples, 1s warm-up, 2s measurement)${NC}"
fi

if [[ -n "$BASELINE" ]]; then
    BENCH_OPTS+=(--save-baseline "$BASELINE")
    echo -e "${GREEN}💾 Saving baseline as: $BASELINE${NC}"
fi

if [[ -n "$COMPARE" ]]; then
    BENCH_OPTS+=(--baseline "$COMPARE")
    echo -e "${BLUE}📊 Comparing with baseline: $COMPARE${NC}"
fi

echo ""

# Run one benchmark target, given an "icon|label|package|bench" entry.
run_benchmark() {
    local icon label pkg bench
    IFS='|' read -r icon label pkg bench <<< "$1"

    if [[ -z "$pkg" || -z "$bench" ]]; then
        echo -e "${RED}malformed suite entry (need icon|label|package|bench): '$1'${NC}"
        exit 1
    fi

    echo -e "${PURPLE}${icon} Running ${label} (${pkg})...${NC}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    if cargo bench -p "$pkg" --bench "$bench" -- "${BENCH_OPTS[@]}"; then
        echo -e "${GREEN}✅ ${label} completed${NC}\n"
    else
        echo -e "${RED}❌ ${label} failed${NC}\n"
        exit 1
    fi
}

# Run every entry in a suite
run_suite() {
    local entry
    for entry in "$@"; do
        run_benchmark "$entry"
    done
}

# Run benchmarks
START_TIME=$(date +%s)

if [[ "$RUN_ALL" == true ]]; then
    echo -e "${CYAN}Running all benchmark suites...${NC}\n"
    run_suite "${CORE_SUITE[@]}"
    run_suite "${H1_SUITE[@]}"
    run_suite "${SECURITY_SUITE[@]}"
    run_suite "${VALIDATION_SUITE[@]}"
    run_suite "${DATA_SUITE[@]}"
    run_suite "${PATTERNS_SUITE[@]}"
else
    if [[ "$RUN_CORE" == true ]]; then
        run_suite "${CORE_SUITE[@]}"
    fi

    if [[ "$RUN_SECURITY" == true ]]; then
        run_suite "${SECURITY_SUITE[@]}"
    fi

    if [[ "$RUN_VALIDATION" == true ]]; then
        run_suite "${VALIDATION_SUITE[@]}"
    fi

    if [[ "$RUN_DATA" == true ]]; then
        run_suite "${DATA_SUITE[@]}"
    fi

    if [[ "$RUN_H1" == true ]]; then
        run_suite "${H1_SUITE[@]}"
    fi

    if [[ "$RUN_PATTERNS" == true ]]; then
        run_suite "${PATTERNS_SUITE[@]}"
    fi
fi

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

# Summary
echo -e "${CYAN}╔════════════════════════════════════════╗${NC}"
echo -e "${CYAN}║           Benchmark Summary            ║${NC}"
echo -e "${CYAN}╚════════════════════════════════════════╝${NC}"
echo ""
echo -e "${GREEN}✅ All benchmarks completed successfully${NC}"
echo -e "${BLUE}⏱️  Total time: ${DURATION}s${NC}"
echo ""

# Report location
REPORT_PATH="target/criterion/report/index.html"
if [[ -f "$REPORT_PATH" ]]; then
    echo -e "${YELLOW}📊 HTML report available at:${NC}"
    echo -e "   ${REPORT_PATH}"
    echo ""

    # Open report if requested
    if [[ "$OPEN_REPORT" == true ]]; then
        echo -e "${GREEN}🌐 Opening report in browser...${NC}"
        if command -v xdg-open &> /dev/null; then
            xdg-open "$REPORT_PATH"
        elif command -v open &> /dev/null; then
            open "$REPORT_PATH"
        else
            echo -e "${YELLOW}⚠️  Could not open browser automatically${NC}"
        fi
    fi
fi

# Performance tips
echo -e "${PURPLE}💡 Tips:${NC}"
echo "   • View detailed report: open $REPORT_PATH"
echo "   • Save baseline: $0 --all --baseline v0.1.0"
echo "   • Compare: $0 --all --compare v0.1.0"
echo "   • Quick test: $0 --all --quick"
echo ""

# List available baselines
BASELINE_DIR="target/criterion"
if [[ -d "$BASELINE_DIR" ]]; then
    BASELINES=$(find "$BASELINE_DIR" -type d -name "*.baseline" 2>/dev/null | wc -l)
    if [[ $BASELINES -gt 0 ]]; then
        echo -e "${BLUE}📂 Available baselines:${NC}"
        find "$BASELINE_DIR" -type d -name "*.baseline" -exec basename {} \; | sed 's/.baseline$//' | sed 's/^/   • /'
        echo ""
    fi
fi

echo -e "${CYAN}════════════════════════════════════════${NC}"
