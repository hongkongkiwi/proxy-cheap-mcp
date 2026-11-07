//! Webhook signature verification and a small receiver for Proxy-Cheap events.
//!
//! Proxy-Cheap signs `event name + event id + raw body` with HMAC-SHA256 using
//! the webhook secret and sends the result as `Webhook-Signature: sha256=<hex>`.

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::net::TcpListener;

use crate::error::{Error, Result};

pub const EVENT_MAINTENANCE_CREATED: &str = "proxy.maintenance_window.created";
pub const EVENT_MAINTENANCE_CANCELLED: &str = "proxy.maintenance_window.cancelled";
pub const EVENT_STATUS_CHANGED: &str = "proxy.status.changed";
pub const EVENT_BANDWIDTH_ADDED: &str = "proxy.bandwidth.added";

pub const EVENTS: [&str; 4] = [
    EVENT_MAINTENANCE_CREATED,
    EVENT_MAINTENANCE_CANCELLED,
    EVENT_STATUS_CHANGED,
    EVENT_BANDWIDTH_ADDED,
];

/// Compute the value expected in the `Webhook-Signature` header.
pub fn signature(secret: &str, event: &str, event_id: &str, body: &[u8]) -> Result<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|error| Error::InvalidInput(format!("invalid webhook secret: {error}")))?;
    mac.update(event.as_bytes());
    mac.update(event_id.as_bytes());
    mac.update(body);
    Ok(format!(
        "sha256={}",
        hex::encode(mac.finalize().into_bytes())
    ))
}

/// Verify a signature header in constant time.
pub fn verify(secret: &str, event: &str, event_id: &str, body: &[u8], header: &str) -> bool {
    let Some(expected_hex) = header.trim().strip_prefix("sha256=") else {
        return false;
    };
    let Ok(expected) = hex::decode(expected_hex) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(event.as_bytes());
    mac.update(event_id.as_bytes());
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

#[derive(Debug)]
pub struct WebhookEvent {
    pub event: String,
    pub id: Option<String>,
    pub verified: bool,
    pub payload: Value,
}

impl WebhookEvent {
    pub fn to_json(&self) -> Value {
        json!({
            "event": self.event,
            "id": self.id,
            "verified": self.verified,
            "payload": self.payload,
        })
    }
}

/// Validate headers and body into a structured event.
pub fn parse_event(
    event: Option<&str>,
    event_id: Option<&str>,
    signature_header: Option<&str>,
    body: &[u8],
    secret: Option<&str>,
) -> Result<WebhookEvent> {
    let event = event
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::InvalidInput("missing Webhook-Event header".to_string()))?;
    let event_id = event_id.filter(|value| !value.is_empty());
    let verified = match secret {
        Some(secret) => {
            let header = signature_header.ok_or(Error::SignatureMismatch)?;
            let id = event_id.unwrap_or("");
            if !verify(secret, event, id, body, header) {
                return Err(Error::SignatureMismatch);
            }
            true
        }
        None => false,
    };
    let payload = serde_json::from_slice(body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).to_string()));
    Ok(WebhookEvent {
        event: event.to_string(),
        id: event_id.map(str::to_string),
        verified,
        payload,
    })
}

#[derive(Debug, Clone)]
pub struct WebhookConfig {
    pub bind: String,
    pub path: String,
    pub secret: Option<String>,
    pub allow_remote: bool,
}

#[derive(Clone)]
struct AppState {
    secret: Option<String>,
}

/// Run the webhook receiver until the process is stopped.
pub async fn serve(config: WebhookConfig) -> Result<()> {
    let listener = TcpListener::bind(&config.bind)
        .await
        .map_err(|error| Error::Config(format!("could not bind {}: {error}", config.bind)))?;
    serve_with_listener(config, listener).await
}

