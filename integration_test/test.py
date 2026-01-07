#!/usr/bin/env python3
"""
Integration test for unbound_ddns with Unbound DNS server.

This script tests the complete flow:
1. Update DNS records via HTTP API
2. Verify DNS resolution through Unbound
"""

import sys
import time
import socket
import requests
from typing import Optional

# Configuration
API_BASE_URL = "http://localhost:3000"
DNS_SERVER = "127.0.0.1"
DNS_PORT = 53

# Test data
TESTS = [
    {
        "name": "Update test.example.com with explicit IP",
        "domain": "test.example.com",
        "key": "test-secret-key-123",
        "ip": "10.0.0.100",
        "expected_ip": "10.0.0.100",
    },
    {
        "name": "Update home.example.com with explicit IP",
        "domain": "home.example.com",
        "key": "home-secret-key-456",
        "ip": "10.0.0.200",
        "expected_ip": "10.0.0.200",
    },
    {
        "name": "Update auto.example.com with auto-detected IP",
        "domain": "auto.example.com",
        "key": "auto-secret-key-789",
        "ip": None,  # Will auto-detect
        "expected_ip": "127.0.0.1",  # Will come from localhost
    },
]


class Colors:
    """ANSI color codes for terminal output"""
    GREEN = '\033[92m'
    RED = '\033[91m'
    YELLOW = '\033[93m'
    BLUE = '\033[94m'
    RESET = '\033[0m'
    BOLD = '\033[1m'


def print_header(message: str):
    """Print a formatted header"""
    print(f"\n{Colors.BLUE}{Colors.BOLD}{'=' * 70}{Colors.RESET}")
    print(f"{Colors.BLUE}{Colors.BOLD}{message}{Colors.RESET}")
    print(f"{Colors.BLUE}{Colors.BOLD}{'=' * 70}{Colors.RESET}\n")


def print_success(message: str):
    """Print a success message"""
    print(f"{Colors.GREEN}✓ {message}{Colors.RESET}")


def print_error(message: str):
    """Print an error message"""
    print(f"{Colors.RED}✗ {message}{Colors.RESET}")


def print_info(message: str):
    """Print an info message"""
    print(f"{Colors.YELLOW}ℹ {message}{Colors.RESET}")


def dns_query(domain: str, server: str = DNS_SERVER, port: int = DNS_PORT) -> Optional[str]:
    """
    Query DNS for A record and return the IP address.

    Args:
        domain: Domain name to query
        server: DNS server IP
        port: DNS server port

    Returns:
        IP address string or None if not found
    """
    try:
        # Create a simple DNS query for A record
        # Using getaddrinfo with the specific DNS server requires more complex setup
        # For simplicity, we'll use socket.gethostbyname which uses system resolver
        # In Docker, we'll configure the container to use localhost as resolver

        # Create a socket and connect to our DNS server
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.settimeout(5)

        # Build a simple DNS query packet
        # This is a minimal A record query
        import struct
        import random

        # Transaction ID
        tid = random.randint(0, 65535)

        # Flags: standard query
        flags = 0x0100

        # Counts
        qdcount = 1  # One question
        ancount = 0
        nscount = 0
        arcount = 0

        # Build header
        header = struct.pack('!HHHHHH', tid, flags, qdcount, ancount, nscount, arcount)

        # Build question section
        question = b''
        for part in domain.split('.'):
            question += struct.pack('B', len(part)) + part.encode()
        question += b'\x00'  # End of domain name
        question += struct.pack('!HH', 1, 1)  # Type A, Class IN

        query = header + question

        # Send query
        sock.sendto(query, (server, port))

        # Receive response
        data, _ = sock.recvfrom(512)
        sock.close()

        # Parse response
        # Skip header (12 bytes)
        offset = 12

        # Skip question section
        while data[offset] != 0:
            offset += 1
        offset += 5  # Skip null byte and QTYPE/QCLASS

        # Parse answer section
        # Skip name (usually a pointer)
        if data[offset] & 0xC0 == 0xC0:
            offset += 2
        else:
            while data[offset] != 0:
                offset += 1
            offset += 1

        # Read type, class, TTL, and data length
        rtype, rclass, ttl, rdlength = struct.unpack('!HHIH', data[offset:offset+10])
        offset += 10

        if rtype == 1 and rdlength == 4:  # A record
            ip_bytes = data[offset:offset+4]
            ip_address = '.'.join(str(b) for b in ip_bytes)
            return ip_address

        return None

    except Exception as e:
        print_error(f"DNS query failed: {e}")
        return None


