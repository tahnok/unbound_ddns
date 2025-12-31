use axum::{Json, Router, routing::post};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct UpdateRequest {
    hostname: String,
    ip: String,
}

#[derive(Serialize, Deserialize)]
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
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

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

    #[tokio::test]
    async fn test_update_handler_function() {
        let request = UpdateRequest {
            hostname: "example.com".to_string(),
            ip: "10.0.0.1".to_string(),
        };

        let response = update_handler(Json(request)).await;

        assert!(response.0.success);
        assert_eq!(response.0.message, "Updated example.com to 10.0.0.1");
    }

    #[tokio::test]
    async fn test_update_endpoint_integration() {
        let app = Router::new().route("/update", post(update_handler));

        let request = Request::builder()
            .method("POST")
            .uri("/update")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"hostname": "test.local", "ip": "172.16.0.1"}"#,
            ))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        let update_response: UpdateResponse = serde_json::from_str(&body_str).unwrap();

        assert!(update_response.success);
        assert_eq!(update_response.message, "Updated test.local to 172.16.0.1");
    }
}
