#!/bin/bash
# =============================================================================
# Armature Workspace Publisher
# =============================================================================
#
# Publishes all workspace crates to crates.io in the correct dependency order.
# Includes rate limiting to avoid hitting crates.io publish limits.
#
# Usage:
#   ./scripts/publish.sh [OPTIONS]
#
# Options:
#   --dry-run       Show publish order without actually publishing
#   --check         Verify all crates are ready to publish
#   --single CRATE  Publish only the specified crate
#   --from CRATE    Publish starting from the specified crate
#   --skip CRATE    Skip the specified crate (can be used multiple times)
#   --no-verify     Skip cargo publish verification step
#   --delay SECS    Delay between publishes (default: 30)
#   --burst N       Publish N crates then pause longer (default: 5)
#   --burst-delay S Delay after burst (default: 120)
#   --help          Show this help message
#
# Environment:
#   CARGO_REGISTRY_TOKEN  Required for publishing (or use `cargo login`)
#
# Rate Limiting:
#   crates.io has rate limits on publishing. This script handles them by:
#   - Waiting between each publish (--delay, default 30s)
#   - Taking longer breaks after bursts (--burst, --burst-delay)
#   - Automatically retrying with exponential backoff on rate limit errors
#
# =============================================================================

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
NC='\033[0m' # No Color

# Configuration
DRY_RUN=false
CHECK_ONLY=false
SINGLE_CRATE=""
FROM_CRATE=""
NO_VERIFY=false
FORCE=false
SKIP_CRATES=()
CRATES_IO_API="https://crates.io/api/v1/crates"

# Rate limiting configuration
PUBLISH_DELAY=30       # Delay between publishes (seconds)
BURST_SIZE=5           # Number of crates to publish before longer pause
BURST_DELAY=120        # Delay after burst (seconds)
MAX_RETRIES=0          # Maximum retries on rate limit (0 = unlimited)
INITIAL_BACKOFF=60     # Initial backoff on rate limit (seconds)
MAX_BACKOFF=600        # Maximum backoff (10 minutes)

# Tracking
BURST_COUNT=0
TOTAL_WAIT_TIME=0
START_TIME=$(date +%s)

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

log_rate() {
    echo -e "${MAGENTA}[RATE]${NC} $1"
}

# =============================================================================
# Rate Limiting Utilities
# =============================================================================

# Format seconds as human-readable time
format_time() {
    local seconds=$1
    if [[ $seconds -lt 60 ]]; then
        echo "${seconds}s"
    elif [[ $seconds -lt 3600 ]]; then
        local mins=$((seconds / 60))
        local secs=$((seconds % 60))
        echo "${mins}m ${secs}s"
    else
        local hours=$((seconds / 3600))
        local mins=$(((seconds % 3600) / 60))
        echo "${hours}h ${mins}m"
    fi
}

# Calculate estimated time for publishing
estimate_publish_time() {
    local total_crates=$1
    local to_publish=$2

    # Estimate: delay per crate + burst delays
    local base_time=$((to_publish * PUBLISH_DELAY))
    local num_bursts=$((to_publish / BURST_SIZE))
    local burst_time=$((num_bursts * BURST_DELAY))
    local total=$((base_time + burst_time))

    echo $total
}

# Show progress bar
show_progress() {
    local current=$1
    local total=$2
    local width=40

    local pct=$((current * 100 / total))
    local filled=$((current * width / total))
    local empty=$((width - filled))

    printf "\r  Progress: ["
    printf "%${filled}s" | tr ' ' '█'
    printf "%${empty}s" | tr ' ' '░'
    printf "] %3d%% (%d/%d)" "$pct" "$current" "$total"
}

# Wait with countdown display
wait_with_countdown() {
    local seconds=$1
    local reason=$2

    if [[ "$DRY_RUN" == "true" ]]; then
        return
    fi

    log_rate "$reason - waiting $(format_time $seconds)..."

    while [[ $seconds -gt 0 ]]; do
        printf "\r  ⏳ %3ds remaining..." "$seconds"
        sleep 1
        ((seconds--))
        TOTAL_WAIT_TIME=$((TOTAL_WAIT_TIME + 1))
    done
    printf "\r  ✓ Wait complete.        \n"
}

