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

        let config: Config =
            toml::from_str(&content).map_err(|e| format!("Failed to parse config file: {}", e))?;

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
    let payload = match parse_update_request(&headers, &body) {
        Ok(p) => p,
        Err(e) => {
            return UpdateResponse {
                success: false,
                message: format!("Failed to parse request: {}", e),
            };
        }
    };

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
        None => addr.ip().to_string(),
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
    let pattern = format!(r#"local-data:\s*"{}\s+IN\s+A\s+"#, regex::escape(domain));
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

    // Check if domain exists in the configuration
    if !domain_exists_in_config(&content, domain) {
        return Err(format!(
            "Domain '{}' not found in Unbound config. Cannot update non-existent domain.",
            domain
        ));
    }

    // Create the new local-data entry
    let new_entry = format!("local-data: \"{} IN A {}\"", domain, ip);

    // Pattern to match existing local-data entry for this domain
    let pattern = format!(
        r#"local-data:\s*"{}\s+IN\s+A\s+[^"]+""#,
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

    #[test]
    fn test_config_parsing() {
        use std::io::Write;

        let temp_dir = std::env::temp_dir();
        let unbound_config_path = temp_dir.join("test_config_parsing.conf");

        // Create mock Unbound config with the domains
        let mut file = fs::File::create(&unbound_config_path).unwrap();
        writeln!(file, "server:").unwrap();
        writeln!(file, "  verbosity: 1").unwrap();
        writeln!(file, "local-data: \"home.example.com IN A 192.168.1.1\"").unwrap();
        writeln!(file, "local-data: \"server.example.com IN A 192.168.1.2\"").unwrap();
        drop(file);

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
            unbound_config_path.display()
        );

        let config: Config = toml::from_str(&toml_content).unwrap();
        config.validate().unwrap();
        assert_eq!(config.unbound_config_path, unbound_config_path);
        assert_eq!(config.domains.len(), 2);
        assert_eq!(config.domains[0].name, "home.example.com");
        assert_eq!(config.domains[0].key, "secret-key-1");

        // Cleanup
        fs::remove_file(&unbound_config_path).unwrap();
    }

    #[test]
    fn test_config_validation_no_domains() {
        let config = Config {
            unbound_config_path: PathBuf::from("/etc/unbound/unbound.conf"),
            domains: vec![],
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least one domain"));
    }

    #[test]
    fn test_config_validation_empty_domain_name() {
        let config = Config {
            unbound_config_path: PathBuf::from("/etc/unbound/unbound.conf"),
            domains: vec![DomainConfig {
                name: "".to_string(),
                key: "key1".to_string(),
            }],
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty name"));
    }

    #[test]
    fn test_config_validation_empty_key() {
        let config = Config {
            unbound_config_path: PathBuf::from("/etc/unbound/unbound.conf"),
            domains: vec![DomainConfig {
                name: "test.example.com".to_string(),
                key: "".to_string(),
            }],
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty key"));
    }

    #[test]
    fn test_config_validation_duplicate_domains() {
        let config = Config {
            unbound_config_path: PathBuf::from("/etc/unbound/unbound.conf"),
            domains: vec![
                DomainConfig {
                    name: "test.example.com".to_string(),
                    key: "key1".to_string(),
                },
                DomainConfig {
                    name: "test.example.com".to_string(),
                    key: "key2".to_string(),
                },
            ],
        };
        let result = config.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Duplicate domain"));
    }

    #[test]
    fn test_find_domain() {
        let config = Config {
            unbound_config_path: PathBuf::from("/etc/unbound/unbound.conf"),
            domains: vec![
                DomainConfig {
                    name: "home.example.com".to_string(),
                    key: "key1".to_string(),
                },
                DomainConfig {
                    name: "server.example.com".to_string(),
                    key: "key2".to_string(),
                },
            ],
        };

        assert!(config.find_domain("home.example.com").is_some());
        assert!(config.find_domain("nonexistent.com").is_none());
    }

    #[test]
    fn test_config_validation_domain_not_in_unbound_config() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir();
        let unbound_config_path = temp_dir.join("test_validation_missing_domain.conf");

        // Create Unbound config without the domain
        let mut file = fs::File::create(&unbound_config_path).unwrap();
        writeln!(file, "server:").unwrap();
        writeln!(file, "  verbosity: 1").unwrap();
        drop(file);

        let config = Config {
            unbound_config_path: unbound_config_path.clone(),
            domains: vec![DomainConfig {
                name: "missing.example.com".to_string(),
                key: "key1".to_string(),
            }],
        };

        let result = config.validate();
        assert!(result.is_err());
        let error_msg = result.unwrap_err();
        assert!(error_msg.contains("missing.example.com"));
        assert!(error_msg.contains("not found in Unbound config"));

        // Cleanup
        fs::remove_file(&unbound_config_path).unwrap();
    }

    #[test]
    fn test_update_unbound_config_nonexistent_domain() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_unbound_nonexistent.conf");

        // Create initial config without the domain
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "server:").unwrap();
        writeln!(file, "  verbosity: 1").unwrap();
        drop(file);

        // Try to update non-existent domain - should fail
        let result = update_unbound_config(&config_path, "test.example.com", "192.168.1.1");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found in Unbound config"));

        // Cleanup
        fs::remove_file(&config_path).unwrap();
    }

    #[test]
    fn test_update_unbound_config_replace_entry() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_unbound_replace.conf");

        // Create initial config with existing entry
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "server:").unwrap();
        writeln!(file, "  verbosity: 1").unwrap();
        writeln!(file, "local-data: \"test.example.com IN A 192.168.1.1\"").unwrap();
        drop(file);

        // Update existing entry
        update_unbound_config(&config_path, "test.example.com", "10.0.0.1").unwrap();

        // Verify
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("local-data: \"test.example.com IN A 10.0.0.1\""));
        assert!(!content.contains("192.168.1.1"));

        // Cleanup
        fs::remove_file(&config_path).unwrap();
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

        let config = Arc::new(Config {
            unbound_config_path: PathBuf::from("/tmp/test.conf"),
            domains: vec![DomainConfig {
                name: "allowed.example.com".to_string(),
                key: "secret123".to_string(),
            }],
        });

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

        let config = Arc::new(Config {
            unbound_config_path: PathBuf::from("/tmp/test.conf"),
            domains: vec![DomainConfig {
                name: "test.example.com".to_string(),
                key: "correct-key".to_string(),
            }],
        });

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
        use std::io::Write;
        use tower::ServiceExt;

        // Create temp config file with initial domain entry
        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_integration.conf");
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "server:").unwrap();
        writeln!(file, "  verbosity: 1").unwrap();
        writeln!(file, "local-data: \"test.example.com IN A 192.168.1.1\"").unwrap();
        drop(file);

        let config = Arc::new(Config {
            unbound_config_path: config_path.clone(),
            domains: vec![DomainConfig {
                name: "test.example.com".to_string(),
                key: "test-key".to_string(),
            }],
        });

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

        // Verify config was updated
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("local-data: \"test.example.com IN A 203.0.113.42\""));

        // Cleanup
        fs::remove_file(&config_path).unwrap();
    }

    #[tokio::test]
    async fn test_update_endpoint_auto_detect_ip() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use std::io::Write;
        use tower::ServiceExt;

        // Create temp config file with initial domain entry
        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_integration_autoip.conf");
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "server:").unwrap();
        writeln!(file, "  verbosity: 1").unwrap();
        writeln!(file, "local-data: \"auto.example.com IN A 192.168.1.1\"").unwrap();
        drop(file);

        let config = Arc::new(Config {
            unbound_config_path: config_path.clone(),
            domains: vec![DomainConfig {
                name: "auto.example.com".to_string(),
                key: "auto-key".to_string(),
            }],
        });

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

        // Verify config was updated with client IP
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("local-data: \"auto.example.com IN A 198.51.100.42\""));

        // Cleanup
        fs::remove_file(&config_path).unwrap();
    }

    #[tokio::test]
    async fn test_update_endpoint_json_with_explicit_ip() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use std::io::Write;
        use tower::ServiceExt;

        // Create temp config file with initial domain entry
        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_integration_json.conf");
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "server:").unwrap();
        writeln!(file, "  verbosity: 1").unwrap();
        writeln!(file, "local-data: \"json.example.com IN A 192.168.1.1\"").unwrap();
        drop(file);

        let config = Arc::new(Config {
            unbound_config_path: config_path.clone(),
            domains: vec![DomainConfig {
                name: "json.example.com".to_string(),
                key: "json-key".to_string(),
            }],
        });

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

        // Verify config was updated
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("local-data: \"json.example.com IN A 203.0.113.100\""));

        // Cleanup
        fs::remove_file(&config_path).unwrap();
    }

    #[tokio::test]
    async fn test_update_endpoint_json_auto_detect_ip() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use std::io::Write;
        use tower::ServiceExt;

        // Create temp config file with initial domain entry
        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_integration_json_auto.conf");
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "server:").unwrap();
        writeln!(file, "  verbosity: 1").unwrap();
        writeln!(file, "local-data: \"autoip.example.com IN A 192.168.1.1\"").unwrap();
        drop(file);

        let config = Arc::new(Config {
            unbound_config_path: config_path.clone(),
            domains: vec![DomainConfig {
                name: "autoip.example.com".to_string(),
                key: "autoip-key".to_string(),
            }],
        });

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

        // Verify config was updated with client IP
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("local-data: \"autoip.example.com IN A 198.51.100.99\""));

        // Cleanup
        fs::remove_file(&config_path).unwrap();
    }

    #[test]
    fn test_config_load() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir();
        let config_file = temp_dir.join("test_config_load.toml");
        let unbound_config = temp_dir.join("test_config_load_unbound.conf");

        // Create Unbound config with domain
        let mut file = fs::File::create(&unbound_config).unwrap();
        writeln!(file, "server:").unwrap();
        writeln!(file, "local-data: \"example.com IN A 192.168.1.1\"").unwrap();
        drop(file);

        // Create config file
        let config_content = format!(
            r#"unbound_config_path = "{}"

[[domains]]
name = "example.com"
key = "test-key"
"#,
            unbound_config.display()
        );
        fs::write(&config_file, config_content).unwrap();

        // Test successful load
        let result = Config::load(config_file.to_str().unwrap());
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.domains.len(), 1);
        assert_eq!(config.domains[0].name, "example.com");

        // Cleanup
        fs::remove_file(&config_file).unwrap();
        fs::remove_file(&unbound_config).unwrap();
    }

    #[test]
    fn test_config_load_file_not_found() {
        let result = Config::load("/nonexistent/path/config.toml");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to read config file"));
    }

    #[test]
    fn test_config_load_invalid_toml() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir();
        let config_file = temp_dir.join("test_invalid_toml.toml");

        let mut file = fs::File::create(&config_file).unwrap();
        writeln!(file, "invalid toml {{{{").unwrap();
        drop(file);

        let result = Config::load(config_file.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse config file"));

        // Cleanup
        fs::remove_file(&config_file).unwrap();
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

        let config = Arc::new(Config {
            unbound_config_path: PathBuf::from("/tmp/test.conf"),
            domains: vec![DomainConfig {
                name: "test.example.com".to_string(),
                key: "test-key".to_string(),
            }],
        });

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

        let config = Arc::new(Config {
            unbound_config_path: PathBuf::from("/tmp/test.conf"),
            domains: vec![DomainConfig {
                name: "test.example.com".to_string(),
                key: "test-key".to_string(),
            }],
        });

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
        let config = Arc::new(Config {
            unbound_config_path: PathBuf::from("/tmp/test.conf"),
            domains: vec![DomainConfig {
                name: "test.example.com".to_string(),
                key: "test-key".to_string(),
            }],
        });

        let app = create_app(config);
        // Just verify the router is created successfully
        // The actual route testing is done in other tests
        assert!(format!("{:?}", app).contains("Router"));
    }

    #[test]
    fn test_print_config_info() {
        let config = Config {
            unbound_config_path: PathBuf::from("/etc/unbound/unbound.conf"),
            domains: vec![
                DomainConfig {
                    name: "example1.com".to_string(),
                    key: "key1".to_string(),
                },
                DomainConfig {
                    name: "example2.com".to_string(),
                    key: "key2".to_string(),
                },
            ],
        };

        // Just ensure it doesn't panic - we can't easily test stdout
        print_config_info(&config);
    }
}
