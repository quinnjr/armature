#!/bin/bash
# =============================================================================
# Profile-Guided Optimization (PGO) Build Script
# =============================================================================
#
# This script automates the PGO build process for maximum performance.
#
# Usage:
#   ./scripts/pgo-build.sh [--help] [--generate] [--build] [--all]
#
# Options:
#   --generate   Generate PGO profile data (step 1)
#   --build      Build using PGO data (step 2)
#   --all        Run full PGO workflow
#   --clean      Clean PGO artifacts
#
# Requirements:
#   - LLVM toolchain (llvm-profdata)
#   - Representative workload binary (e.g., benchmarks)
#
# =============================================================================

set -e

# Configuration
PGO_DIR="${PGO_DIR:-/tmp/armature-pgo}"
PROFILE_DATA="${PGO_DIR}/merged.profdata"
# Benchmarks live in the crates they measure, so the profiling workload has to
# name them explicitly to exercise more than the root package. These three cover
# the hot request path: HTTP/1.1 parse+write, routing/DI/handler dispatch, and
# JWT on the auth path. The data/infra crates sit off that path, and the root
# package's two benches touch no `armature_*` symbol at all (they time stdlib
# and crossbeam allocation shapes), so including any of them dilutes the profile
# rather than improving it. Override WORKLOAD_CMD to widen it.
WORKLOAD_CMD="${WORKLOAD_CMD:-cargo bench --profile pgo-generate -p armature-core -p armature-h1 -p armature-jwt}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

show_help() {
    cat << EOF
Profile-Guided Optimization (PGO) Build Script

USAGE:
    ./scripts/pgo-build.sh [OPTIONS]

OPTIONS:
    --generate   Step 1: Build instrumented binary and collect profile data
    --build      Step 2: Build optimized binary using profile data
    --all        Run complete PGO workflow (generate + build)
    --clean      Remove PGO artifacts
    --help       Show this help message

ENVIRONMENT VARIABLES:
    PGO_DIR          Directory for PGO data (default: /tmp/armature-pgo)
    WORKLOAD_CMD     Command to run for profiling
                     (default: cargo bench --profile pgo-generate over
                      armature-core, armature-h1, armature-jwt)

EXAMPLES:
    # Full PGO workflow
    ./scripts/pgo-build.sh --all

    # Just generate profile data
    ./scripts/pgo-build.sh --generate

    # Build using existing profile data
    ./scripts/pgo-build.sh --build

    # Custom workload
    WORKLOAD_CMD="./target/pgo-generate/my-server &" ./scripts/pgo-build.sh --generate

EXPECTED IMPROVEMENT:
    PGO typically provides 10-20% performance improvement for CPU-bound workloads.

EOF
}

check_llvm_tools() {
    if ! command -v llvm-profdata &> /dev/null; then
        log_error "llvm-profdata not found. Install LLVM toolchain."
        echo "  Ubuntu/Debian: sudo apt install llvm"
        echo "  macOS: brew install llvm"
        echo "  Add to PATH: export PATH=\"\$(brew --prefix llvm)/bin:\$PATH\""
        exit 1
    fi
}

clean_pgo() {
    log_info "Cleaning PGO artifacts..."
    rm -rf "${PGO_DIR}"
    rm -f merged.profdata
    log_success "PGO artifacts cleaned"
}

pgo_generate() {
    check_llvm_tools

    log_info "Step 1: Building instrumented binary for PGO..."
    mkdir -p "${PGO_DIR}"

    # Build with profile generation
    log_info "Building with profile generation enabled..."
    RUSTFLAGS="-Cprofile-generate=${PGO_DIR}" cargo build --profile pgo-generate

    log_success "Instrumented build complete"
    log_info "Step 2: Running workload to collect profile data..."

    # Run the workload
    log_info "Running: ${WORKLOAD_CMD}"
    # Start from a clean slate: a previous aborted run's .profraw would
    # otherwise be merged alongside this run's, and they come from a different
    # instrumented binary — llvm-profdata either fails on a function-hash
    # mismatch or silently produces a contaminated profile.
    rm -f "${PGO_DIR}"/*.profraw

    WORKLOAD_STATUS=0
    RUSTFLAGS="-Cprofile-generate=${PGO_DIR}" eval "${WORKLOAD_CMD}" || WORKLOAD_STATUS=$?

    # A nonzero profraw count only proves *something* ran. With a multi-package
    # workload, an abort partway through still leaves files behind, and a PGO
    # build trained on a fraction of the intended workload is silently worse
    # than one trained on all of it — so treat a failed workload as fatal.
    if [ "${WORKLOAD_STATUS}" -ne 0 ]; then
        if [ "${WORKLOAD_STATUS}" -eq 130 ]; then
            log_error "Workload interrupted; the profile is partial. Refusing to build."
        else
            log_error "Workload exited ${WORKLOAD_STATUS}; the profile is partial. Refusing to build."
        fi
        log_info "Narrow or replace it via WORKLOAD_CMD, then re-run."
        exit 1
    fi

    # Check for profile data
    PROFRAW_COUNT=$(find "${PGO_DIR}" -name "*.profraw" 2>/dev/null | wc -l)
    if [ "${PROFRAW_COUNT}" -eq 0 ]; then
        log_error "No profile data generated. Ensure the workload ran correctly."
        exit 1
    fi

    log_success "Collected ${PROFRAW_COUNT} profile data files"

    # Merge profile data
    log_info "Step 3: Merging profile data..."
    llvm-profdata merge -o "${PROFILE_DATA}" "${PGO_DIR}"/*.profraw

    log_success "Profile data merged to: ${PROFILE_DATA}"
    log_info "Profile data size: $(du -h "${PROFILE_DATA}" | cut -f1)"
}

pgo_build() {
    check_llvm_tools

    if [ ! -f "${PROFILE_DATA}" ]; then
        log_error "Profile data not found at: ${PROFILE_DATA}"
        log_info "Run with --generate first, or set PGO_DIR to existing data location"
        exit 1
    fi

    log_info "Building with PGO optimization..."
    log_info "Using profile data: ${PROFILE_DATA}"

    RUSTFLAGS="-Cprofile-use=${PROFILE_DATA}" cargo build --profile pgo-use

    log_success "PGO-optimized build complete!"

    # Show binary info
    if [ -f "target/pgo-use/armature" ]; then
        BINARY_SIZE=$(du -h "target/pgo-use/armature" | cut -f1)
        log_info "Binary size: ${BINARY_SIZE}"
    fi

    echo ""
    log_success "PGO build completed successfully!"
    echo "  Binary location: target/pgo-use/"
    echo ""
    echo "To benchmark:"
    echo "  cargo bench --profile pgo-use -p armature-core"
}

pgo_all() {
    log_info "Running full PGO workflow..."
    echo ""

    pgo_generate
    echo ""

    pgo_build
}

# Main
case "${1:-}" in
    --generate)
        pgo_generate
        ;;
    --build)
        pgo_build
        ;;
    --all)
        pgo_all
        ;;
    --clean)
        clean_pgo
        ;;
    --help|-h)
        show_help
        ;;
    *)
        show_help
        exit 1
        ;;
esac

