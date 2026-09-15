#!/bin/bash
# Benchmark Comparison Script
#
# Compares current branch benchmarks against a baseline (default: develop)
#
# Usage:
#   ./scripts/benchmark-compare.sh [baseline-branch] [threshold]
#
# Examples:
#   ./scripts/benchmark-compare.sh                    # Compare against develop
#   ./scripts/benchmark-compare.sh main               # Compare against main
#   ./scripts/benchmark-compare.sh develop 10         # 10% threshold

set -e
# Both bench runs below pipe into `tee`; without pipefail a failing
# `cargo bench` is masked by tee's exit 0 and the script reports a comparison
# built from an empty file.
set -o pipefail

BASELINE_BRANCH="${1:-develop}"
THRESHOLD="${2:-15}"
RESULTS_DIR="target/benchmark-compare"

# Benchmarks are owned by the crates they measure; the root package keeps only
# the two pattern benches (database access shapes, allocation timing) — not the
# cross-framework comparison, which is not a criterion bench at all.
# Keep this list in sync with the suites in scripts/run-benchmarks.sh.
BENCH_PACKAGES=(
    -p armature-core
    -p armature-h1
    -p armature-jwt
    -p armature-auth
    -p armature-validation
    -p armature-cache
    -p armature-cron
    -p armature-queue
    -p armature-ratelimit
    -p armature-storage
    -p armature-http-client
    -p armature-framework
)

echo "🔍 Armature Benchmark Comparison"
echo "================================"
echo "Baseline: $BASELINE_BRANCH"
echo "Threshold: ${THRESHOLD}%"
echo ""

# Create results directory
mkdir -p "$RESULTS_DIR"

# Save current branch. On a detached HEAD this is empty, and a later
# `git checkout ""` would fail silently and leave the user on the baseline
# branch — with their work popped onto it. Refuse up front instead.
CURRENT_BRANCH=$(git branch --show-current)
if [ -z "$CURRENT_BRANCH" ]; then
    echo "❌ Detached HEAD — check out a branch before comparing."
    exit 1
fi
CURRENT_COMMIT=$(git rev-parse --short HEAD)

# The baseline run below checks out another branch with the user's work
# stashed. Under `set -e` a failed bench, a Ctrl-C, or a package that does not
# exist on the baseline branch would otherwise abandon them on the wrong
# branch with their changes only in a stash they were never told about.
STASHED=0
cleanup() {
    # Only pop if we actually got back — otherwise the stash would land on the
    # baseline branch, silently, and the script would still report success.
    if git checkout "$CURRENT_BRANCH"; then
        git submodule update --init --recursive || true
        if [ "$STASHED" -eq 1 ]; then
            git stash pop || \
                echo -e "\n⚠️  Could not restore your stash automatically — recover it with: git stash pop"
            STASHED=0
        fi
    else
        echo -e "\n⚠️  Could not return to '$CURRENT_BRANCH'; you are on $(git rev-parse --abbrev-ref HEAD)."
        [ "$STASHED" -eq 1 ] && echo "   Your work is in the stash entry 'benchmark-compare-temp'."
    fi
}
# INT/TERM must re-raise, or bash resumes at the next statement and the script
# goes on to print a verdict for a run the user cancelled.
trap cleanup EXIT
trap 'cleanup; trap - INT EXIT; kill -INT $$' INT
trap 'cleanup; trap - TERM EXIT; kill -TERM $$' TERM

echo "📊 Running benchmarks on current branch ($CURRENT_BRANCH @ $CURRENT_COMMIT)..."
echo ""

# Run current benchmarks
cargo bench "${BENCH_PACKAGES[@]}" -- --noplot --save-baseline current 2>&1 | tee "$RESULTS_DIR/current.txt"

echo ""
echo "📊 Running benchmarks on baseline ($BASELINE_BRANCH)..."
echo ""

# Stash any changes. Track whether anything was actually stashed — an
# unconditional `git stash pop` on a clean tree pops an unrelated older stash.
if ! git diff --quiet || ! git diff --cached --quiet; then
    git stash push -m "benchmark-compare-temp"
    STASHED=1
fi

# Checkout baseline and run benchmarks. Every benchmark now lives inside a
# submodule, and a superproject checkout moves only the gitlink — without the
# `submodule update` the "baseline" would be the current working trees, i.e.
# the script would compare the branch against itself and report 0% forever.
git checkout "$BASELINE_BRANCH" 2>/dev/null || {
    echo "❌ Failed to checkout $BASELINE_BRANCH"
    exit 1
}
git submodule update --init --recursive

cargo bench "${BENCH_PACKAGES[@]}" -- --noplot --save-baseline baseline 2>&1 | tee "$RESULTS_DIR/baseline.txt"

# Return to original branch (the EXIT trap also covers the failure paths)
cleanup
trap - EXIT INT TERM

echo ""
echo "📈 Comparing Results"
echo "===================="
echo ""

# NOTE: this script measures both sides and saves two criterion baselines; it
# does NOT compute a verdict. It used to print "no significant regressions"
# unconditionally, which is worse than printing nothing. Use `critcmp` (or
# criterion's own --baseline output) to compare, per the instructions below.

echo "Both runs are saved as criterion baselines: 'baseline' and 'current'."
echo "Raw output: $RESULTS_DIR/{baseline,current}.txt"
echo ""
echo "To see the comparison (threshold ${THRESHOLD}%):"
echo "   cargo install critcmp && critcmp baseline current"
echo ""

echo ""
echo "📋 Summary"
echo "=========="
echo "Results saved to: $RESULTS_DIR/"
echo ""

# Compare using Criterion's built-in comparison (if available)
echo "💡 For detailed comparison, use:"
echo "   cargo bench -p armature-core -- --baseline baseline"
echo ""

echo -e "${CYAN}════════════════════════════════════════${NC}"
