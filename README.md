# unbound_ddns

A lightweight self-hosted Dynamic DNS server that integrates with Unbound DNS.

## Overview

This server allows you to easily update DNS records for your self-hosted Unbound DNS server when your IP address changes. Perfect for home servers, dynamic IP connections, or any scenario where you need to keep DNS records in sync with changing IP addresses.

## How it Works

Clients make HTTP requests to the server (typically using `curl` from a cron job or network event) to update their DNS records. The server:

1. Authenticates the request using a simple secret key
2. Extracts the domain name to update
3. Determines the new IP address (either from the request body or by detecting the client's IP)
4. Updates the `local-data` entry in the Unbound configuration file for the specified domain (e.g., `local-data: "home.example.com. IN A 203.0.113.42"`)
5. Issues a reload command to Unbound to apply the changes without downtime

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

## Setup

_(Coming soon)_

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
