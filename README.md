# unbound_ddns

A lightweight self-hosted Dynamic DNS server that integrates with Unbound DNS.

## Overview

This server allows you to easily update DNS records for your self-hosted Unbound DNS server when your IP address changes. Perfect for home servers, dynamic IP connections, or any scenario where you need to keep DNS records in sync with changing IP addresses.

## How it Works

Clients make HTTP requests to the server (typically using `curl` from a cron job or network event) to update their DNS records. The server:

1. Authenticates the request using a simple secret key
2. Extracts the domain name to update
3. Determines the new IP address (either from the request body or by detecting the client's IP)
4. Updates the Unbound DNS configuration accordingly

## API

### Update DNS Record

**Endpoint:** `POST /update`

**Parameters:**
- `key` (required) - Authentication secret key
- `domain` (required) - The domain name to update
- `ip` (optional) - The new IP address. If omitted, the server will use the client's IP address from the request

**Example Usage:**

```bash
# Update with explicit IP
curl -X POST https://your-server.com/update \
  -d "key=your-secret-key" \
  -d "domain=home.example.com" \
  -d "ip=203.0.113.42"

# Update using client's IP (auto-detected)
curl -X POST https://your-server.com/update \
  -d "key=your-secret-key" \
  -d "domain=home.example.com"
```

## Setup

_(Coming soon)_

## Configuration

_(Coming soon)_

## License

_(Coming soon)_