# Handle rate limit with exponential backoff
handle_rate_limit() {
    local attempt=$1
    local crate=$2

    # MAX_RETRIES=0 means unlimited
    if [[ $MAX_RETRIES -gt 0 && $attempt -ge $MAX_RETRIES ]]; then
        log_error "Max retries ($MAX_RETRIES) exceeded for $crate"
        return 1
    fi

    # Exponential backoff: initial * 2^attempt, capped at max
    local backoff=$((INITIAL_BACKOFF * (2 ** attempt)))
    if [[ $backoff -gt $MAX_BACKOFF ]]; then
        backoff=$MAX_BACKOFF
    fi

    if [[ $MAX_RETRIES -eq 0 ]]; then
        log_warn "Rate limited! Attempt $((attempt + 1)) (unlimited retries)"
    else
        log_warn "Rate limited! Attempt $((attempt + 1))/$MAX_RETRIES"
    fi
    wait_with_countdown $backoff "Rate limit backoff"
    return 0
}

# Check if cargo publish output indicates rate limiting
is_rate_limited() {
    local output="$1"

    # crates.io rate limit messages
    if echo "$output" | grep -qiE "rate.?limit|too many requests|429|slow down"; then
        return 0
    fi
    return 1
}

show_help() {
    cat << 'EOF'
Armature Workspace Publisher

Publishes all workspace crates to crates.io in the correct dependency order.
Includes rate limiting to avoid hitting crates.io publish limits.

USAGE:
    ./scripts/publish.sh [OPTIONS]

OPTIONS:
    --dry-run       Show publish order without actually publishing
    --check         Verify all crates are ready to publish
    --single CRATE  Publish only the specified crate
    --from CRATE    Start publishing from the specified crate
    --skip CRATE    Skip the specified crate (can be repeated)
    --no-verify     Skip cargo publish verification step
    --force         Publish even if version already exists on crates.io
    --help          Show this help message

RATE LIMITING OPTIONS:
    --delay SECS    Delay between each publish (default: 30)
    --burst N       Publish N crates then take a longer break (default: 5)
    --burst-delay S Delay after each burst of publishes (default: 120)
    --fast          Fast mode: minimal delays (5s/3/30s) - risky!
    --safe          Safe mode: conservative delays (60s/3/300s)

ENVIRONMENT:
    CARGO_REGISTRY_TOKEN  API token for crates.io (or use `cargo login`)

RATE LIMITING:
    crates.io enforces rate limits on publishing. This script handles them by:

    1. Standard Delay: Waits --delay seconds between each publish
    2. Burst Control: After --burst publishes, waits --burst-delay seconds
    3. Auto-Retry: On rate limit errors, retries with exponential backoff

    Default timing for ~50 crates:
    - Normal: ~30 min (30s delays, 2min burst pauses)
    - Safe:   ~60 min (60s delays, 5min burst pauses)
    - Fast:   ~15 min (5s delays, 30s burst pauses) - may hit limits!

EXAMPLES:
    # See publish order and timing estimate
    ./scripts/publish.sh --dry-run

    # Check all crates are ready
    ./scripts/publish.sh --check

    # Publish everything with defaults
    ./scripts/publish.sh

    # Publish with custom timing
    ./scripts/publish.sh --delay 45 --burst 3 --burst-delay 180

    # Safe mode for first-time publishing
    ./scripts/publish.sh --safe

    # Publish single crate
    ./scripts/publish.sh --single armature-log

    # Resume from a specific crate
    ./scripts/publish.sh --from armature-auth

    # Skip problematic crates
    ./scripts/publish.sh --skip armature-cli --skip armature-ferron

DEPENDENCY ORDER:
    The script automatically determines the correct publish order by
    analyzing inter-workspace dependencies. Crates with no workspace
    dependencies are published first.

EOF
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --check)
            CHECK_ONLY=true
            shift
            ;;
        --single)
            SINGLE_CRATE="$2"
            shift 2
            ;;
        --from)
            FROM_CRATE="$2"
            shift 2
            ;;
        --skip)
            SKIP_CRATES+=("$2")
            shift 2
            ;;
        --no-verify)
            NO_VERIFY=true
            shift
            ;;
        --force)
            FORCE=true
            shift
            ;;
        --delay)
            PUBLISH_DELAY="$2"
            shift 2
            ;;
        --burst)
            BURST_SIZE="$2"
            shift 2
            ;;
        --burst-delay)
            BURST_DELAY="$2"
            shift 2
            ;;
        --fast)
            PUBLISH_DELAY=5
            BURST_SIZE=3
            BURST_DELAY=30
            INITIAL_BACKOFF=30
            shift
            ;;
        --safe)
            PUBLISH_DELAY=60
            BURST_SIZE=3
            BURST_DELAY=300
            INITIAL_BACKOFF=120
            shift
            ;;
        --help|-h)
            show_help
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            show_help
            exit 1
            ;;
    esac
