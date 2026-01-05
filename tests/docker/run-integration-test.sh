#!/bin/bash
# Script to run integration tests with Docker-based unbound

set -e

echo "=== Starting Docker-based Integration Test ==="

# Navigate to docker directory
cd "$(dirname "$0")"

# Build and start unbound container
echo "Starting unbound container..."
docker-compose up -d

# Wait for unbound to be ready
echo "Waiting for unbound to start..."
sleep 5

# Check if unbound is responding
echo "Checking if unbound is responding..."
if ! docker-compose ps | grep -q "Up"; then
    echo "ERROR: Unbound container failed to start"
    docker-compose logs
    docker-compose down
    exit 1
fi

# Run the integration test from the project root
echo "Running integration test..."
cd ../..
cargo test test_integration_with_real_unbound_and_dns_query -- --ignored --nocapture

# Store test result
TEST_RESULT=$?

# Cleanup
echo "Cleaning up Docker containers..."
cd tests/docker
docker-compose down

# Exit with test result
if [ $TEST_RESULT -eq 0 ]; then
    echo "=== Integration test PASSED ==="
else
    echo "=== Integration test FAILED ==="
fi

exit $TEST_RESULT
