use axum::{
    extract::{ConnectInfo, Form, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Router,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

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
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;

        toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config file: {}", e))
    }

    fn find_domain(&self, name: &str) -> Option<&DomainConfig> {
        self.domains.iter().find(|d| d.name == name)
    }
}

#[derive(Debug, Deserialize)]
struct UpdateRequest {
    key: String,
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

async fn update_handler(
    State(config): State<Arc<Config>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Form(payload): Form<UpdateRequest>,
) -> UpdateResponse {
    // Authenticate the request
    let domain_config = match config.find_domain(&payload.domain) {
        Some(d) => d,
        None => {
            return UpdateResponse {
                success: false,
                message: format!("Domain '{}' is not authorized", payload.domain),
            };
        }
    };

    if domain_config.key != payload.key {
        return UpdateResponse {
            success: false,
            message: "Invalid authentication key".to_string(),
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

fn update_unbound_config(
    config_path: &PathBuf,
    domain: &str,
    ip: &str,
) -> Result<(), String> {
    // Read the current configuration
    let content = fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read Unbound config: {}", e))?;

    // Create the new local-data entry
    let new_entry = format!("local-data: \"{} IN A {}\"", domain, ip);

    // Pattern to match existing local-data entry for this domain
    let pattern = format!(r#"local-data:\s*"{}\s+IN\s+A\s+[^"]+""#, regex::escape(domain));
    let re = Regex::new(&pattern)
        .map_err(|e| format!("Failed to compile regex: {}", e))?;

    let updated_content = if re.is_match(&content) {
        // Replace existing entry
        re.replace(&content, new_entry.as_str()).to_string()
    } else {
        // Append new entry
        format!("{}\n{}\n", content.trim_end(), new_entry)
    };

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

    println!("Loaded configuration:");
    println!("  Unbound config path: {:?}", config.unbound_config_path);
    println!("  Authorized domains: {}", config.domains.len());
    for domain in &config.domains {
        println!("    - {}", domain.name);
    }

    // Build the router
    let app = Router::new()
        .route("/update", post(update_handler))
        .with_state(config);

    // Start the server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();

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
        let toml_content = r#"
unbound_config_path = "/etc/unbound/unbound.conf"

[[domains]]
name = "home.example.com"
key = "secret-key-1"

[[domains]]
name = "server.example.com"
key = "secret-key-2"
"#;
        let config: Config = toml::from_str(toml_content).unwrap();
        assert_eq!(config.unbound_config_path, PathBuf::from("/etc/unbound/unbound.conf"));
        assert_eq!(config.domains.len(), 2);
        assert_eq!(config.domains[0].name, "home.example.com");
        assert_eq!(config.domains[0].key, "secret-key-1");
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
    fn test_update_unbound_config_new_entry() {
        use std::io::Write;
        let temp_dir = std::env::temp_dir();
        let config_path = temp_dir.join("test_unbound.conf");

        // Create initial config
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "server:").unwrap();
        writeln!(file, "  verbosity: 1").unwrap();
        drop(file);

        // Add new entry
        update_unbound_config(&config_path, "test.example.com", "192.168.1.1").unwrap();

        // Verify
        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("local-data: \"test.example.com IN A 192.168.1.1\""));

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
}