done

# Get script directory and project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_ROOT"

# =============================================================================
# Dependency Analysis
# =============================================================================

# Get all workspace members
get_workspace_members() {
    grep -E '^\s+"armature-' Cargo.toml | sed 's/.*"\(armature-[^"]*\)".*/\1/' | sort -u
}

# Get workspace dependencies for a crate
get_workspace_deps() {
    local crate=$1
    local cargo_toml="$crate/Cargo.toml"

    if [[ ! -f "$cargo_toml" ]]; then
        return
    fi

    # Extract path dependencies that are workspace members
    grep -E 'path\s*=\s*"\.\./armature-' "$cargo_toml" 2>/dev/null | \
        sed 's/.*path\s*=\s*"\.\.\/\(armature-[^"]*\)".*/\1/' | \
        sort -u
}

# Build dependency graph and compute publish order using topological sort
compute_publish_order() {
    local members
    members=$(get_workspace_members)

    declare -A in_degree
    declare -A deps
    declare -a order

    # Initialize
    for crate in $members; do
        in_degree[$crate]=0
        deps[$crate]=""
    done

    # Build dependency graph
    for crate in $members; do
        local crate_deps
        crate_deps=$(get_workspace_deps "$crate")
        deps[$crate]="$crate_deps"

        for dep in $crate_deps; do
            if [[ -n "${in_degree[$dep]+x}" ]]; then
                ((in_degree[$crate]++))
            fi
        done
    done

    # Kahn's algorithm for topological sort
    local queue=()

    # Find all crates with no dependencies
    for crate in $members; do
        if [[ ${in_degree[$crate]} -eq 0 ]]; then
            queue+=("$crate")
        fi
    done

    while [[ ${#queue[@]} -gt 0 ]]; do
        # Sort queue for deterministic order
        IFS=$'\n' sorted_queue=($(sort <<<"${queue[*]}")); unset IFS
        local current="${sorted_queue[0]}"
        queue=("${sorted_queue[@]:1}")

        order+=("$current")

        # For each crate that depends on current
        for crate in $members; do
            if [[ "${deps[$crate]}" == *"$current"* ]]; then
                ((in_degree[$crate]--))
                if [[ ${in_degree[$crate]} -eq 0 ]]; then
                    queue+=("$crate")
                fi
            fi
        done
    done

    # Check for cycles
    local total_members
    total_members=$(echo "$members" | wc -w)
    if [[ ${#order[@]} -ne $total_members ]]; then
        log_error "Circular dependency detected!"
        exit 1
    fi

    echo "${order[@]}"
}

# =============================================================================
# Verification
# =============================================================================

# Check if a crate is ready to publish
check_crate() {
    local crate=$1
    local cargo_toml="$crate/Cargo.toml"
    local errors=()
    local warnings=()

    # Check Cargo.toml exists
    if [[ ! -f "$cargo_toml" ]]; then
        errors+=("Cargo.toml not found")
    else
        # Check for required fields
        if ! grep -q '^name\s*=' "$cargo_toml" && ! grep -q 'name.workspace' "$cargo_toml"; then
            errors+=("Missing 'name' field")
        fi

        if ! grep -q 'version' "$cargo_toml"; then
            errors+=("Missing 'version' field")
        fi

        if ! grep -q 'license' "$cargo_toml"; then
            errors+=("Missing 'license' field")
        fi

        if ! grep -q 'description' "$cargo_toml"; then
            errors+=("Missing 'description' field")
        fi

        # Check for path dependencies (warning - will need conversion)
        local path_deps
        path_deps=$(grep -cE 'path\s*=\s*"\.\./armature-' "$cargo_toml" 2>/dev/null) || path_deps=0
        if [[ $path_deps -gt 0 ]]; then
            warnings+=("$path_deps path deps (run prepare-publish.sh)")
        fi
    fi

    # Check src/lib.rs or src/main.rs exists
    if [[ ! -f "$crate/src/lib.rs" && ! -f "$crate/src/main.rs" ]]; then
        errors+=("Missing src/lib.rs or src/main.rs")
    fi

    # Check crates.io publication status
    local version
    version=$(get_crate_version "$crate")
    if [[ -n "$version" ]]; then
        local pub_status
        pub_status=$(check_crates_io_version "$crate" "$version")
        if [[ "$pub_status" == "published" ]]; then
            warnings+=("v$version already on crates.io")
        fi
    fi

    if [[ ${#errors[@]} -gt 0 ]]; then
        echo "FAIL: ${errors[*]}"
        return 1
    elif [[ ${#warnings[@]} -gt 0 ]]; then
        echo "WARN: ${warnings[*]}"
        return 0  # Don't fail on warnings
    else
        echo "OK"
        return 0
    fi
}

# Check all crates
check_all_crates() {
    log_info "Checking all crates for publish readiness..."
    echo ""

    local publish_order
    publish_order=$(compute_publish_order)

    local failed=0
    local passed=0
    local warned=0

    for crate in $publish_order; do
        local result
        result=$(check_crate "$crate")

        if [[ "$result" == "OK" ]]; then
            echo -e "  ${GREEN}✓${NC} $crate"
            passed=$((passed + 1))
        elif [[ "$result" == WARN:* ]]; then
            echo -e "  ${YELLOW}!${NC} $crate: ${result#WARN: }"
            warned=$((warned + 1))
        else
            echo -e "  ${RED}✗${NC} $crate: ${result#FAIL: }"
            failed=$((failed + 1))
        fi
    done

    echo ""
    echo "Results: $passed ready, $warned warnings, $failed failed"

    if [[ $failed -gt 0 ]]; then
        return 1
    fi
    return 0
}

# =============================================================================
# crates.io Version Checking
# =============================================================================

# Get the version from a crate's Cargo.toml
get_crate_version() {
    local crate=$1
    local cargo_toml="$crate/Cargo.toml"

    if [[ ! -f "$cargo_toml" ]]; then
        echo ""
        return
    fi

    # Check for workspace version (must be exactly "version.workspace", not "rust-version.workspace")
    if grep -qE '^version\.workspace\s*=\s*true' "$cargo_toml"; then
        # Get version from root Cargo.toml
        grep -E '^version\s*=' "$PROJECT_ROOT/Cargo.toml" | head -1 | sed 's/.*"\([^"]*\)".*/\1/'
    else
        # Get version from crate's Cargo.toml
        grep -E '^version\s*=' "$cargo_toml" | head -1 | sed 's/.*"\([^"]*\)".*/\1/'
    fi
}

# Check if a specific version is published on crates.io
# Returns: "published", "not_found", or "error"
check_crates_io_version() {
    local crate=$1
    local version=$2

    # Convert underscores to hyphens for crates.io lookup
    local crate_name="${crate//_/-}"

    # Query crates.io API
    local response
    local http_code

    # Use curl with proper User-Agent (required by crates.io API)
    response=$(curl -s -w "\n%{http_code}" \
        -H "User-Agent: armature-publish-script/1.0" \
        "$CRATES_IO_API/$crate_name" 2>/dev/null)

    http_code=$(echo "$response" | tail -1)
    local body=$(echo "$response" | sed '$d')

    if [[ "$http_code" == "404" ]]; then
        echo "not_found"
        return
    fi

    if [[ "$http_code" != "200" ]]; then
        echo "error"
        return
    fi

    # Check if the specific version exists
    if echo "$body" | grep -q "\"num\":\"$version\""; then
        echo "published"
    else
        echo "not_published"
    fi
}

# Get all published versions for a crate
get_published_versions() {
    local crate=$1
    local crate_name="${crate//_/-}"

    local response
    response=$(curl -s \
        -H "User-Agent: armature-publish-script/1.0" \
        "$CRATES_IO_API/$crate_name/versions" 2>/dev/null)

    if [[ $? -ne 0 ]]; then
        echo ""
        return
    fi

    # Extract version numbers (requires jq or simple parsing)
    echo "$response" | grep -oE '"num":"[^"]*"' | sed 's/"num":"//g; s/"//g' | head -10
}

# Check if crate needs publishing
needs_publishing() {
    local crate=$1
    local version

    version=$(get_crate_version "$crate")

    if [[ -z "$version" ]]; then
        log_error "Could not determine version for $crate"
        return 1
    fi

    local status
    status=$(check_crates_io_version "$crate" "$version")

    case "$status" in
        "published")
            echo "already_published"
            ;;
        "not_found"|"not_published")
            echo "needs_publish"
            ;;
        *)
            echo "error"
            ;;
    esac
}

# =============================================================================
# Publishing
# =============================================================================

# Publish a single crate with retry logic
# Returns: 0 = published, 1 = error, 2 = skipped (already published)
publish_crate() {
    local crate=$1

    # Get version
    local version
    version=$(get_crate_version "$crate")

    if [[ -z "$version" ]]; then
        log_error "Could not determine version for $crate"
        return 1
    fi

    # Check if already published (unless --force)
    if [[ "$FORCE" != "true" ]]; then
        local pub_status
        pub_status=$(check_crates_io_version "$crate" "$version")

        if [[ "$pub_status" == "published" ]]; then
            echo -e "  ${CYAN}⊘${NC} $crate v$version - already on crates.io, skipping"
            return 2
        elif [[ "$pub_status" == "error" ]]; then
            log_warn "Could not check crates.io for $crate, attempting publish anyway..."
        fi
    fi

    log_info "Publishing $crate v$version..."

    cd "$PROJECT_ROOT/$crate"

    local publish_args=()
    if [[ "$NO_VERIFY" == "true" ]]; then
        publish_args+=("--no-verify")
    fi

    if [[ "$DRY_RUN" == "true" ]]; then
        log_info "[DRY RUN] Would publish: cargo publish ${publish_args[*]}"
        cd "$PROJECT_ROOT"
        return 0
    fi

    # Publish with retry on rate limit (MAX_RETRIES=0 means unlimited)
    local attempt=0
    while [[ $MAX_RETRIES -eq 0 || $attempt -lt $MAX_RETRIES ]]; do
        local output
        local exit_code

        # Capture both stdout and stderr
        output=$(cargo publish "${publish_args[@]}" 2>&1) && exit_code=0 || exit_code=$?

        if [[ $exit_code -eq 0 ]]; then
            log_success "$crate v$version published successfully"
            cd "$PROJECT_ROOT"
            return 0
        fi

        # Check if rate limited
        if is_rate_limited "$output"; then
            if handle_rate_limit $attempt "$crate"; then
                ((++attempt))
                continue
            else
                echo "$output"
                cd "$PROJECT_ROOT"
                return 1
            fi
        fi

        # Other error - print and fail
        echo "$output"
        log_error "Failed to publish $crate"
        cd "$PROJECT_ROOT"
        return 1
    done

    log_error "Max retries exceeded for $crate"
    cd "$PROJECT_ROOT"
    return 1
}

# Check if crate should be skipped
should_skip() {
    local crate=$1

    for skip in "${SKIP_CRATES[@]}"; do
        if [[ "$crate" == "$skip" ]]; then
            return 0
        fi
    done
    return 1
}

# Main publish function
publish_all() {
    log_info "Computing publish order..."

    local publish_order
    publish_order=$(compute_publish_order)
    local publish_order_array=($publish_order)
    local total_crates=${#publish_order_array[@]}

    echo ""
    log_info "Publish order ($total_crates crates):"

    # Count how many will actually be published
    local to_publish=0
    local i=1
    for crate in $publish_order; do
        local version
        version=$(get_crate_version "$crate")
        local status=""
        local version_info=""
        local will_publish=true

        if [[ -n "$version" ]]; then
            version_info=" (v$version)"

            # Check crates.io status
            local pub_status
            pub_status=$(check_crates_io_version "$crate" "$version")

            if [[ "$pub_status" == "published" ]]; then
                status=" ${CYAN}[on crates.io]${NC}"
                will_publish=false
            elif [[ "$pub_status" == "not_found" ]]; then
                status=" ${GREEN}[new crate]${NC}"
            elif [[ "$pub_status" == "not_published" ]]; then
                status=" ${GREEN}[new version]${NC}"
            fi
        fi

        if should_skip "$crate"; then
            status=" ${YELLOW}(skip)${NC}"
            will_publish=false
        fi

        if [[ "$will_publish" == "true" ]]; then
            ((++to_publish))
        fi

        echo -e "  $i. $crate$version_info$status"
        ((++i))
    done
    echo ""

    # Show rate limiting configuration
    log_info "Rate limiting configuration:"
    echo "  • Delay between publishes: ${PUBLISH_DELAY}s"
    echo "  • Burst size: $BURST_SIZE crates"
    echo "  • Burst delay: ${BURST_DELAY}s"
    if [[ $MAX_RETRIES -eq 0 ]]; then
        echo "  • Max retries on rate limit: unlimited"
    else
        echo "  • Max retries on rate limit: $MAX_RETRIES"
    fi
    echo ""

    # Estimate time
    if [[ $to_publish -gt 0 ]]; then
        local est_time
        est_time=$(estimate_publish_time $total_crates $to_publish)
        log_info "Estimated time: $(format_time $est_time) for $to_publish crates"
        echo ""
    fi

    if [[ "$DRY_RUN" == "true" ]]; then
        log_info "Dry run complete. No packages were published."
        return 0
    fi

    if [[ "$CHECK_ONLY" == "true" ]]; then
        check_all_crates
        return $?
    fi

    # Confirm before publishing
    if [[ -z "$SINGLE_CRATE" ]]; then
        echo -e "${YELLOW}This will publish $to_publish crates to crates.io.${NC}"
        echo -n "Continue? [y/N] "
        read -r confirm
        if [[ "$confirm" != "y" && "$confirm" != "Y" ]]; then
            log_info "Aborted."
            return 0
        fi
    fi

    echo ""
    log_info "Starting publish..."
    echo ""

    # Publish crates
    local started=false
    local published=0
    local skipped=0
    local already_published=0
    local failed=0
    local current=0

    for crate in $publish_order; do
        ((++current))

        # Handle --from flag
        if [[ -n "$FROM_CRATE" && "$started" == "false" ]]; then
            if [[ "$crate" == "$FROM_CRATE" ]]; then
                started=true
            else
                log_info "Skipping $crate (before --from)"
                ((++skipped))
                continue
            fi
        fi

        # Handle --single flag
        if [[ -n "$SINGLE_CRATE" && "$crate" != "$SINGLE_CRATE" ]]; then
            continue
        fi

        # Handle --skip flag
        if should_skip "$crate"; then
            log_warn "Skipping $crate (--skip)"
            ((++skipped))
            continue
        fi

        # Show progress
        show_progress $current $total_crates
        echo ""

        # Publish the crate (capture exit code without triggering set -e)
        local result
        publish_crate "$crate" && result=0 || result=$?

        case $result in
            0)
                ((++published))
                ((++BURST_COUNT))
                ;;
            1)
                ((++failed))
                log_error "Failed to publish $crate"
                ;;
            2)
                ((++already_published))
                ;;
        esac

        # Exit if single crate mode
        if [[ -n "$SINGLE_CRATE" ]]; then
            break
        fi

        # Rate limiting: delay after each publish
        if [[ "$DRY_RUN" != "true" && $result -eq 0 ]]; then
            # Check if we need a burst delay
            if [[ $BURST_COUNT -ge $BURST_SIZE ]]; then
                BURST_COUNT=0
                wait_with_countdown $BURST_DELAY "Burst limit reached ($BURST_SIZE crates)"
            else
                # Standard delay
                wait_with_countdown $PUBLISH_DELAY "Indexing delay"
            fi
        fi
    done

    # Final summary
    local end_time=$(date +%s)
    local elapsed=$((end_time - START_TIME))

    echo ""
    echo "═══════════════════════════════════════════════════════════════════════"
    log_success "Publishing complete!"
    echo "═══════════════════════════════════════════════════════════════════════"
    echo ""
    echo "  📦 Published:           $published"
    echo "  ✓  Already on crates.io: $already_published"
    echo "  ⊘  Skipped:             $skipped"
    if [[ $failed -gt 0 ]]; then
        echo -e "  ${RED}✗  Failed:              $failed${NC}"
    fi
    echo ""
    echo "  ⏱  Total time:          $(format_time $elapsed)"
    echo "  ⏳ Wait time:           $(format_time $TOTAL_WAIT_TIME)"
    echo ""
}

# =============================================================================
# Main
# =============================================================================

if [[ "$CHECK_ONLY" == "true" ]]; then
    check_all_crates
else
    publish_all
fi

