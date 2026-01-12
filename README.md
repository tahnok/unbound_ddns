# unbound_ddns

A lightweight self-hosted Dynamic DNS server that integrates with Unbound DNS.

This app is am experiment in writing code using Claude Code from my phone while nap trapped by my newborn. It is entirely written by AI. Do what you will with that information 

## Overview

This server allows you to easily update DNS records for your self-hosted Unbound DNS server when your IP address changes. Perfect for home servers, dynamic IP connections, or any scenario where you need to keep DNS records in sync with changing IP addresses.

## How it Works

Clients make HTTP requests to the server (typically using `curl` from a cron job or network event) to update their DNS records. The server:

1. Authenticates the request using a simple secret key
2. Extracts the domain name to update
3. Determines the new IP address (either from the request body or by detecting the client's IP)
4. Updates the `local-data` entry in the Unbound configuration file for the specified domain (e.g., `local-data: "home.example.com. IN A 203.0.113.42"`)
5. Issues a reload command to Unbound to apply the changes without downtime

## Limitations

- **IPv4 only**: Currently only IPv4 addresses are supported. The server creates DNS A records and will reject IPv6 addresses. IPv6/AAAA record support may be added in a future release.

## API

### Update DNS Record

**Endpoint:** `POST /update`

**Headers:**
- `Authorization` (required) - Authentication key in the format `Bearer <key>` or just `<key>`

**Parameters:**
- `domain` (required) - The domain name to update
- `ip` (optional) - The new IP address. If omitted, the server will use the client's IP address from the request

**Content Types:** The server accepts both `application/x-www-form-urlencoded` (form data) and `application/json`.

**Example Usage with Form Data:**

```bash
# Update with explicit IP
curl -X POST https://your-server.com/update \
  -H "Authorization: Bearer your-secret-key" \
  -d "domain=home.example.com" \
  -d "ip=203.0.113.42"

# Update using client's IP (auto-detected)
curl -X POST https://your-server.com/update \
  -H "Authorization: Bearer your-secret-key" \
  -d "domain=home.example.com"
```

**Example Usage with JSON:**

```bash
# Update with explicit IP
curl -X POST https://your-server.com/update \
  -H "Authorization: Bearer your-secret-key" \
  -H "Content-Type: application/json" \
  -d '{"domain":"home.example.com","ip":"203.0.113.42"}'

# Update using client's IP (auto-detected)
curl -X POST https://your-server.com/update \
  -H "Authorization: Bearer your-secret-key" \
  -H "Content-Type: application/json" \
  -d '{"domain":"home.example.com"}'
```

## Installation

### Building from Source

1. **Install Rust** (if not already installed):
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Clone the repository**:
   ```bash
   git clone https://github.com/tahnok/unbound_ddns.git
   cd unbound_ddns
   ```

3. **Build the release binary**:
   ```bash
   cargo build --release
   ```

4. **Install the binary**:
   ```bash
   sudo cp target/release/unbound_ddns /usr/local/bin/
   sudo chmod +x /usr/local/bin/unbound_ddns
   ```

### Configuration Setup

1. **Create a configuration file** at `/etc/unbound_ddns/config.toml`:
   ```bash
   sudo mkdir -p /etc/unbound_ddns
   sudo nano /etc/unbound_ddns/config.toml
   ```

2. **Add your configuration** (see Configuration section below for details):

   ```toml
   # Path to the Unbound configuration file to update
   unbound_config_path = "/etc/unbound/unbound.conf"

   # Authorized domains and their secret keys
   [[domains]]
   name = "home.example.com"
   key = "your-secret-key-here"

   [[domains]]
   name = "server.example.com"
   key = "another-secret-key"
   ```

3. **Ensure proper permissions**:
   ```bash
   sudo chown root:root /etc/unbound_ddns/config.toml
   sudo chmod 600 /etc/unbound_ddns/config.toml
   ```

### Systemd Service

Create a systemd service to run unbound_ddns automatically.

1. **Create the service file** at `/etc/systemd/system/unbound_ddns.service`:

```ini
[Unit]
Description=Unbound Dynamic DNS Server
After=network.target unbound.service
Wants=network.target

[Service]
Type=simple
User=root
WorkingDirectory=/etc/unbound_ddns
ExecStart=/usr/local/bin/unbound_ddns
Restart=on-failure
RestartSec=5s

# Log level: error, warn, info, debug, trace (default: info)
Environment=RUST_LOG=info

# Security hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/etc/unbound

# Allow binding to privileged ports if needed
AmbientCapabilities=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
```

2. **Enable and start the service**:
   ```bash
   sudo systemctl daemon-reload
   sudo systemctl enable unbound_ddns
   sudo systemctl start unbound_ddns
   ```

3. **Check the service status**:
   ```bash
   sudo systemctl status unbound_ddns
   sudo journalctl -u unbound_ddns -f
   ```

**Note:** The service runs as root because it needs to:
- Write to the Unbound configuration file
- Reload the Unbound service

For added security, consider using group permissions and allowing a dedicated user to modify the Unbound configuration instead.

## Configuration

The server is configured using a TOML configuration file.

**Example configuration:**

```toml
# Path to the Unbound configuration file to update
unbound_config_path = "/etc/unbound/unbound.conf"

# Authorized domains and their secret keys
[[domains]]
name = "home.example.com"
key = "secret-key-1"

[[domains]]
name = "server.example.com"
key = "secret-key-2"

[[domains]]
name = "vpn.example.com"
key = "secret-key-3"
```

**Configuration options:**
- `unbound_config_path` - Path to the Unbound configuration file that will be updated
- `domains` - Array of domain configurations, each containing:
  - `name` - The domain name that can be updated
  - `key` - The secret key required to authenticate updates for this domain

## License

MIT
