#!/bin/bash
# Integration test runner for unbound_ddns
# Can be run standalone or in CI/CD environments

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
IMAGE_NAME="unbound-ddns-integration-test"
CONTAINER_NAME="unbound-ddns-test-$$"
TEST_TIMEOUT=60

# Change to project root (parent of integration_test)
cd "$(dirname "$0")/.."

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}Unbound DDNS Integration Test Runner${NC}"
echo -e "${BLUE}========================================${NC}\n"

# Cleanup function
cleanup() {
    echo -e "\n${YELLOW}Cleaning up...${NC}"

    # Stop and remove container if it exists
    if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
        echo "Stopping container ${CONTAINER_NAME}..."
        docker stop "${CONTAINER_NAME}" >/dev/null 2>&1 || true
        docker rm "${CONTAINER_NAME}" >/dev/null 2>&1 || true
    fi

    echo -e "${GREEN}Cleanup complete${NC}"
}

# Set up cleanup on exit
trap cleanup EXIT

# Step 1: Build Docker image
echo -e "${BLUE}Step 1: Building Docker image...${NC}"
docker build -t "${IMAGE_NAME}" -f integration_test/Dockerfile .

if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓ Docker image built successfully${NC}\n"
else
    echo -e "${RED}✗ Failed to build Docker image${NC}"
    exit 1
fi

# Step 2: Start container
echo -e "${BLUE}Step 2: Starting container...${NC}"
docker run -d \
    --name "${CONTAINER_NAME}" \
    -p 3000:3000 \
    -p 53:53/udp \
    -p 53:53/tcp \
    "${IMAGE_NAME}"

if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓ Container started: ${CONTAINER_NAME}${NC}\n"
else
    echo -e "${RED}✗ Failed to start container${NC}"
    exit 1
fi

# Step 3: Wait for services to be ready
echo -e "${BLUE}Step 3: Waiting for services to be ready...${NC}"
sleep 5

# Check if container is still running
if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    echo -e "${RED}✗ Container stopped unexpectedly${NC}"
    echo -e "${YELLOW}Container logs:${NC}"
    docker logs "${CONTAINER_NAME}"
    exit 1
fi

# Check if the API is responding
MAX_ATTEMPTS=30
ATTEMPT=0
while [ $ATTEMPT -lt $MAX_ATTEMPTS ]; do
    if docker exec "${CONTAINER_NAME}" curl -s http://localhost:3000/update >/dev/null 2>&1; then
        break
    fi
    ATTEMPT=$((ATTEMPT + 1))
    sleep 1
done

if [ $ATTEMPT -eq $MAX_ATTEMPTS ]; then
    echo -e "${RED}✗ Service did not start in time${NC}"
    echo -e "${YELLOW}Container logs:${NC}"
    docker logs "${CONTAINER_NAME}"
    exit 1
fi

echo -e "${GREEN}✓ Services are ready${NC}\n"

# Step 4: Run integration tests
echo -e "${BLUE}Step 4: Running integration tests...${NC}"
docker exec "${CONTAINER_NAME}" python3 /build/integration_test/test.py

TEST_EXIT_CODE=$?

# Step 5: Show results
echo -e "\n${BLUE}========================================${NC}"
if [ $TEST_EXIT_CODE -eq 0 ]; then
    echo -e "${GREEN}✓ All tests passed!${NC}"
    echo -e "${BLUE}========================================${NC}\n"
    exit 0
else
    echo -e "${RED}✗ Tests failed${NC}"
    echo -e "${BLUE}========================================${NC}\n"

    echo -e "${YELLOW}Container logs:${NC}"
    docker logs "${CONTAINER_NAME}"

    exit 1
fi
