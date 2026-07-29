#!/bin/bash
# scripts/test-docs.sh
# Run documentation tests across all workspace members

set -e

echo "📚 Armature Documentation Test Runner"
echo "======================================"
echo ""

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Counters
TOTAL=0
PASSED=0
FAILED=0

# Get script directory and project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# List of all workspace members, extracted dynamically from the workspace
# Cargo.toml (see scripts/publish.sh's get_workspace_members for the same
# pattern).
# This pattern is duplicated in scripts/check-doc-coverage.sh and
# scripts/publish.sh's get_workspace_members() — a future pass should
# extract this into scripts/lib.sh and have all three source it.
mapfile -t MEMBERS < <(grep -E '^\s+"armature-' "$PROJECT_ROOT/Cargo.toml" | sed 's/.*"\(armature-[^"]*\)".*/\1/' | sort -u)

if [ "${#MEMBERS[@]}" -eq 0 ]; then
    echo "❌ No workspace members discovered from Cargo.toml — aborting" >&2
    exit 1
fi

cd "$PROJECT_ROOT"

# Function to test a single member
test_member() {
    local member=$1
    TOTAL=$((TOTAL + 1))

    echo -n "Testing $member... "

    if cargo test --doc -p "$member" --quiet 2>&1 | grep -q "test result: ok"; then
        echo -e "${GREEN}✓ PASSED${NC}"
        PASSED=$((PASSED + 1))
        return 0
    else
        echo -e "${RED}✗ FAILED${NC}"
        FAILED=$((FAILED + 1))
        return 1
    fi
}

# Test each member
for member in "${MEMBERS[@]}"; do
    test_member "$member" || true
done

echo ""
echo "======================================"
echo "Summary:"
echo "  Total:  $TOTAL"
echo -e "  ${GREEN}Passed: $PASSED${NC}"
if [ $FAILED -gt 0 ]; then
    echo -e "  ${RED}Failed: $FAILED${NC}"
fi
echo ""

# Exit with error if any tests failed
if [ $FAILED -gt 0 ]; then
    echo -e "${RED}❌ Some documentation tests failed${NC}"
    exit 1
else
    echo -e "${GREEN}✅ All documentation tests passed!${NC}"
    exit 0
fi


