#!/bin/bash
set -e

echo "=== Starting Unbound DDNS Integration Test Environment ==="

# Setup unbound-control certificates
echo "Setting up unbound-control certificates..."
unbound-control-setup -d /etc/unbound

# Start Unbound in the background
echo "Starting Unbound DNS server..."
unbound -d -c /etc/unbound/unbound.conf &
UNBOUND_PID=$!

# Wait for Unbound to start
echo "Waiting for Unbound to be ready..."
sleep 3

# Verify Unbound is running
if ! unbound-control status > /dev/null 2>&1; then
    echo "ERROR: Unbound failed to start properly"
    exit 1
fi
echo "Unbound is running (PID: $UNBOUND_PID)"

# Start the unbound_ddns service in the foreground
echo "Starting unbound_ddns service..."
cd /etc/unbound_ddns
exec /usr/local/bin/unbound_ddns config.toml
