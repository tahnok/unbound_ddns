# GitHub CI Poll

This skill polls GitHub CI/CD checks and reports their status.

## Usage

When invoked, poll for GitHub CI check results using the GitHub API. The skill should:

1. **Determine what to check:**
   - If user provides a commit SHA, use that
   - If user provides a PR number, fetch the HEAD SHA for that PR
   - Otherwise, use the current HEAD commit (`git rev-parse HEAD`)

2. **Get repository info:**
   - Extract owner/repo from git remote (format: `owner/repo`)
   - Use: `git remote get-url origin` and parse the URL

3. **Poll CI checks:**
   - Use API endpoint: `https://api.github.com/repos/{owner}/{repo}/commits/{sha}/check-runs`
   - Poll every 30 seconds (configurable)
   - Default timeout: 10 minutes (configurable)

4. **Parse and display results:**
   - Show each check's name and status
   - Status can be: `queued`, `in_progress`, or `completed`
   - For completed checks, show conclusion: `success`, `failure`, `neutral`, `cancelled`, `skipped`, `timed_out`, `action_required`
   - Display with appropriate symbols:
     - `⏳` for queued
     - `⏵` for in_progress
     - `✓` for success
     - `✗` for failure
     - `○` for neutral/skipped
     - `⊗` for cancelled/timed_out

5. **Polling loop:**
   - Continue polling while any check has status != `completed`
   - Show live updates as checks progress
   - Exit when all checks complete or timeout reached

6. **Final summary:**
   - Show overall result: "All checks passed ✓" or "Some checks failed ✗"
   - For failed checks, include the `html_url` so user can view details
   - Exit with appropriate status code

## Example Output Format

```
Checking CI for commit abc1234...

⏵ Test (in_progress)
⏵ Build (in_progress)
⏳ Deploy (queued)

[After 30s]
✓ Test (success)
⏵ Build (in_progress)
⏳ Deploy (queued)

[After 60s]
✓ Test (success)
✓ Build (success)
⏵ Deploy (in_progress)

[After 90s]
✓ Test (success)
✓ Build (success)
✓ Deploy (success)

All checks passed ✓
```

## Parameters

The user may specify:
- `--commit <sha>` - Check specific commit
- `--pr <number>` - Check specific PR's HEAD commit
- `--timeout <seconds>` - Override default 600s timeout
- `--interval <seconds>` - Override default 30s polling interval
- `--once` - Check once without polling

## Implementation Notes

- Use `curl` for API requests (no auth needed for public repos, will use existing auth for private)
- Parse JSON with `jq` if available, otherwise use `grep`/`sed`
- Handle rate limiting gracefully
- Show elapsed time
- Handle case where no checks are configured (not an error)

## Error Handling

- If API returns error, display message and exit
- If timeout reached, show current status and exit with code 2
- If checks fail, exit with code 1
- If all checks pass, exit with code 0
