---
description: Poll GitHub CI checks and wait for completion
---

Poll GitHub CI/CD checks for the current commit (or specified commit/PR) and wait until all checks complete.

## What to do:

1. **Get target commit:**
   - Check if user provided `--commit <sha>` argument - use that SHA
   - Check if user provided `--pr <number>` argument - fetch HEAD SHA for that PR using: `curl -s https://api.github.com/repos/{owner}/{repo}/pulls/{number} | grep -o '"sha": "[^"]*"' | head -1 | cut -d'"' -f4`
   - Otherwise use current HEAD: `git rev-parse HEAD`

2. **Get repository info:**
   - Run: `git remote get-url origin`
   - Parse owner/repo from the URL (e.g., `tahnok/unbound_ddns`)

3. **Poll CI checks:**
   - Endpoint: `https://api.github.com/repos/{owner}/{repo}/commits/{sha}/check-runs`
   - Parse the response to get check runs
   - Check every 30 seconds (or use `--interval <seconds>` if provided)
   - Timeout after 10 minutes (or use `--timeout <seconds>` if provided)

4. **Display status:**
   - For each check, show: name and status
   - Use symbols:
     - `⏳` queued
     - `⏵` in_progress
     - `✓` success
     - `✗` failure
     - `○` neutral/skipped/cancelled
   - Update display as checks progress
   - Show elapsed time

5. **Exit conditions:**
   - All checks completed → show summary and exit
   - Timeout reached → show current state and warn
   - API error → display error and exit

6. **Final summary:**
   - If all checks passed: "All checks passed ✓"
   - If any failed: "Some checks failed ✗" and list failed checks with URLs
   - If timeout: "Timeout reached, some checks still running"

## Example usage:
- `/ci-poll` - Poll current commit
- `/ci-poll --commit abc123` - Poll specific commit
- `/ci-poll --pr 42` - Poll PR #42
- `/ci-poll --once` - Check once without polling
- `/ci-poll --interval 10 --timeout 300` - Custom timing
