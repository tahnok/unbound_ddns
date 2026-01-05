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
- Direct links to failed check details
- Configurable polling interval and timeout
- Exit codes: 0 for success, 1 for failure, 2 for timeout

### Requirements

- `curl` - for API requests
- `jq` - for JSON parsing
- `git` - to determine repository and commit info
