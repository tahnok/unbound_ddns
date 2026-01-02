use axum::{
    Router,
    body::Bytes,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// Normalizes a domain name by removing the trailing dot if present.
///
/// In DNS, `example.com.` and `example.com` should be treated as the same domain.
/// This function normalizes domain names for internal comparisons while we maintain
/// proper FQDN format (with trailing dot) when writing to Unbound config.
///
/// # Arguments
/// * `domain` - The domain name to normalize
///
/// # Returns
/// The domain name without a trailing dot
fn normalize_domain(domain: &str) -> String {
    domain.trim_end_matches('.').to_string()
}

#[derive(Debug, Deserialize, Clone)]
struct Config {
    unbound_config_path: PathBuf,
    domains: Vec<DomainConfig>,
}

#[derive(Debug, Deserialize, Clone)]
struct DomainConfig {
    name: String,
    key: String,
}

impl Config {
    fn load(path: &str) -> Result<Self, String> {
        let content =
            fs::read_to_string(path).map_err(|e| format!("Failed to read config file: {}", e))?;

        let mut config: Config =
            toml::from_str(&content).map_err(|e| format!("Failed to parse config file: {}", e))?;

        // Normalize all domain names by removing trailing dots
        for domain in &mut config.domains {
            domain.name = normalize_domain(&domain.name);
        }

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        // Check if there are any domains configured
        if self.domains.is_empty() {
            return Err("Configuration must contain at least one domain".to_string());
        }

        // Check each domain for validity
        for (idx, domain) in self.domains.iter().enumerate() {
            if domain.name.trim().is_empty() {
                return Err(format!("Domain at index {} has an empty name", idx));
            }
            if domain.key.trim().is_empty() {
                return Err(format!("Domain '{}' has an empty key", domain.name));
            }
        }

        // Check for duplicate domain names
        for i in 0..self.domains.len() {
            for j in (i + 1)..self.domains.len() {
                if self.domains[i].name == self.domains[j].name {
                    return Err(format!(
                        "Duplicate domain '{}' found in configuration",
                        self.domains[i].name
                    ));
                }
            }
        }

        // Check if Unbound config file exists and contains all configured domains
        let unbound_content = fs::read_to_string(&self.unbound_config_path).map_err(|e| {
            format!(
                "Failed to read Unbound config file at {:?}: {}",
                self.unbound_config_path, e
            )
        })?;

        for domain in &self.domains {
            if !domain_exists_in_config(&unbound_content, &domain.name) {
                return Err(format!(
                    "Domain '{}' not found in Unbound config file. Please add 'local-data: \"{} IN A <ip>\"' to {:?} first.",
                    domain.name, domain.name, self.unbound_config_path
                ));
            }
        }

        Ok(())
    }

    fn find_domain(&self, name: &str) -> Option<&DomainConfig> {
        self.domains.iter().find(|d| d.name == name)
    }
}

#[derive(Debug, Deserialize)]
struct UpdateRequest {
    domain: String,
    ip: Option<String>,
}

#[derive(Debug, Serialize)]
struct UpdateResponse {
    success: bool,
    message: String,
}

impl IntoResponse for UpdateResponse {
    fn into_response(self) -> axum::response::Response {
        let status = if self.success {
            StatusCode::OK
        } else {
            StatusCode::BAD_REQUEST
        };

        (status, axum::Json(self)).into_response()
    }
}

fn extract_auth_key(headers: &HeaderMap) -> Result<String, String> {
    let auth_header = headers
        .get("authorization")
        .ok_or_else(|| "Missing Authorization header".to_string())?;

    let auth_str = auth_header
        .to_str()
        .map_err(|_| "Invalid Authorization header encoding".to_string())?;

    // Support both "Bearer <key>" and just "<key>" formats
    let key = if let Some(bearer_key) = auth_str.strip_prefix("Bearer ") {
        bearer_key.to_string()
    } else {
        auth_str.to_string()
    };

    if key.trim().is_empty() {
        return Err("Authorization header cannot be empty".to_string());
    }

    Ok(key)
}

/// Extracts the real client IP address from the request headers when running behind a proxy.
///
/// This function checks for common proxy headers in the following order:
/// 1. X-Forwarded-For: Takes the leftmost (original client) IP from the comma-separated list
/// 2. X-Real-IP: The direct client IP set by the proxy
/// 3. Falls back to the direct connection IP if no proxy headers are present
///
/// # Arguments
/// * `headers` - The HTTP request headers
/// * `addr` - The socket address of the direct connection
///
/// # Returns
/// The client IP address as a string
fn extract_client_ip(headers: &HeaderMap, addr: &SocketAddr) -> String {
    // Check X-Forwarded-For header first (most common)
    // Format: "client, proxy1, proxy2" - we want the leftmost (client) IP
    if let Some(forwarded_for) = headers.get("x-forwarded-for")
        && let Ok(forwarded_str) = forwarded_for.to_str()
    {
        // Take the first IP in the comma-separated list
        if let Some(client_ip) = forwarded_str.split(',').next() {
            let ip = client_ip.trim();
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
    }

    // Check X-Real-IP header (used by nginx and others)
    if let Some(real_ip) = headers.get("x-real-ip")
        && let Ok(ip_str) = real_ip.to_str()
    {
        let ip = ip_str.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }

    // Fall back to the direct connection IP
    addr.ip().to_string()
}

async fn update_handler(
    State(config): State<Arc<Config>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> UpdateResponse {
    // Extract and validate Authorization header
    let auth_key = match extract_auth_key(&headers) {
        Ok(key) => key,
        Err(e) => {
            return UpdateResponse {
                success: false,
                message: e,
            };
        }
    };

    // Parse the request based on Content-Type
    let mut payload = match parse_update_request(&headers, &body) {
        Ok(p) => p,
        Err(e) => {
            return UpdateResponse {
                success: false,
                message: format!("Failed to parse request: {}", e),
            };
        }
    };

    // Normalize the domain name by removing trailing dot
    payload.domain = normalize_domain(&payload.domain);

    // Authenticate the request - use same error message for both invalid domain and invalid key
    // to prevent leaking information about which domains are valid
    const UNAUTHORIZED_ERROR: &str = "Unauthorized";

    let domain_config = match config.find_domain(&payload.domain) {
        Some(d) => d,
        None => {
            return UpdateResponse {
                success: false,
                message: UNAUTHORIZED_ERROR.to_string(),
            };
        }
    };

    // Use constant-time comparison to prevent timing attacks
    // that could be used to guess the key byte-by-byte
    if !bool::from(domain_config.key.as_bytes().ct_eq(auth_key.as_bytes())) {
        return UpdateResponse {
            success: false,
            message: UNAUTHORIZED_ERROR.to_string(),
        };
    }

    // Determine the IP address
    let ip = match payload.ip {
        Some(ip) => ip,
        None => extract_client_ip(&headers, &addr),
    };

    // Update the Unbound configuration
    match update_unbound_config(&config.unbound_config_path, &payload.domain, &ip) {
        Ok(_) => {
            // Reload Unbound
            match reload_unbound() {
                Ok(_) => UpdateResponse {
                    success: true,
                    message: format!("Updated {} to {}", payload.domain, ip),
                },
                Err(e) => UpdateResponse {
                    success: false,
                    message: format!("Failed to reload Unbound: {}", e),
                },
            }
        }
        Err(e) => UpdateResponse {
            success: false,
            message: format!("Failed to update configuration: {}", e),
        },
    }
}

fn parse_update_request(headers: &HeaderMap, body: &Bytes) -> Result<UpdateRequest, String> {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if content_type.contains("application/json") {
        // Parse as JSON
        serde_json::from_slice(body).map_err(|e| format!("Invalid JSON: {}", e))
    } else {
        // Parse as form data (default)
        let body_str = std::str::from_utf8(body).map_err(|e| format!("Invalid UTF-8: {}", e))?;

        serde_urlencoded::from_str(body_str).map_err(|e| format!("Invalid form data: {}", e))
    }
}

fn domain_exists_in_config(content: &str, domain: &str) -> bool {
    // Match domain with or without trailing dot (\.? makes the dot optional)
    let pattern = format!(r#"local-data:\s*"{}\.?\s+IN\s+A\s+"#, regex::escape(domain));
    if let Ok(re) = Regex::new(&pattern) {
        re.is_match(content)
    } else {
        false
    }
}

fn update_unbound_config(config_path: &PathBuf, domain: &str, ip: &str) -> Result<(), String> {
    // Read the current configuration
    let content = fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read Unbound config: {}", e))?;

    // Check if domain exists in the configuration (domain is already normalized without trailing dot)
    if !domain_exists_in_config(&content, domain) {
        return Err(format!(
            "Domain '{}' not found in Unbound config. Cannot update non-existent domain.",
            domain
        ));
    }

    // Create the new local-data entry with proper FQDN format (trailing dot)
    let new_entry = format!("local-data: \"{}. IN A {}\"", domain, ip);

    // Pattern to match existing local-data entry for this domain (with or without trailing dot)
    let pattern = format!(
        r#"local-data:\s*"{}\.?\s+IN\s+A\s+[^"]+""#,
        regex::escape(domain)
    );
    let re = Regex::new(&pattern).map_err(|e| format!("Failed to compile regex: {}", e))?;

    // Replace existing entry (we already checked it exists)
    let updated_content = re.replace(&content, new_entry.as_str()).to_string();

    // Write the updated configuration
    fs::write(config_path, updated_content)
        .map_err(|e| format!("Failed to write Unbound config: {}", e))?;

    Ok(())
}

fn reload_unbound() -> Result<(), String> {
    let output = Command::new("unbound-control")
        .arg("reload")
        .output()
        .map_err(|e| format!("Failed to execute unbound-control: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("unbound-control failed: {}", stderr))
    }
}

fn create_app(config: Arc<Config>) -> Router {
    Router::new()
        .route("/update", post(update_handler))
        .with_state(config)
}

fn print_config_info(config: &Config) {
    println!("Loaded configuration:");
    println!("  Unbound config path: {:?}", config.unbound_config_path);
    println!("  Authorized domains: {}", config.domains.len());
    for domain in &config.domains {
        println!("    - {}", domain.name);
    }
}

#[tokio::main]
async fn main() {
    // Load configuration
    let config = match Config::load("config.toml") {
        Ok(config) => Arc::new(config),
        Err(e) => {
            eprintln!("Error loading configuration: {}", e);
            std::process::exit(1);
        }
    };

    print_config_info(&config);

    // Build the router
    let app = create_app(config);

    // Start the server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    println!("\nServer running on http://0.0.0.0:3000");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // ============================================================================
    // TEST HELPERS
    // ============================================================================

    /// Creates a temporary unbound config file with optional domain entries.
    ///
    /// # Arguments
    /// * `domains` - Optional slice of tuples (domain_name, ip_address) to add as local-data entries
    ///
    /// # Returns
    /// A NamedTempFile that will be automatically deleted when dropped
    fn create_unbound_config(domains: Option<&[(&str, &str)]>) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "server:").unwrap();
        writeln!(file, "  verbosity: 1").unwrap();

        if let Some(domain_entries) = domains {
            for (domain, ip) in domain_entries {
                writeln!(file, "local-data: \"{} IN A {}\"", domain, ip).unwrap();
            }
        }

        file
    }

    /// Creates a test Config struct with optional parameters.
    ///
    /// # Arguments
    /// * `unbound_config_path` - Optional path to unbound config file
    /// * `domains` - Optional slice of tuples (domain_name, api_key) for domain configs
    ///
    /// # Returns
    /// A Config struct ready for testing
    fn create_test_config(
        unbound_config_path: Option<PathBuf>,
        domains: Option<&[(&str, &str)]>,
    ) -> Config {
        Config {
            unbound_config_path: unbound_config_path
                .unwrap_or_else(|| PathBuf::from("/tmp/test.conf")),
            domains: domains
                .map(|d| {
                    d.iter()
                        .map(|(name, key)| DomainConfig {
                            name: name.to_string(),
                            key: key.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_else(Vec::new),
        }
    }

    // ============================================================================
    // TESTS
    // ============================================================================

    #[test]
    fn test_config_parsing() {
        let unbound_file = create_unbound_config(Some(&[
            ("home.example.com", "192.168.1.1"),
            ("server.example.com", "192.168.1.2"),
        ]));

        let toml_content = format!(
            r#"
unbound_config_path = "{}"

[[domains]]
name = "home.example.com"
key = "secret-key-1"

[[domains]]
name = "server.example.com"
key = "secret-key-2"
"#,
            unbound_file.path().display()
        );

        let config: Config = toml::from_str(&toml_content).unwrap();
        config.validate().unwrap();
        assert_eq!(config.unbound_config_path, unbound_file.path());
        assert_eq!(config.domains.len(), 2);
        assert_eq!(config.domains[0].name, "home.example.com");
        assert_eq!(config.domains[0].key, "secret-key-1");
    }

    #[test]
    fn test_config_validation_no_domains() {
        let config = create_test_config(None, None);
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least one domain"));
    }

    #[test]
    fn test_config_validation_empty_domain_name() {
        let config = create_test_config(None, Some(&[("", "key1")]));
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty name"));
    }

    #[test]
    fn test_config_validation_empty_key() {
        let config = create_test_config(None, Some(&[("test.example.com", "")]));
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty key"));
    }

    #[test]
    fn test_config_validation_duplicate_domains() {
        let config = create_test_config(
            None,
            Some(&[("test.example.com", "key1"), ("test.example.com", "key2")]),
        );
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Duplicate domain"));
    }

    #[test]
    fn test_find_domain() {
        let config = create_test_config(
            None,
            Some(&[("home.example.com", "key1"), ("server.example.com", "key2")]),
        );

        assert!(config.find_domain("home.example.com").is_some());
        assert!(config.find_domain("nonexistent.com").is_none());
    }

    #[test]
    fn test_config_validation_domain_not_in_unbound_config() {
        let unbound_file = create_unbound_config(None);

        let config = create_test_config(
            Some(unbound_file.path().to_path_buf()),
            Some(&[("missing.example.com", "key1")]),
        );

        let result = config.validate();
        assert!(result.is_err());
        let error_msg = result.unwrap_err();
        assert!(error_msg.contains("missing.example.com"));
        assert!(error_msg.contains("not found in Unbound config"));
    }

    #[test]
    fn test_update_unbound_config_nonexistent_domain() {
        let unbound_file = create_unbound_config(None);

        // Try to update non-existent domain - should fail
        let result = update_unbound_config(
            &unbound_file.path().to_path_buf(),
            "test.example.com",
            "192.168.1.1",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found in Unbound config"));
    }

    #[test]
    fn test_update_unbound_config_replace_entry() {
        let unbound_file = create_unbound_config(Some(&[("test.example.com", "192.168.1.1")]));

        // Update existing entry
        update_unbound_config(
            &unbound_file.path().to_path_buf(),
            "test.example.com",
            "10.0.0.1",
        )
        .unwrap();

        // Verify - now writes with trailing dot (proper FQDN)
        let content = fs::read_to_string(unbound_file.path()).unwrap();
        assert!(content.contains("local-data: \"test.example.com. IN A 10.0.0.1\""));
        assert!(!content.contains("192.168.1.1"));
    }

    // ============================================================================
    // INTEGRATION TESTS - DO NOT REMOVE
    // These tests verify the actual HTTP endpoint behavior with form data
    // ============================================================================

    #[tokio::test]
    async fn test_update_endpoint_unauthorized_domain() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let config = Arc::new(create_test_config(
            None,
            Some(&[("allowed.example.com", "secret123")]),
        ));

        let app = Router::new()
            .route("/update", post(update_handler))
            .with_state(config);

        let request = Request::builder()
            .method("POST")
            .uri("/update")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("authorization", "Bearer secret123")
            .extension(ConnectInfo(
                "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from("domain=notallowed.example.com"))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Unauthorized"));
    }

    #[tokio::test]
    async fn test_update_endpoint_invalid_key() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let config = Arc::new(create_test_config(
            None,
            Some(&[("test.example.com", "correct-key")]),
        ));

        let app = Router::new()
            .route("/update", post(update_handler))
            .with_state(config);

        let request = Request::builder()
            .method("POST")
            .uri("/update")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("authorization", "Bearer wrong-key")
            .extension(ConnectInfo(
                "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from("domain=test.example.com&ip=10.0.0.1"))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Unauthorized"));
    }

    #[tokio::test]
    async fn test_update_endpoint_with_explicit_ip() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let unbound_file = create_unbound_config(Some(&[("test.example.com", "192.168.1.1")]));

        let config = Arc::new(create_test_config(
            Some(unbound_file.path().to_path_buf()),
            Some(&[("test.example.com", "test-key")]),
        ));

        let app = Router::new()
            .route("/update", post(update_handler))
            .with_state(config);

        let request = Request::builder()
            .method("POST")
            .uri("/update")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("authorization", "Bearer test-key")
            .extension(ConnectInfo(
                "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from("domain=test.example.com&ip=203.0.113.42"))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // Note: This will fail to reload unbound, but that's expected in test
        // We're mainly testing the request parsing and validation
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();

        // Either succeeds or fails on unbound-control (expected in test environment)
        assert!(
            status == StatusCode::OK || body_str.contains("Failed to reload Unbound"),
            "Unexpected response: {} - {}",
            status,
            body_str
        );

        // Verify config was updated with proper FQDN (trailing dot)
        let content = fs::read_to_string(unbound_file.path()).unwrap();
        assert!(content.contains("local-data: \"test.example.com. IN A 203.0.113.42\""));
    }

    #[tokio::test]
    async fn test_update_endpoint_auto_detect_ip() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let unbound_file = create_unbound_config(Some(&[("auto.example.com", "192.168.1.1")]));

        let config = Arc::new(create_test_config(
            Some(unbound_file.path().to_path_buf()),
            Some(&[("auto.example.com", "auto-key")]),
        ));

        let app = Router::new()
            .route("/update", post(update_handler))
            .with_state(config);

        let request = Request::builder()
            .method("POST")
            .uri("/update")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("authorization", "Bearer auto-key")
            .extension(ConnectInfo(
                "198.51.100.42:54321".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from("domain=auto.example.com"))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();

        // Either succeeds or fails on unbound-control (expected in test environment)
        assert!(
            status == StatusCode::OK || body_str.contains("Failed to reload Unbound"),
            "Unexpected response: {} - {}",
            status,
            body_str
        );

        // Verify config was updated with client IP and proper FQDN (trailing dot)
        let content = fs::read_to_string(unbound_file.path()).unwrap();
        assert!(content.contains("local-data: \"auto.example.com. IN A 198.51.100.42\""));
    }

    #[tokio::test]
    async fn test_update_endpoint_json_with_explicit_ip() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let unbound_file = create_unbound_config(Some(&[("json.example.com", "192.168.1.1")]));

        let config = Arc::new(create_test_config(
            Some(unbound_file.path().to_path_buf()),
            Some(&[("json.example.com", "json-key")]),
        ));

        let app = Router::new()
            .route("/update", post(update_handler))
            .with_state(config);

        let json_body = r#"{"domain":"json.example.com","ip":"203.0.113.100"}"#;

        let request = Request::builder()
            .method("POST")
            .uri("/update")
            .header("content-type", "application/json")
            .header("authorization", "Bearer json-key")
            .extension(ConnectInfo(
                "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from(json_body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();

        // Either succeeds or fails on unbound-control (expected in test environment)
        assert!(
            status == StatusCode::OK || body_str.contains("Failed to reload Unbound"),
            "Unexpected response: {} - {}",
            status,
            body_str
        );

        // Verify config was updated with proper FQDN (trailing dot)
        let content = fs::read_to_string(unbound_file.path()).unwrap();
        assert!(content.contains("local-data: \"json.example.com. IN A 203.0.113.100\""));
    }

    #[tokio::test]
    async fn test_update_endpoint_json_auto_detect_ip() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let unbound_file = create_unbound_config(Some(&[("autoip.example.com", "192.168.1.1")]));

        let config = Arc::new(create_test_config(
            Some(unbound_file.path().to_path_buf()),
            Some(&[("autoip.example.com", "autoip-key")]),
        ));

        let app = Router::new()
            .route("/update", post(update_handler))
            .with_state(config);

        let json_body = r#"{"domain":"autoip.example.com"}"#;

        let request = Request::builder()
            .method("POST")
            .uri("/update")
            .header("content-type", "application/json")
            .header("authorization", "Bearer autoip-key")
            .extension(ConnectInfo(
                "198.51.100.99:54321".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from(json_body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();

        // Either succeeds or fails on unbound-control (expected in test environment)
        assert!(
            status == StatusCode::OK || body_str.contains("Failed to reload Unbound"),
            "Unexpected response: {} - {}",
            status,
            body_str
        );

        // Verify config was updated with client IP and proper FQDN (trailing dot)
        let content = fs::read_to_string(unbound_file.path()).unwrap();
        assert!(content.contains("local-data: \"autoip.example.com. IN A 198.51.100.99\""));
    }

    #[test]
    fn test_config_load() {
        let unbound_file = create_unbound_config(Some(&[("example.com", "192.168.1.1")]));
        let config_file = NamedTempFile::new().unwrap();

        // Create config file
        let config_content = format!(
            r#"unbound_config_path = "{}"

[[domains]]
name = "example.com"
key = "test-key"
"#,
            unbound_file.path().display()
        );
        fs::write(config_file.path(), config_content).unwrap();

        // Test successful load
        let result = Config::load(config_file.path().to_str().unwrap());
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.domains.len(), 1);
        assert_eq!(config.domains[0].name, "example.com");
    }

    #[test]
    fn test_config_load_file_not_found() {
        let result = Config::load("/nonexistent/path/config.toml");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to read config file"));
    }

    #[test]
    fn test_config_load_invalid_toml() {
        let mut config_file = NamedTempFile::new().unwrap();
        writeln!(config_file, "invalid toml {{{{").unwrap();

        let result = Config::load(config_file.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse config file"));
    }

    #[test]
    fn test_extract_auth_key_with_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer my-secret-key".parse().unwrap());

        let result = extract_auth_key(&headers);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "my-secret-key");
    }

    #[test]
    fn test_extract_auth_key_without_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "my-secret-key".parse().unwrap());

        let result = extract_auth_key(&headers);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "my-secret-key");
    }

    #[test]
    fn test_extract_auth_key_missing() {
        let headers = HeaderMap::new();
        let result = extract_auth_key(&headers);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing Authorization header"));
    }

    #[test]
    fn test_extract_auth_key_empty() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer ".parse().unwrap());

        let result = extract_auth_key(&headers);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot be empty"));
    }

    #[test]
    fn test_update_response_success() {
        let response = UpdateResponse {
            success: true,
            message: "Updated successfully".to_string(),
        };
        let axum_response = response.into_response();
        assert_eq!(axum_response.status(), StatusCode::OK);
    }

    #[test]
    fn test_update_response_failure() {
        let response = UpdateResponse {
            success: false,
            message: "Update failed".to_string(),
        };
        let axum_response = response.into_response();
        assert_eq!(axum_response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_parse_update_request_invalid_json() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        let body = Bytes::from("invalid json {{{");

        let result = parse_update_request(&headers, &body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid JSON"));
    }

    #[test]
    fn test_parse_update_request_invalid_utf8() {
        let headers = HeaderMap::new();
        let body = Bytes::from(vec![0xFF, 0xFE, 0xFD]);

        let result = parse_update_request(&headers, &body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid UTF-8"));
    }

    #[tokio::test]
    async fn test_update_endpoint_missing_auth_header() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let config = Arc::new(create_test_config(
            None,
            Some(&[("test.example.com", "test-key")]),
        ));

        let app = Router::new()
            .route("/update", post(update_handler))
            .with_state(config);

        let request = Request::builder()
            .method("POST")
            .uri("/update")
            .header("content-type", "application/x-www-form-urlencoded")
            .extension(ConnectInfo(
                "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from("domain=test.example.com"))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Missing Authorization header"));
    }

    #[tokio::test]
    async fn test_update_endpoint_invalid_json() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let config = Arc::new(create_test_config(
            None,
            Some(&[("test.example.com", "test-key")]),
        ));

        let app = Router::new()
            .route("/update", post(update_handler))
            .with_state(config);

        let request = Request::builder()
            .method("POST")
            .uri("/update")
            .header("content-type", "application/json")
            .header("authorization", "Bearer test-key")
            .extension(ConnectInfo(
                "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from("invalid json {{{"))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("Failed to parse request"));
    }

    #[test]
    fn test_create_app() {
        let config = Arc::new(create_test_config(
            None,
            Some(&[("test.example.com", "test-key")]),
        ));

        let app = create_app(config);
        // Just verify the router is created successfully
        // The actual route testing is done in other tests
        assert!(format!("{:?}", app).contains("Router"));
    }

    #[test]
    fn test_print_config_info() {
        let config = create_test_config(
            None,
            Some(&[("example1.com", "key1"), ("example2.com", "key2")]),
        );

        // Just ensure it doesn't panic - we can't easily test stdout
        print_config_info(&config);
    }

    #[test]
    fn test_extract_client_ip_from_x_forwarded_for_single() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.1".parse().unwrap());
        let addr: SocketAddr = "192.168.1.1:12345".parse().unwrap();

        let ip = extract_client_ip(&headers, &addr);
        assert_eq!(ip, "203.0.113.1");
    }

    #[test]
    fn test_extract_client_ip_from_x_forwarded_for_multiple() {
        let mut headers = HeaderMap::new();
        // Client IP is the leftmost one
        headers.insert(
            "x-forwarded-for",
            "203.0.113.1, 198.51.100.1, 192.0.2.1".parse().unwrap(),
        );
        let addr: SocketAddr = "192.168.1.1:12345".parse().unwrap();

        let ip = extract_client_ip(&headers, &addr);
        assert_eq!(ip, "203.0.113.1");
    }

    #[test]
    fn test_extract_client_ip_from_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.42".parse().unwrap());
        let addr: SocketAddr = "192.168.1.1:12345".parse().unwrap();

        let ip = extract_client_ip(&headers, &addr);
        assert_eq!(ip, "203.0.113.42");
    }

    #[test]
    fn test_extract_client_ip_x_forwarded_for_takes_precedence() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.1".parse().unwrap());
        headers.insert("x-real-ip", "203.0.113.2".parse().unwrap());
        let addr: SocketAddr = "192.168.1.1:12345".parse().unwrap();

        let ip = extract_client_ip(&headers, &addr);
        // X-Forwarded-For should take precedence
        assert_eq!(ip, "203.0.113.1");
    }

    #[test]
    fn test_extract_client_ip_fallback_to_connection() {
        let headers = HeaderMap::new();
        let addr: SocketAddr = "198.51.100.99:54321".parse().unwrap();

        let ip = extract_client_ip(&headers, &addr);
        assert_eq!(ip, "198.51.100.99");
    }

    #[test]
    fn test_extract_client_ip_empty_x_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "".parse().unwrap());
        let addr: SocketAddr = "198.51.100.99:54321".parse().unwrap();

        let ip = extract_client_ip(&headers, &addr);
        // Should fall back to connection IP
        assert_eq!(ip, "198.51.100.99");
    }

    #[test]
    fn test_extract_client_ip_with_spaces() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "  203.0.113.1  ".parse().unwrap());
        let addr: SocketAddr = "192.168.1.1:12345".parse().unwrap();

        let ip = extract_client_ip(&headers, &addr);
        // Should trim whitespace
        assert_eq!(ip, "203.0.113.1");
    }

    // ============================================================================
    // TRAILING DOT NORMALIZATION TESTS
    // ============================================================================

    #[test]
    fn test_normalize_domain_with_trailing_dot() {
        assert_eq!(normalize_domain("example.com."), "example.com");
        assert_eq!(normalize_domain("foo.example.com."), "foo.example.com");
    }

    #[test]
    fn test_normalize_domain_without_trailing_dot() {
        assert_eq!(normalize_domain("example.com"), "example.com");
        assert_eq!(normalize_domain("foo.example.com"), "foo.example.com");
    }

    #[test]
    fn test_normalize_domain_multiple_trailing_dots() {
        assert_eq!(normalize_domain("example.com.."), "example.com");
        assert_eq!(normalize_domain("example.com..."), "example.com");
    }

    #[test]
    fn test_domain_exists_in_config_with_trailing_dot() {
        let config = "local-data: \"example.com. IN A 192.168.1.1\"";
        assert!(domain_exists_in_config(config, "example.com"));
    }

    #[test]
    fn test_domain_exists_in_config_without_trailing_dot() {
        let config = "local-data: \"example.com IN A 192.168.1.1\"";
        assert!(domain_exists_in_config(config, "example.com"));
    }

    #[test]
    fn test_config_load_normalizes_trailing_dots() {
        let unbound_file = create_unbound_config(Some(&[("test.example.com", "192.168.1.1")]));
        let config_file = NamedTempFile::new().unwrap();

        // Create config with trailing dots
        let config_content = format!(
            r#"unbound_config_path = "{}"

[[domains]]
name = "test.example.com."
key = "test-key"
"#,
            unbound_file.path().display()
        );
        fs::write(config_file.path(), config_content).unwrap();

        // Load config and verify trailing dot is removed
        let result = Config::load(config_file.path().to_str().unwrap());
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.domains[0].name, "test.example.com");
    }

    #[test]
    fn test_update_unbound_config_writes_trailing_dot() {
        let unbound_file = create_unbound_config(Some(&[("test.example.com", "192.168.1.1")]));

        // Update with normalized domain (no trailing dot)
        update_unbound_config(
            &unbound_file.path().to_path_buf(),
            "test.example.com",
            "10.0.0.1",
        )
        .unwrap();

        // Verify the written config has trailing dot (proper FQDN)
        let content = fs::read_to_string(unbound_file.path()).unwrap();
        assert!(content.contains("local-data: \"test.example.com. IN A 10.0.0.1\""));
    }

    #[test]
    fn test_update_unbound_config_handles_existing_trailing_dot() {
        // Create config with trailing dot
        let unbound_file = create_unbound_config(Some(&[("test.example.com.", "192.168.1.1")]));

        // Update should work even though existing config has trailing dot
        update_unbound_config(
            &unbound_file.path().to_path_buf(),
            "test.example.com",
            "10.0.0.1",
        )
        .unwrap();

        // Verify the updated config has trailing dot
        let content = fs::read_to_string(unbound_file.path()).unwrap();
        assert!(content.contains("local-data: \"test.example.com. IN A 10.0.0.1\""));
    }

    #[tokio::test]
    async fn test_update_endpoint_with_trailing_dot_in_domain() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let unbound_file = create_unbound_config(Some(&[("trailing.example.com", "192.168.1.1")]));

        let config = Arc::new(create_test_config(
            Some(unbound_file.path().to_path_buf()),
            Some(&[("trailing.example.com", "trailing-key")]),
        ));

        let app = Router::new()
            .route("/update", post(update_handler))
            .with_state(config);

        // Send request with trailing dot in domain name
        let request = Request::builder()
            .method("POST")
            .uri("/update")
            .header("content-type", "application/json")
            .header("authorization", "Bearer trailing-key")
            .extension(ConnectInfo(
                "127.0.0.1:12345".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from(
                r#"{"domain":"trailing.example.com.","ip":"203.0.113.99"}"#,
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();

        // Either succeeds or fails on unbound-control (expected in test environment)
        assert!(
            status == StatusCode::OK || body_str.contains("Failed to reload Unbound"),
            "Unexpected response: {} - {}",
            status,
            body_str
        );

        // Verify config was updated with proper FQDN (trailing dot)
        let content = fs::read_to_string(unbound_file.path()).unwrap();
        assert!(content.contains("local-data: \"trailing.example.com. IN A 203.0.113.99\""));
    }

    #[tokio::test]
    async fn test_update_endpoint_with_x_forwarded_for() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let unbound_file = create_unbound_config(Some(&[("proxy.example.com", "192.168.1.1")]));

        let config = Arc::new(create_test_config(
            Some(unbound_file.path().to_path_buf()),
            Some(&[("proxy.example.com", "proxy-key")]),
        ));

        let app = Router::new()
            .route("/update", post(update_handler))
            .with_state(config);

        let request = Request::builder()
            .method("POST")
            .uri("/update")
            .header("content-type", "application/json")
            .header("authorization", "Bearer proxy-key")
            .header("x-forwarded-for", "203.0.113.50")
            .extension(ConnectInfo(
                "192.168.1.100:12345".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from(r#"{"domain":"proxy.example.com"}"#))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();

        // Either succeeds or fails on unbound-control (expected in test environment)
        assert!(
            status == StatusCode::OK || body_str.contains("Failed to reload Unbound"),
            "Unexpected response: {} - {}",
            status,
            body_str
        );

        // Verify config was updated with the IP from X-Forwarded-For, not the connection IP
        let content = fs::read_to_string(unbound_file.path()).unwrap();
        assert!(content.contains("local-data: \"proxy.example.com. IN A 203.0.113.50\""));
        assert!(!content.contains("192.168.1.100")); // Should NOT use the proxy IP
    }

    #[tokio::test]
    async fn test_update_endpoint_with_x_real_ip() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let unbound_file = create_unbound_config(Some(&[("realip.example.com", "192.168.1.1")]));

        let config = Arc::new(create_test_config(
            Some(unbound_file.path().to_path_buf()),
            Some(&[("realip.example.com", "realip-key")]),
        ));

        let app = Router::new()
            .route("/update", post(update_handler))
            .with_state(config);

        let request = Request::builder()
            .method("POST")
            .uri("/update")
            .header("content-type", "application/json")
            .header("authorization", "Bearer realip-key")
            .header("x-real-ip", "203.0.113.75")
            .extension(ConnectInfo(
                "192.168.1.100:12345".parse::<SocketAddr>().unwrap(),
            ))
            .body(Body::from(r#"{"domain":"realip.example.com"}"#))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();

        // Either succeeds or fails on unbound-control (expected in test environment)
        assert!(
            status == StatusCode::OK || body_str.contains("Failed to reload Unbound"),
            "Unexpected response: {} - {}",
            status,
            body_str
        );

        // Verify config was updated with the IP from X-Real-IP and proper FQDN (trailing dot)
        let content = fs::read_to_string(unbound_file.path()).unwrap();
        assert!(content.contains("local-data: \"realip.example.com. IN A 203.0.113.75\""));
    }
}
