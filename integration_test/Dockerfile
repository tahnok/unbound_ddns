# Multi-stage Dockerfile for integration testing
# Stage 1: Build the Rust application
# Use latest stable Rust (1.92+) which supports edition 2024
FROM rust:1.92-bookworm AS builder

WORKDIR /build

# Copy Cargo files first for better caching
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build the release binary
RUN cargo build --release

# Stage 2: Runtime image with Unbound
FROM debian:bookworm-slim

# Install Unbound and required dependencies
RUN apt-get update && \
    apt-get install -y \
    unbound \
    dns-root-data \
    dnsutils \
    curl \
    python3 \
    python3-requests \
    && rm -rf /var/lib/apt/lists/*

# Create necessary directories
RUN mkdir -p /etc/unbound/unbound.conf.d /var/log/unbound

# Copy the built binary from the builder stage
COPY --from=builder /build/target/release/unbound_ddns /usr/local/bin/unbound_ddns
RUN chmod +x /usr/local/bin/unbound_ddns

# Copy configuration files
COPY integration_test/unbound.conf /etc/unbound/unbound.conf
COPY integration_test/config.toml /etc/unbound_ddns/config.toml

# Copy the startup script and test files
COPY integration_test/start.sh /start.sh
COPY integration_test/test.py /build/integration_test/test.py
RUN chmod +x /start.sh && chmod +x /build/integration_test/test.py

# Expose ports
# 53 for DNS (Unbound)
# 3000 for unbound_ddns HTTP API
EXPOSE 53/tcp 53/udp 3000/tcp

# Run the startup script
CMD ["/start.sh"]
