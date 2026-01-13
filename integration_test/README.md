# Integration Test

This directory contains a Docker-based integration test for `unbound_ddns` that verifies the complete workflow of updating DNS records through the HTTP API and resolving them via Unbound DNS.

## Overview

The integration test runs in a Docker container that includes:
- **Unbound DNS server** - A fully functional DNS server
- **unbound_ddns service** - The dynamic DNS update service
- **Test suite** - Python-based tests that verify end-to-end functionality

## Test Scenarios

The test suite verifies:

1. **Update with explicit IP** - Updates DNS records with a specified IP address
2. **Update with auto-detected IP** - Updates DNS records using the client's IP address
3. **DNS resolution** - Queries Unbound to verify records were updated correctly
4. **Authentication** - Validates that API authentication works properly
5. **Unbound reload** - Confirms that Unbound reloads configuration after updates

## Files

- `Dockerfile` - Multi-stage Docker image with Unbound and unbound_ddns
- `unbound.conf` - Unbound DNS server configuration with test domains
- `config.toml` - unbound_ddns configuration with test credentials
- `start.sh` - Container startup script that launches both services
- `test.py` - Python integration test suite
- `run-test.sh` - Test runner script for local and CI execution

## Running the Tests

### Prerequisites

- Docker installed and running
- Bash shell (for the runner script)

### Run Locally

From the project root directory:

```bash
./integration_test/run-test.sh
```

This script will:
1. Build the Docker image
2. Start a container with Unbound and unbound_ddns
3. Execute the test suite
4. Clean up containers automatically

### Run in GitHub Actions

The integration test runs automatically in CI via the `integration-test` job in `.github/workflows/ci.yml`.

## Manual Testing

If you want to manually interact with the test environment:

### Start the container

```bash
docker build -t unbound-ddns-integration-test -f integration_test/Dockerfile .
docker run -d --name test-container -p 3000:3000 -p 53:53/udp unbound-ddns-integration-test
```

### Test the API

```bash
# Update a DNS record
curl -X POST http://localhost:3000/update \
  -H "Authorization: Bearer test-secret-key-123" \
  -H "Content-Type: application/json" \
  -d '{"domain":"test.example.com","ip":"10.0.0.50"}'
```

### Query DNS

```bash
# Query the DNS server
dig @localhost test.example.com
# or
nslookup test.example.com localhost
```

### Run tests manually

```bash
docker exec test-container python3 /build/integration_test/test.py
```

### View logs

```bash
# Application logs
docker logs test-container

# Enter container for debugging
docker exec -it test-container /bin/bash
```

### Cleanup

```bash
docker stop test-container
docker rm test-container
```

## Test Credentials

The following test domains and keys are configured:

| Domain | API Key |
|--------|---------|
| test.example.com | test-secret-key-123 |
| home.example.com | home-secret-key-456 |
| auto.example.com | auto-secret-key-789 |

## Architecture

```
┌─────────────────────────────────────────┐
│         Docker Container                │
│                                         │
│  ┌──────────────┐   ┌───────────────┐  │
│  │   Unbound    │   │ unbound_ddns  │  │
│  │  DNS Server  │◄──┤  HTTP API     │  │
│  │   (port 53)  │   │  (port 3000)  │  │
│  └──────────────┘   └───────────────┘  │
│         ▲                   ▲           │
│         │                   │           │
│         │  DNS Query        │  HTTP     │
│         │                   │  Request  │
└─────────┼───────────────────┼───────────┘
          │                   │
    ┌─────┴───────────────────┴─────┐
    │      Integration Tests         │
    │         (test.py)              │
    └────────────────────────────────┘
```

## Troubleshooting

### Container fails to start

Check the container logs:
```bash
docker logs <container-name>
```

Common issues:
- Port 53 or 3000 already in use - stop other services using these ports
- Unbound fails to generate certificates - check disk space and permissions

### Tests fail

1. Verify services are running:
   ```bash
   docker exec <container-name> curl http://localhost:3000/update
   docker exec <container-name> unbound-control status
   ```

2. Check Unbound configuration:
   ```bash
   docker exec <container-name> cat /etc/unbound/unbound.conf
   ```

3. Check unbound_ddns logs in container logs

### DNS queries fail

Ensure the container is exposing port 53:
```bash
docker port <container-name>
```

Test DNS from inside the container:
```bash
docker exec <container-name> dig @localhost test.example.com
```

## Contributing

When modifying the integration test:

1. Update test cases in `test.py` for new functionality
2. Update `config.toml` and `unbound.conf` if adding new test domains
3. Run tests locally before pushing
4. Ensure GitHub Actions passes all tests