/// Run the webhook receiver on an already-bound listener (used by tests).
pub async fn serve_with_listener(config: WebhookConfig, listener: TcpListener) -> Result<()> {
    let local = listener
        .local_addr()
        .map_err(|error| Error::Config(format!("could not inspect listener: {error}")))?;
    if !local.ip().is_loopback() && !config.allow_remote {
        return Err(Error::Config(format!(
            "refusing to bind webhook receiver to {local}: pass --allow-remote to expose it"
        )));
    }
    if config.secret.is_none() {
        tracing::warn!(
            "webhook receiver started without a secret; signatures will not be verified"
        );
    }
    let state = AppState {
        secret: config.secret,
    };
    let router = Router::new()
        .route("/health", get(health))
        .route(&config.path, post(receive))
        .with_state(Arc::new(state));
    tracing::info!(%local, path = %config.path, "webhook receiver listening");
    axum::serve(listener, router)
        .await
        .map_err(|error| Error::Other(anyhow::anyhow!("webhook receiver failed: {error}")))
}

async fn health() -> impl IntoResponse {
    axum::Json(json!({ "status": "ok" }))
}

async fn receive(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> Response {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    match parse_event(
        header("Webhook-Event"),
        header("Webhook-Id"),
        header("Webhook-Signature"),
        &body,
        state.secret.as_deref(),
    ) {
        Ok(event) => {
            println!("{}", event.to_json());
            (StatusCode::OK, axum::Json(json!({ "ok": true }))).into_response()
        }
        Err(Error::SignatureMismatch) => (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({ "error": "invalid signature" })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "0qFbBP1zE5MuSMy";

    #[test]
    fn round_trips_a_signature() {
        let body = br#"{"proxyId":1410379,"oldStatus":"ACTIVE","status":"CANCELED"}"#;
        let header = signature(SECRET, EVENT_STATUS_CHANGED, "evt-1", body).unwrap();
        assert!(verify(SECRET, EVENT_STATUS_CHANGED, "evt-1", body, &header));
    }

    #[test]
    fn rejects_tampered_body_and_event() {
        let body = br#"{"proxyId":1}"#;
        let header = signature(SECRET, EVENT_STATUS_CHANGED, "evt-1", body).unwrap();
        assert!(!verify(
            SECRET,
            EVENT_STATUS_CHANGED,
            "evt-1",
            br#"{"proxyId":2}"#,
            &header
        ));
        assert!(!verify(
            SECRET,
            EVENT_BANDWIDTH_ADDED,
            "evt-1",
            body,
            &header
        ));
        assert!(!verify(
            SECRET,
            EVENT_STATUS_CHANGED,
            "evt-2",
            body,
            &header
        ));
        assert!(!verify(
            "wrong",
            EVENT_STATUS_CHANGED,
            "evt-1",
            body,
            &header
        ));
    }

    #[test]
    fn rejects_malformed_headers() {
        assert!(!verify(SECRET, "e", "id", b"{}", "not-a-signature"));
        assert!(!verify(SECRET, "e", "id", b"{}", "sha256=zzzz"));
        assert!(!verify(SECRET, "e", "id", b"{}", "md5=abcd"));
    }

    #[test]
    fn parses_verified_event() {
        let body = br#"{"proxyId":1410379,"trafficInGb":1}"#;
        let header = signature(SECRET, EVENT_BANDWIDTH_ADDED, "evt-9", body).unwrap();
        let event = parse_event(
            Some(EVENT_BANDWIDTH_ADDED),
            Some("evt-9"),
            Some(&header),
            body,
            Some(SECRET),
        )
        .unwrap();
        assert!(event.verified);
        assert_eq!(event.payload["trafficInGb"], 1);
    }

    #[test]
    fn requires_a_signature_when_secret_is_configured() {
        let error =
            parse_event(Some(EVENT_STATUS_CHANGED), None, None, b"{}", Some(SECRET)).unwrap_err();
        assert!(matches!(error, Error::SignatureMismatch));
    }
}
