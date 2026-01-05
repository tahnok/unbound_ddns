#!/bin/bash
# GitHub CI Polling Script
# Polls GitHub Actions check runs and waits for completion

set -euo pipefail

# Default configuration
INTERVAL=30
TIMEOUT=600
ONCE=false
COMMIT=""
PR=""
VERBOSE=false

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Symbols
SYM_QUEUED="⏳"
SYM_PROGRESS="⏵"
SYM_SUCCESS="✓"
SYM_FAILURE="✗"
SYM_NEUTRAL="○"

usage() {
    cat << EOF
Usage: $0 [OPTIONS]

Poll GitHub CI/CD checks and wait for completion.

OPTIONS:
    --commit <sha>      Check specific commit SHA
    --pr <number>       Check specific PR number
    --interval <sec>    Polling interval in seconds (default: 30)
    --timeout <sec>     Timeout in seconds (default: 600)
    --once              Check once without polling
    --verbose           Show check run IDs (for use with action-logs.sh)
    -h, --help          Show this help message

EXAMPLES:
    $0                          # Poll current commit
    $0 --once                   # Check current commit once
    $0 --commit abc123          # Poll specific commit
    $0 --pr 42                  # Poll PR #42
    $0 --interval 10 --timeout 300  # Custom timing
EOF
    exit 0
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --commit)
            COMMIT="$2"
            shift 2
            ;;
        --pr)
            PR="$2"
            shift 2
            ;;
        --interval)
            INTERVAL="$2"
            shift 2
            ;;
        --timeout)
            TIMEOUT="$2"
            shift 2
            ;;
        --once)
            ONCE=true
            shift
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Unknown option: $1"
            usage
            ;;
    esac
done

# Get repository info
get_repo_info() {
    local remote_url
    remote_url=$(git remote get-url origin 2>/dev/null || echo "")

    if [[ -z "$remote_url" ]]; then
        echo "Error: Not a git repository or no remote configured" >&2
        exit 1
    fi

    # Parse owner/repo from various URL formats
    if [[ "$remote_url" =~ github\.com[:/]([^/]+)/([^/.]+) ]]; then
        echo "${BASH_REMATCH[1]}/${BASH_REMATCH[2]}"
    elif [[ "$remote_url" =~ /git/([^/]+)/([^/]+) ]]; then
        # Handle local proxy format
        echo "${BASH_REMATCH[1]}/${BASH_REMATCH[2]}"
    else
        echo "Error: Could not parse repository from remote URL: $remote_url" >&2
        exit 1
    fi
}

# Get commit SHA
get_commit_sha() {
    if [[ -n "$COMMIT" ]]; then
        echo "$COMMIT"
    elif [[ -n "$PR" ]]; then
        local repo_info
        repo_info=$(get_repo_info)
        echo "Fetching HEAD SHA for PR #$PR..." >&2
        curl -s "https://api.github.com/repos/$repo_info/pulls/$PR" | \
            jq -r '.head.sha' | head -1
    else
        git rev-parse HEAD
    fi
}

# Fetch check runs
fetch_checks() {
    local repo_info=$1
    local sha=$2

    curl -s "https://api.github.com/repos/$repo_info/commits/$sha/check-runs"
}

# Get status symbol
get_symbol() {
    local status=$1
    local conclusion=$2

    case "$status" in
        queued)
            echo "$SYM_QUEUED"
            ;;
        in_progress)
            echo "$SYM_PROGRESS"
            ;;
        completed)
            case "$conclusion" in
                success)
                    echo "$SYM_SUCCESS"
                    ;;
                failure|timed_out|action_required)
                    echo "$SYM_FAILURE"
                    ;;
                *)
                    echo "$SYM_NEUTRAL"
                    ;;
            esac
            ;;
        *)
            echo "?"
            ;;
    esac
}

