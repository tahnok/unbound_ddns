# Docker-based Integration Testing

This directory contains a Docker-based setup for integration testing with unbound.

## Overview

The Docker approach provides a more reliable testing environment by:
- Running unbound in an isolated container
- Ensuring consistent configuration across different CI environments
- Avoiding permission and path issues that can occur with direct unbound installation

## Files

- `Dockerfile` - Builds an Ubuntu-based image with unbound installed
- `docker-compose.yml` - Orchestrates the unbound container
- `unbound-test.conf` - Unbound configuration for testing
- `run-integration-test.sh` - Script to run the full integration test suite

## Usage

### Running Locally

```bash
# From the project root
cd tests/docker
./run-integration-test.sh
```

### Manual Testing

```bash
# Start unbound container
docker-compose up -d

# Check if it's running
docker-compose ps

# View logs
docker-compose logs

# Test DNS query
dig @127.0.0.1 -p 15353 integration-test.example.com

# Stop and cleanup
docker-compose down
```

## CI Integration

The GitHub Actions workflow uses this Docker setup automatically. See `.github/workflows/ci.yml` for the integration test job.

## Troubleshooting

If the test fails:

1. Check if Docker is running: `docker ps`
2. View unbound logs: `docker-compose logs`
3. Test DNS manually: `dig @127.0.0.1 -p 15353 integration-test.example.com`
4. Verify port is not in use: `lsof -i :15353`
