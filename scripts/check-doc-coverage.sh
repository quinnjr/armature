#!/bin/bash
# scripts/check-doc-coverage.sh
# Analyze documentation test coverage across workspace

set -e

echo "📊 Armature Documentation Coverage Analysis"
echo "============================================"
echo ""

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Get script directory and project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Workspace members, extracted dynamically from the workspace Cargo.toml
# (see scripts/publish.sh's get_workspace_members for the same pattern).
# This pattern is duplicated in scripts/test-docs.sh and scripts/publish.sh's
# get_workspace_members() — a future pass should extract this into
# scripts/lib.sh and have all three source it.
mapfile -t MEMBERS < <(grep -E '^\s+"armature-' "$PROJECT_ROOT/Cargo.toml" | sed 's/.*"\(armature-[^"]*\)".*/\1/' | sort -u)

if [ "${#MEMBERS[@]}" -eq 0 ]; then
    echo "❌ No workspace members discovered from Cargo.toml — aborting" >&2
    exit 1
fi

cd "$PROJECT_ROOT"

TOTAL_TESTS=0
TOTAL_MEMBERS=0

echo "Member                    | Doc Tests | Status"
echo "--------------------------|-----------|--------"

for member in "${MEMBERS[@]}"; do
    TOTAL_MEMBERS=$((TOTAL_MEMBERS + 1))

    # Run doc tests and capture output
    OUTPUT=$(cargo test --doc -p "$member" --quiet 2>&1 || true)

    # Extract test count
    TEST_COUNT=$(echo "$OUTPUT" | grep "running.*tests" | head -1 | grep -oP '\d+' | head -1)

    if [ -z "$TEST_COUNT" ]; then
        TEST_COUNT=0
    fi

    TOTAL_TESTS=$((TOTAL_TESTS + TEST_COUNT))

    # Format output
    printf "%-25s | %-9s | " "$member" "$TEST_COUNT"

    if [ "$TEST_COUNT" -eq 0 ]; then
        echo -e "${RED}⚠️  NO TESTS${NC}"
    elif [ "$TEST_COUNT" -lt 5 ]; then
        echo -e "${YELLOW}⚠️  LOW${NC}"
    else
        echo -e "${GREEN}✓ GOOD${NC}"
    fi
done

echo ""
echo "============================================"
echo "Summary:"
echo "  Total members:  $TOTAL_MEMBERS"
echo "  Total doc tests: $TOTAL_TESTS"
echo "  Average per member: $((TOTAL_TESTS / TOTAL_MEMBERS))"
echo ""

# Calculate coverage rating
if [ $TOTAL_TESTS -lt 50 ]; then
    echo -e "${RED}Coverage: LOW - Add more documentation examples${NC}"
elif [ $TOTAL_TESTS -lt 100 ]; then
    echo -e "${YELLOW}Coverage: MEDIUM - Consider adding more examples${NC}"
else
    echo -e "${GREEN}Coverage: GOOD - Well documented!${NC}"
fi