def update_dns(domain: str, key: str, ip: Optional[str] = None) -> bool:
    """
    Update DNS record via HTTP API.

    Args:
        domain: Domain name to update
        key: Authentication key
        ip: IP address (None for auto-detect)

    Returns:
        True if successful, False otherwise
    """
    try:
        headers = {
            "Authorization": f"Bearer {key}",
            "Content-Type": "application/json",
        }

        payload = {"domain": domain}
        if ip:
            payload["ip"] = ip

        response = requests.post(
            f"{API_BASE_URL}/update",
            json=payload,
            headers=headers,
            timeout=10
        )

        if response.status_code == 200:
            result = response.json()
            if result.get("success"):
                print_success(f"API returned: {result.get('message')}")
                return True
            else:
                print_error(f"API error: {result.get('message')}")
                return False
        else:
            print_error(f"HTTP {response.status_code}: {response.text}")
            return False

    except Exception as e:
        print_error(f"Request failed: {e}")
        return False


def wait_for_service(url: str, timeout: int = 30) -> bool:
    """
    Wait for a service to be available.

    Args:
        url: URL to check
        timeout: Maximum time to wait in seconds

    Returns:
        True if service is available, False otherwise
    """
    start_time = time.time()
    while time.time() - start_time < timeout:
        try:
            response = requests.get(url, timeout=2)
            # Any response (even 404) means the service is up
            return True
        except requests.exceptions.RequestException:
            time.sleep(1)
    return False


def run_tests() -> int:
    """
    Run all integration tests.

    Returns:
        Exit code (0 for success, 1 for failure)
    """
    print_header("Unbound DDNS Integration Test")

    # Wait for services to be ready
    print_info("Waiting for unbound_ddns service to be ready...")
    if not wait_for_service(f"{API_BASE_URL}/update"):
        print_error("Service did not start in time")
        return 1
    print_success("Service is ready")

    # Give Unbound a moment to fully initialize
    time.sleep(2)

    # Track results
    passed = 0
    failed = 0

    # Run each test
    for test in TESTS:
        print_header(f"Test: {test['name']}")

        # Step 1: Update DNS via API
        print_info(f"Updating {test['domain']} via API...")
        if test['ip']:
            print_info(f"  Setting IP to: {test['ip']}")
        else:
            print_info(f"  Using auto-detected IP")

        if not update_dns(test['domain'], test['key'], test['ip']):
            print_error(f"Failed to update DNS record")
            failed += 1
            continue

        # Step 2: Wait a moment for Unbound to reload
        print_info("Waiting for Unbound to reload...")
        time.sleep(2)

        # Step 3: Query DNS
        print_info(f"Querying DNS for {test['domain']}...")
        resolved_ip = dns_query(test['domain'])

        if resolved_ip is None:
            print_error(f"DNS query returned no result")
            failed += 1
            continue

        print_info(f"  Resolved to: {resolved_ip}")

        # Step 4: Verify result
        if resolved_ip == test['expected_ip']:
            print_success(f"Test passed! IP matches expected value: {test['expected_ip']}")
            passed += 1
        else:
            print_error(f"Test failed! Expected {test['expected_ip']}, got {resolved_ip}")
            failed += 1

    # Print summary
    print_header("Test Summary")
    print(f"Total tests: {len(TESTS)}")
    print_success(f"Passed: {passed}")
    if failed > 0:
        print_error(f"Failed: {failed}")
    else:
        print_info(f"Failed: {failed}")

    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(run_tests())
