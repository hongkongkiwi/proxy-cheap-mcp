//! MCP server over Streamable HTTP and the legacy HTTP+SSE transport.
//!
//! Streamable HTTP: `POST /mcp` carries JSON-RPC requests, `GET /mcp` opens a
//! server-sent-events stream for server notifications, `DELETE /mcp` ends a
//! session.
//!
//! Legacy SSE: `GET /sse` opens the event stream and announces
//! `/message?sessionId=...`; JSON-RPC requests are POSTed to that endpoint and
//! responses are delivered over the stream.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tower_http::cors::{AllowOrigin, CorsLayer};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::mcp::protocol::{McpServer, parse_error};

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub bind: String,
    pub path: String,
    pub allow_remote: bool,
    pub cors_origins: Vec<String>,
    pub streamable: bool,
    pub sse: bool,
}

impl HttpConfig {
    pub fn validate(&self) -> Result<()> {
        if !self.path.starts_with('/') {
            return Err(Error::Config(format!(
                "MCP HTTP path must start with '/', got {:?}",
                self.path
            )));
        }
        if !self.streamable && !self.sse {
            return Err(Error::Config(
                "MCP HTTP transport needs at least one of streamable or SSE enabled".to_string(),
            ));
        }
        Ok(())
    }
}

struct AppState {
    server: Arc<McpServer>,
    sessions: Arc<RwLock<HashMap<String, mpsc::Sender<String>>>>,
    broadcast: broadcast::Sender<String>,
}

impl AppState {
    async fn prune_sessions(&self) {
        self.sessions
            .write()
            .await
            .retain(|_, sender| !sender.is_closed());
    }
}

pub async fn serve(server: Arc<McpServer>, config: HttpConfig) -> Result<()> {
    config.validate()?;
    let listener = TcpListener::bind(&config.bind)
        .await
        .map_err(|error| Error::Config(format!("could not bind {}: {error}", config.bind)))?;
    serve_with_listener(server, config, listener).await
}

/// Run the HTTP transports on an already-bound listener (used by tests).
pub async fn serve_with_listener(
    server: Arc<McpServer>,
    config: HttpConfig,
    listener: TcpListener,
) -> Result<()> {
    config.validate()?;
    let local = listener
        .local_addr()
        .map_err(|error| Error::Config(format!("could not inspect listener: {error}")))?;
    if !local.ip().is_loopback() && !config.allow_remote {
        return Err(Error::Config(format!(
            "refusing to bind MCP HTTP to {local}: pass --allow-remote to expose it"
        )));
    }

    let (broadcast, _) = broadcast::channel::<String>(64);
    let state = Arc::new(AppState {
        server,
        sessions: Arc::new(RwLock::new(HashMap::new())),
        broadcast,
    });

    let mut router = Router::new().route("/health", get(health));
    if config.streamable {
        router = router.route(
            &config.path,
            post(streamable_post)
                .get(streamable_get)
                .delete(streamable_delete),
        );
    }
    if config.sse {
        router = router
            .route("/sse", get(sse_connect))
            .route("/message", post(sse_message));
    }
    if let Some(cors) = cors_layer(&config) {
        router = router.layer(cors);
    }
    let router = router.with_state(state);

    tracing::info!(
        %local,
        streamable = config.streamable,
        sse = config.sse,
        path = %config.path,
        "MCP HTTP server listening"
    );
    axum::serve(listener, router)
        .await
        .map_err(|error| Error::Other(anyhow::anyhow!("MCP HTTP server failed: {error}")))
}

async fn health() -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "status": "ok",
        "server": "proxy-cheap",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn streamable_post(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> axum::response::Response {
    let message: Value = match serde_json::from_slice(&body) {
        Ok(message) => message,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(parse_error(error.to_string())),
            )
                .into_response();
        }
    };
    match state.server.handle(message).await {
        Some(response) => axum::Json(response).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

async fn streamable_get(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let receiver = state.broadcast.subscribe();
    let stream = BroadcastStream::new(receiver).filter_map(|message| match message {
        Ok(text) => Some(Ok::<_, Infallible>(
            Event::default().event("message").data(text),
        )),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(keep_alive()).into_response()
}

async fn streamable_delete() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn sse_connect(State(state): State<Arc<AppState>>) -> axum::response::Response {
    state.prune_sessions().await;
    let (sender, receiver) = mpsc::channel::<String>(64);
    let session_id = Uuid::new_v4().to_string();
    state
        .sessions
        .write()
        .await
        .insert(session_id.clone(), sender);

    let endpoint = format!("/message?sessionId={session_id}");
    let initial = tokio_stream::once(Ok::<_, Infallible>(
        Event::default().event("endpoint").data(endpoint),
    ));
    let messages = ReceiverStream::new(receiver)
        .map(|text| Ok::<_, Infallible>(Event::default().event("message").data(text)));
    Sse::new(initial.chain(messages))
        .keep_alive(keep_alive())
        .into_response()
}

async fn sse_message(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
    body: Bytes,
) -> axum::response::Response {
    let Some(session_id) = params.get("sessionId").cloned() else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": "missing sessionId" })),
        )
            .into_response();
    };
    let sender = { state.sessions.read().await.get(&session_id).cloned() };
    let Some(sender) = sender else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({ "error": "unknown or expired session" })),
        )
            .into_response();
    };
    let message: Value = match serde_json::from_slice(&body) {
        Ok(message) => message,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(parse_error(error.to_string())),
            )
                .into_response();
        }
    };
    if let Some(response) = state.server.handle(message).await {
        let text = match serde_json::to_string(&response) {
            Ok(text) => text,
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({ "error": error.to_string() })),
                )
                    .into_response();
            }
        };
        if sender.send(text).await.is_err() {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({ "error": "session closed" })),
            )
                .into_response();
        }
    }
    StatusCode::ACCEPTED.into_response()
}

fn keep_alive() -> KeepAlive {
    KeepAlive::new()
        .interval(Duration::from_secs(15))
        .text("keep-alive")
}

fn cors_layer(config: &HttpConfig) -> Option<CorsLayer> {
    if config.cors_origins.is_empty() {
        return None;
    }
    let layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_headers([
            header::CONTENT_TYPE,
            header::ACCEPT,
            header::AUTHORIZATION,
            HeaderName::from_static("mcp-session-id"),
            HeaderName::from_static("last-event-id"),
        ]);
    if config.cors_origins.iter().any(|origin| origin == "*") {
        Some(layer.allow_origin(tower_http::cors::Any))
    } else {
        let origins: Vec<HeaderValue> = config
            .cors_origins
            .iter()
            .filter_map(|origin| origin.parse().ok())
            .collect();
        Some(layer.allow_origin(AllowOrigin::list(origins)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_config() {
        let mut config = HttpConfig {
            bind: "127.0.0.1:0".to_string(),
            path: "mcp".to_string(),
            allow_remote: false,
            cors_origins: vec![],
            streamable: true,
            sse: false,
        };
        assert!(config.validate().is_err());
        config.path = "/mcp".to_string();
        config.streamable = false;
        assert!(config.validate().is_err());
        config.sse = true;
        assert!(config.validate().is_ok());
    }
}
