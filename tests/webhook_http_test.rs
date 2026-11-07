use std::time::Duration;

use proxy_cheap_mcp::webhook::{self, WebhookConfig, serve_with_listener};
use serde_json::Value;
use tokio::net::TcpListener;

struct Receiver {
    base_url: String,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn start(secret: Option<&str>) -> Receiver {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = WebhookConfig {
        bind: address.to_string(),
        path: "/webhook".to_string(),
        secret: secret.map(str::to_string),
        allow_remote: false,
    };
    let handle = tokio::spawn(async move {
        serve_with_listener(config, listener).await.unwrap();
    });
    let base_url = format!("http://{address}");
    let client = reqwest::Client::new();
    for _ in 0..100 {
        if client
            .get(format!("{base_url}/health"))
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Receiver { base_url, handle }
}

#[tokio::test]
async fn accepts_a_valid_signed_delivery() {
    let receiver = start(Some("test-secret")).await;
    let body = r#"{"proxyId":1410379,"trafficInGb":1}"#;
    let signature = webhook::signature(
        "test-secret",
        "proxy.bandwidth.added",
        "evt-1",
        body.as_bytes(),
    )
    .unwrap();

    let response = reqwest::Client::new()
        .post(format!("{}/webhook", receiver.base_url))
        .header("webhook-event", "proxy.bandwidth.added")
        .header("webhook-id", "evt-1")
        .header("webhook-signature", signature)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let value: Value = response.json().await.unwrap();
    assert_eq!(value["ok"], true);
}

#[tokio::test]
async fn rejects_a_delivery_with_a_bad_signature() {
    let receiver = start(Some("test-secret")).await;
    let response = reqwest::Client::new()
        .post(format!("{}/webhook", receiver.base_url))
        .header("webhook-event", "proxy.status.changed")
        .header("webhook-id", "evt-1")
        .header("webhook-signature", "sha256=00")
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn rejects_a_delivery_without_an_event_header() {
    let receiver = start(Some("test-secret")).await;
    let response = reqwest::Client::new()
        .post(format!("{}/webhook", receiver.base_url))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn accepts_unsigned_deliveries_when_no_secret_is_configured() {
    let receiver = start(None).await;
    let response = reqwest::Client::new()
        .post(format!("{}/webhook", receiver.base_url))
        .header("webhook-event", "proxy.status.changed")
        .header("content-type", "application/json")
        .body(r#"{"proxyId":1,"status":"ACTIVE"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn refuses_a_non_loopback_bind_without_opt_in() {
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let config = WebhookConfig {
        bind: "0.0.0.0:0".to_string(),
        path: "/webhook".to_string(),
        secret: None,
        allow_remote: false,
    };
    let result = serve_with_listener(config, listener).await;
    assert!(result.is_err());
}
