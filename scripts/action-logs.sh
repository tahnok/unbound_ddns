#!/bin/bash
# GitHub Action Logs Script
# Fetches logs for a given GitHub Actions check run

set -euo pipefail

# Default configuration
CHECK_RUN_ID=""
REPO=""
DOWNLOAD=false

usage() {
    cat << EOF
Usage: $0 [OPTIONS] <check-run-id>

Fetch logs for a GitHub Actions check run.

ARGUMENTS:
    <check-run-id>      The ID of the check run to fetch logs for

OPTIONS:
    --repo <owner/repo> Specify repository (default: auto-detect from git)
    --download          Download logs as zip file instead of displaying
    -h, --help          Show this help message

EXAMPLES:
    $0 12345678                    # Fetch logs for check run 12345678
    $0 --repo owner/repo 12345678  # Fetch logs for specific repo
    $0 --download 12345678         # Download logs as zip file

NOTE:
    The check run ID can be obtained from the ci-poll.sh script output.
EOF
    exit 0
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --repo)
            REPO="$2"
            shift 2
            ;;
        --download)
            DOWNLOAD=true
            shift
            ;;
        -h|--help)
            usage
            ;;
        -*)
            echo "Unknown option: $1"
            usage
            ;;
        *)
            CHECK_RUN_ID="$1"
            shift
            ;;
    esac
done

# Validate required arguments
if [[ -z "$CHECK_RUN_ID" ]]; then
    echo "Error: check-run-id is required" >&2
    echo ""
    usage
fi

# Get repository info
get_repo_info() {
    if [[ -n "$REPO" ]]; then
        echo "$REPO"
        return
    fi

    local remote_url
    remote_url=$(git remote get-url origin 2>/dev/null || echo "")

    if [[ -z "$remote_url" ]]; then
        echo "Error: Not a git repository or no remote configured. Use --repo option." >&2
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

# Fetch check run details
fetch_check_run() {
    local repo_info=$1
    local check_run_id=$2

    if [[ -n "${GITHUB_TOKEN:-}" ]]; then
        curl -s -H "Authorization: token $GITHUB_TOKEN" \
            "https://api.github.com/repos/$repo_info/check-runs/$check_run_id"
    else
        curl -s "https://api.github.com/repos/$repo_info/check-runs/$check_run_id"
    fi
}

# Get workflow run ID from check run
get_workflow_run_id() {
    local check_run_json=$1

    # Extract run_id from details_url (this is the workflow run ID)
    local run_id
    run_id=$(echo "$check_run_json" | jq -r '.details_url' | grep -oP 'runs/\K[0-9]+' || echo "")

    echo "$run_id"
}

# List jobs for a workflow run
list_jobs() {
    local repo_info=$1
    local run_id=$2

    if [[ -n "${GITHUB_TOKEN:-}" ]]; then
        curl -s -H "Authorization: token $GITHUB_TOKEN" \
            "https://api.github.com/repos/$repo_info/actions/runs/$run_id/jobs"
    else
        curl -s "https://api.github.com/repos/$repo_info/actions/runs/$run_id/jobs"
    fi
}

# Fetch job logs
fetch_job_logs() {
    local repo_info=$1
    local job_id=$2

    if [[ -n "${GITHUB_TOKEN:-}" ]]; then
        curl -sL -H "Authorization: token $GITHUB_TOKEN" \
            "https://api.github.com/repos/$repo_info/actions/jobs/$job_id/logs"
    else
        curl -sL "https://api.github.com/repos/$repo_info/actions/jobs/$job_id/logs"
    fi
}

# Download workflow run logs
download_workflow_logs() {
    local repo_info=$1
    local run_id=$2
    local output_file="logs-run-${run_id}.zip"

    echo "Downloading logs to ${output_file}..."

    if [[ -n "${GITHUB_TOKEN:-}" ]]; then
        curl -L \
            -H "Accept: application/vnd.github+json" \
            -H "Authorization: token $GITHUB_TOKEN" \
            "https://api.github.com/repos/$repo_info/actions/runs/$run_id/logs" \
            -o "$output_file"
    else
        curl -L \
            -H "Accept: application/vnd.github+json" \
            "https://api.github.com/repos/$repo_info/actions/runs/$run_id/logs" \
            -o "$output_file"
    fi

    if [[ -f "$output_file" ]]; then
        echo "Downloaded logs to: ${output_file}"
    else
        echo "Failed to download logs" >&2
        exit 1
    fi
}

# Display logs
display_logs() {
    local logs=$1
    local job_name=$2

    echo ""
    echo "==================================================="
    echo "Logs for: ${job_name}"
    echo "==================================================="
    echo ""
    echo "$logs"
}

# Main function
main() {
    local repo_info
    local check_run_json
    local run_id
    local jobs_json
    local job_count

    repo_info=$(get_repo_info)

    echo "Repository: ${repo_info}"
    echo "Check Run ID: ${CHECK_RUN_ID}"
    echo ""

    # Fetch check run details
    echo "Fetching check run details..."
    check_run_json=$(fetch_check_run "$repo_info" "$CHECK_RUN_ID")

    # Check for API errors
    if echo "$check_run_json" | jq -e '.message' > /dev/null 2>&1; then
        local error_msg
        error_msg=$(echo "$check_run_json" | jq -r '.message')
        echo "API Error: $error_msg" >&2
        exit 1
    fi

    # Extract check run info
    local check_name
    local check_status
    check_name=$(echo "$check_run_json" | jq -r '.name')
    check_status=$(echo "$check_run_json" | jq -r '.status')

    echo "Check Name: ${check_name}"
    echo "Status: ${check_status}"
    echo ""

    # Get workflow run ID
    run_id=$(get_workflow_run_id "$check_run_json")

    if [[ -z "$run_id" || "$run_id" == "null" ]]; then
        echo "Error: Could not determine workflow run ID from check run" >&2
        echo "This check run may not be associated with a GitHub Actions workflow." >&2
        exit 1
    fi

    echo "Workflow Run ID: ${run_id}"
    echo ""

    # Check if download mode
    if [[ "$DOWNLOAD" == "true" ]]; then
        download_workflow_logs "$repo_info" "$run_id"
        exit 0
    fi

    # List jobs
    echo "Fetching jobs for workflow run..."
    jobs_json=$(list_jobs "$repo_info" "$run_id")

    # Check for API errors
    if echo "$jobs_json" | jq -e '.message' > /dev/null 2>&1; then
        local error_msg
        error_msg=$(echo "$jobs_json" | jq -r '.message')
        echo "API Error: $error_msg" >&2
        exit 1
    fi

    job_count=$(echo "$jobs_json" | jq -r '.total_count')

    if [[ "$job_count" == "0" ]]; then
        echo "No jobs found for this workflow run"
        exit 0
    fi

    echo "Found ${job_count} job(s)"
    echo ""

    # Fetch and display logs for each job
    echo "$jobs_json" | jq -r '.jobs[] | "\(.id)|\(.name)|\(.status)"' | \
    while IFS='|' read -r job_id job_name job_status; do
        echo "Fetching logs for job: ${job_name} (${job_status})..."

        local logs
        logs=$(fetch_job_logs "$repo_info" "$job_id")

        if [[ -n "$logs" ]]; then
            display_logs "$logs" "$job_name"
        else
            echo "No logs available for this job yet"
            echo ""
        fi
    done

    echo ""
    echo "Logs fetch complete"
}

main
