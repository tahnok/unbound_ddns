# Dockerfile for integration testing with unbound
FROM ubuntu:24.04

# Install unbound and required tools
RUN apt-get update && \
    apt-get install -y \
    unbound \
    dns-root-data \
    && rm -rf /var/lib/apt/lists/*

# Create directory for test configs
RUN mkdir -p /test-config

# Copy test unbound config
COPY unbound-test.conf /test-config/unbound.conf

# Expose DNS port
EXPOSE 15353/udp
EXPOSE 15353/tcp

# Run unbound in foreground
CMD ["unbound", "-d", "-c", "/test-config/unbound.conf"]