# Get status color
get_color() {
    local status=$1
    local conclusion=$2

    case "$status" in
        completed)
            case "$conclusion" in
                success)
                    echo "$GREEN"
                    ;;
                failure|timed_out|action_required)
                    echo "$RED"
                    ;;
                *)
                    echo "$YELLOW"
                    ;;
            esac
            ;;
        in_progress)
            echo "$BLUE"
            ;;
        *)
            echo "$YELLOW"
            ;;
    esac
}

# Display checks
display_checks() {
    local json=$1
    local total_count
    local i

    total_count=$(echo "$json" | jq -r '.total_count')

    if [[ "$total_count" == "0" ]]; then
        echo "No CI checks configured for this commit."
        return 0
    fi

    echo "$json" | jq -r '.check_runs[] | "\(.id)|\(.name)|\(.status)|\(.conclusion // "null")|\(.html_url)"' | \
    while IFS='|' read -r id name status conclusion url; do
        local symbol
        local color
        symbol=$(get_symbol "$status" "$conclusion")
        color=$(get_color "$status" "$conclusion")

        printf "${color}%s %s${NC}" "$symbol" "$name"

        if [[ "$status" == "completed" ]]; then
            printf " (${conclusion})"
        else
            printf " (${status})"
        fi

        if [[ "$VERBOSE" == "true" ]]; then
            printf "\n  ${BLUE}ID: %s${NC}" "$id"
        fi

        if [[ "$status" == "completed" && "$conclusion" != "success" && "$conclusion" != "skipped" ]]; then
            printf "\n  ${BLUE}→ %s${NC}" "$url"
        fi

        echo
    done
}

# Check if all completed
all_completed() {
    local json=$1
    local pending
    pending=$(echo "$json" | jq '[.check_runs[] | select(.status != "completed")] | length')
    [[ "$pending" == "0" ]]
}

# Check if any failed
any_failed() {
    local json=$1
    local failed
    failed=$(echo "$json" | jq '[.check_runs[] | select(.status == "completed" and (.conclusion == "failure" or .conclusion == "timed_out" or .conclusion == "action_required"))] | length')
    [[ "$failed" != "0" ]]
}

# Main polling loop
main() {
    local repo_info
    local sha
    local start_time
    local elapsed

    repo_info=$(get_repo_info)
    sha=$(get_commit_sha)

    echo "Repository: $repo_info"
    echo "Commit: $sha"
    echo ""

    start_time=$(date +%s)

    while true; do
        local json
        json=$(fetch_checks "$repo_info" "$sha")

        # Check for API errors
        if echo "$json" | jq -e '.message' > /dev/null 2>&1; then
            local error_msg
            error_msg=$(echo "$json" | jq -r '.message')
            echo -e "${RED}API Error: $error_msg${NC}" >&2
            exit 1
        fi

        # Clear screen for updates (only if not first iteration and not --once)
        if [[ "$ONCE" == "false" ]] && [[ $(date +%s) -ne $start_time ]]; then
            echo -e "\n--- Update $(date +%H:%M:%S) ---"
        fi

        display_checks "$json"

        # Check if all completed
        if all_completed "$json"; then
            echo ""
            if any_failed "$json"; then
                echo -e "${RED}Some checks failed ✗${NC}"
                exit 1
            else
                echo -e "${GREEN}All checks passed ✓${NC}"
                exit 0
            fi
        fi

        # Exit if --once flag set
        if [[ "$ONCE" == "true" ]]; then
            echo ""
            echo "Checks still in progress. Use without --once to poll until completion."
            exit 0
        fi

        # Check timeout
        elapsed=$(($(date +%s) - start_time))
        if [[ $elapsed -ge $TIMEOUT ]]; then
            echo ""
            echo -e "${YELLOW}Timeout reached after ${elapsed}s${NC}"
            echo "Some checks are still running."
            exit 2
        fi

        # Wait before next poll
        echo ""
        echo "Waiting ${INTERVAL}s before next check... (elapsed: ${elapsed}s, timeout: ${TIMEOUT}s)"
        sleep "$INTERVAL"
    done
}

main
