mod common;

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use proxy_cheap_mcp::ProxyCheapClient;
use proxy_cheap_mcp::config::ClientConfig;
use proxy_cheap_mcp::mcp::McpServer;
use proxy_cheap_mcp::mcp::http::{HttpConfig, serve_with_listener};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_stream::{Stream, StreamExt};

struct TestServer {
    base_url: String,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn mcp_server(base_url: &str) -> Arc<McpServer> {
    let config = ClientConfig {
        api_key: Some("key".to_string()),
        api_secret: Some("secret".to_string()),
        session_token: None,
        session_token_expires_at: None,
        session_token_environment: None,
        base_url: base_url.to_string(),
        timeout_secs: 5,
        max_retries: 0,
    };
    Arc::new(McpServer::new(Arc::new(
        ProxyCheapClient::new(&config).unwrap(),
    )))
}

async fn start_http(base_url: &str) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = HttpConfig {
        bind: address.to_string(),
        path: "/mcp".to_string(),
        allow_remote: false,
        cors_origins: vec!["*".to_string()],
        streamable: true,
        sse: true,
    };
    let server = mcp_server(base_url);
    let handle = tokio::spawn(async move {
        serve_with_listener(server, config, listener).await.unwrap();
    });
    TestServer {
        base_url: format!("http://{address}"),
        handle,
    }
}

async fn rpc(http: &reqwest::Client, base: &str, body: Value) -> reqwest::Response {
    http.post(format!("{base}/mcp"))
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn streamable_http_roundtrip() {
    let mock = common::start().await;
    let server = start_http(mock.base_url()).await;
    let http = reqwest::Client::new();

    let init = rpc(
        &http,
        &server.base_url,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test"}}
        }),
    )
    .await;
    assert_eq!(init.status(), 200);
    assert_eq!(
        init.headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "application/json"
    );
    let init: Value = init.json().await.unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "proxy-cheap");

    let notification = rpc(
        &http,
        &server.base_url,
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
    assert_eq!(notification.status(), 202);

    let tools = rpc(
        &http,
        &server.base_url,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await
    .json::<Value>()
    .await
    .unwrap();
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 25);

    let call = rpc(
        &http,
        &server.base_url,
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "list_services", "arguments": {}}
        }),
    )
    .await
    .json::<Value>()
    .await
    .unwrap();
    assert_eq!(call["result"]["isError"], false);
    assert_eq!(
        call["result"]["structuredContent"]["services"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let health = http
        .get(format!("{}/health", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);

    let delete = http
        .delete(format!("{}/mcp", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), 204);
}

#[tokio::test]
async fn malformed_json_is_a_parse_error() {
    let mock = common::start().await;
    let server = start_http(mock.base_url()).await;
    let response = reqwest::Client::new()
        .post(format!("{}/mcp", server.base_url))
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32700);
}

#[tokio::test]
async fn cors_headers_are_served_when_configured() {
    let mock = common::start().await;
    let server = start_http(mock.base_url()).await;
    let response = reqwest::Client::new()
        .post(format!("{}/mcp", server.base_url))
        .header("origin", "http://localhost:3000")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|value| value.to_str().ok()),
        Some("*")
    );
}

#[tokio::test]
async fn legacy_sse_transport_roundtrip() {
    let mock = common::start().await;
    let server = start_http(mock.base_url()).await;
    let http = reqwest::Client::new();

    let response = http
        .get(format!("{}/sse", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .split(';')
            .next(),
        Some("text/event-stream")
    );

    let mut stream = SseReader::new(response);
    let endpoint = stream.next_data().await;
    assert!(
        endpoint.starts_with("/message?sessionId="),
        "unexpected endpoint event: {endpoint}"
    );

    let posted = http
        .post(format!("{}{}", server.base_url, endpoint))
        .json(&json!({
            "jsonrpc": "2.0", "id": 11, "method": "initialize",
            "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "test"}}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(posted.status(), 202);

    let data = stream.next_data().await;
    let response: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(response["id"], 11);
    assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
}

#[tokio::test]
async fn sse_rejects_unknown_sessions() {
    let mock = common::start().await;
    let server = start_http(mock.base_url()).await;
    let response = reqwest::Client::new()
        .post(format!("{}/message?sessionId=missing", server.base_url))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn refuses_non_loopback_bind_without_opt_in() {
    let mock = common::start().await;
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let config = HttpConfig {
        bind: "0.0.0.0:0".to_string(),
        path: "/mcp".to_string(),
        allow_remote: false,
        cors_origins: vec![],
        streamable: true,
        sse: false,
    };
    let result = serve_with_listener(mcp_server(mock.base_url()), config, listener).await;
    assert!(result.is_err());
}

struct SseReader {
    stream: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>,
    buffer: String,
}

impl SseReader {
    fn new(response: reqwest::Response) -> Self {
        Self {
            stream: Box::pin(response.bytes_stream()),
            buffer: String::new(),
        }
    }

    async fn next_data(&mut self) -> String {
        loop {
            if let Some(event) = take_event(&mut self.buffer) {
                if let Some(data) = event.lines().find_map(|line| line.strip_prefix("data: ")) {
                    return data.to_string();
                }
                continue;
            }
            let chunk = tokio::time::timeout(Duration::from_secs(10), self.stream.next())
                .await
                .expect("timed out waiting for SSE data")
                .expect("SSE stream ended")
                .expect("SSE stream error");
            self.buffer.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
}

fn take_event(buffer: &mut String) -> Option<String> {
    if let Some(position) = buffer.find("\r\n\r\n") {
        let event = buffer[..position].to_string();
        buffer.drain(..position + 4);
        return Some(event.replace("\r\n", "\n"));
    }
    if let Some(position) = buffer.find("\n\n") {
        let event = buffer[..position].to_string();
        buffer.drain(..position + 2);
        return Some(event);
    }
    None
}
