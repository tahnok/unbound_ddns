# Scripts

## ci-poll.sh

A bash script to poll GitHub Actions CI/CD checks and wait for completion.

### Usage

```bash
# Poll current commit until all checks complete
./scripts/ci-poll.sh

# Check current commit once without polling
./scripts/ci-poll.sh --once

# Poll specific commit
./scripts/ci-poll.sh --commit abc123

# Poll specific PR
./scripts/ci-poll.sh --pr 42

# Custom polling interval and timeout
./scripts/ci-poll.sh --interval 10 --timeout 300
```

### Options

- `--commit <sha>` - Check specific commit SHA
- `--pr <number>` - Check specific PR number
- `--interval <sec>` - Polling interval in seconds (default: 30)
- `--timeout <sec>` - Timeout in seconds (default: 600)
- `--once` - Check once without polling
- `-h, --help` - Show help message

### Features

- Color-coded output (green for success, red for failure, blue for in-progress)
- Status symbols (✓ success, ✗ failure, ⏵ in-progress, ⏳ queued)
- Displays check run IDs (for use with action-logs.sh)
- Direct links to failed check details
- Configurable polling interval and timeout
- Exit codes: 0 for success, 1 for failure, 2 for timeout

### Requirements

- `curl` - for API requests
- `jq` - for JSON parsing
- `git` - to determine repository and commit info

## action-logs.sh

A bash script to fetch logs for a GitHub Actions check run.

### Usage

```bash
# Fetch logs for a specific check run
./scripts/action-logs.sh 12345678

# Fetch logs for a specific repository
./scripts/action-logs.sh --repo owner/repo 12345678

# Download logs as a zip file
./scripts/action-logs.sh --download 12345678
```

### Options

- `<check-run-id>` - The ID of the check run to fetch logs for (required)
- `--repo <owner/repo>` - Specify repository (default: auto-detect from git)
- `--download` - Download logs as zip file instead of displaying
- `-h, --help` - Show help message

### Features

- Fetches and displays logs for GitHub Actions check runs
- Supports multiple jobs within a workflow run
- Can download logs as a zip file for offline viewing
- Auto-detects repository from git remote

### Getting Check Run IDs

Use the `ci-poll.sh` script to get check run IDs:

```bash
./scripts/ci-poll.sh
```

The script will display the check run ID for each check, which can then be used with `action-logs.sh`.

### Requirements

- `curl` - for API requests
- `jq` - for JSON parsing
- `git` - to determine repository info (optional, can use --repo)
