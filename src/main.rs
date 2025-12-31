use axum::{Json, Router, routing::post};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct UpdateRequest {
    hostname: String,
    ip: String,
}

#[derive(Serialize)]
struct UpdateResponse {
    success: bool,
    message: String,
}

async fn update_handler(Json(payload): Json<UpdateRequest>) -> Json<UpdateResponse> {
    println!(
        "Received update request for hostname: {} with IP: {}",
        payload.hostname, payload.ip
    );

    Json(UpdateResponse {
        success: true,
        message: format!("Updated {} to {}", payload.hostname, payload.ip),
    })
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/update", post(update_handler));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    println!("Server running on http://0.0.0.0:3000");

    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_request_deserialization() {
        let json = r#"{"hostname": "test.example.com", "ip": "192.168.1.1"}"#;
        let request: UpdateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.hostname, "test.example.com");
        assert_eq!(request.ip, "192.168.1.1");
    }

    #[test]
    fn test_update_response_serialization() {
        let response = UpdateResponse {
            success: true,
            message: "Test message".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"message\":\"Test message\""));
    }
}
