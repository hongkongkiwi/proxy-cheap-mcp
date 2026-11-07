//! JSON-RPC 2.0 and MCP protocol handling, shared by every transport.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::api::ProxyCheapClient;
use crate::mcp::tools::{self, ToolError};
use crate::skill;

pub const LATEST_PROTOCOL_VERSION: &str = "2026-07-28";

pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 4] =
    ["2026-07-28", "2025-06-18", "2025-03-26", "2024-11-05"];

pub const INSTRUCTIONS: &str = "\
Proxy-Cheap API server. Tools cover service discovery, price calculation, ordering, \
proxy management (extend, bandwidth, whitelist, protocol, authentication, auto-extend, \
IP rotation), account balance, legacy ordering, and webhook signature verification. \
Ordering and purchase tools spend real account balance: call price_order first, show the \
total, and get explicit user confirmation before place_order, extend_proxy, \
buy_proxy_bandwidth, or legacy_place_order. Credentials come from PROXY_CHEAP_API_KEY and \
PROXY_CHEAP_API_SECRET (or the proxy-cheap config file). Public endpoints (services, setup, \
price) work without credentials. The bundled usage guide is the resource \
proxy-cheap://skill. This server is unofficial and not affiliated with Proxy-Cheap.";

#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }

    fn method_not_found(method: &str) -> Self {
        Self::new(-32601, format!("method not found: {method}"))
    }
}

pub struct McpServer {
    client: Arc<ProxyCheapClient>,
}

impl McpServer {
    pub fn new(client: Arc<ProxyCheapClient>) -> Self {
        Self { client }
    }

    /// Handle one JSON-RPC message or a batch. Returns `None` when the input
    /// consisted only of notifications (transports should answer 202/204).
    pub async fn handle(&self, message: Value) -> Option<Value> {
        if let Value::Array(items) = message {
            if items.is_empty() {
                return Some(error_response(
                    Value::Null,
                    -32600,
                    "invalid request: empty batch",
                ));
            }
            let mut responses = Vec::new();
            for item in items {
                if let Some(response) = self.handle_one(item).await {
                    responses.push(response);
                }
            }
            if responses.is_empty() {
                None
            } else {
                Some(Value::Array(responses))
            }
        } else {
            self.handle_one(message).await
        }
    }

    async fn handle_one(&self, message: Value) -> Option<Value> {
        let Some(object) = message.as_object() else {
            return Some(error_response(
                Value::Null,
                -32600,
                "invalid request: expected a JSON object",
            ));
        };

        let id = object.get("id").cloned();
        let method = object.get("method").and_then(Value::as_str);
        let params = object.get("params").cloned().unwrap_or(Value::Null);

        let Some(method) = method else {
            return id.map(|id| error_response(id, -32600, "invalid request: missing method"));
        };

        // Notifications never receive a response.
        if id.is_none() {
            if method == "notifications/initialized"
                || method == "notifications/cancelled"
                || method == "notifications/progress"
                || method == "notifications/roots/list_changed"
            {
                tracing::debug!(%method, "notification");
            } else {
                tracing::debug!(%method, "ignored notification");
            }
            return None;
        }

        let id = id.unwrap();
        match self.dispatch(method, params).await {
            Ok(result) => Some(success_response(id, result)),
            Err(error) => Some(error_response(id, error.code, error.message)),
        }
    }

    async fn dispatch(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::definitions() })),
            "tools/call" => self.call_tool(&params).await,
            "resources/list" => Ok(json!({
                "resources": [{
                    "uri": "proxy-cheap://skill",
                    "name": "Proxy-Cheap usage guide",
                    "description": "Bundled SKILL.md: commands, MCP tools, ordering workflow, and safety rules.",
                    "mimeType": "text/markdown"
                }]
            })),
            "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
            "resources/read" => self.read_resource(&params),
            "prompts/list" => Ok(json!({ "prompts": prompts() })),
            "prompts/get" => self.get_prompt(&params),
            "logging/setLevel" => Ok(json!({})),
            other => Err(RpcError::method_not_found(other)),
        }
    }

    fn initialize(&self, params: &Value) -> Value {
        let requested = params.get("protocolVersion").and_then(Value::as_str);
        let version = requested
            .filter(|version| SUPPORTED_PROTOCOL_VERSIONS.contains(version))
            .unwrap_or(LATEST_PROTOCOL_VERSION);
        json!({
            "protocolVersion": version,
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "subscribe": false, "listChanged": false },
                "prompts": { "listChanged": false },
                "logging": {}
            },
            "serverInfo": {
                "name": "proxy-cheap",
                "title": "Proxy-Cheap",
                "version": env!("CARGO_PKG_VERSION"),
                "websiteUrl": "https://github.com/hongkongkiwi/proxy-cheap-mcp"
            },
            "instructions": INSTRUCTIONS
        })
    }

    async fn call_tool(&self, params: &Value) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid_params("tools/call requires a tool name"))?;
        let arguments = params.get("arguments").cloned();
        match tools::call(self.client.clone(), name, arguments).await {
            Ok(value) => {
                let text = serde_json::to_string_pretty(&value).unwrap_or_default();
                let mut result = json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false
                });
                // structuredContent must be an object per the MCP spec.
                if value.is_object() {
                    result["structuredContent"] = value;
                }
                Ok(result)
            }
            Err(ToolError::Unknown(name)) => {
                Err(RpcError::invalid_params(format!("unknown tool: {name}")))
            }
            Err(ToolError::Failed(error)) => Ok(json!({
                "content": [{ "type": "text", "text": format!("error: {error}") }],
                "isError": true
            })),
        }
    }

    fn read_resource(&self, params: &Value) -> Result<Value, RpcError> {
        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid_params("resources/read requires a uri"))?;
        if uri != "proxy-cheap://skill" {
            return Err(RpcError::invalid_params(format!("unknown resource: {uri}")));
        }
        Ok(json!({
            "contents": [{
                "uri": uri,
                "mimeType": "text/markdown",
                "text": skill::SKILL_MD
            }]
        }))
    }

    fn get_prompt(&self, params: &Value) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid_params("prompts/get requires a name"))?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let text = match name {
            "order_proxy" => {
                let service = arguments
                    .get("service_id")
                    .and_then(Value::as_str)
                    .unwrap_or("any");
                let requirements = arguments
                    .get("requirements")
                    .and_then(Value::as_str)
                    .unwrap_or("not specified");
                format!(
                    "Help me order a Proxy-Cheap proxy.\n\nRequirements: {requirements}\nPreferred service: {service}\n\n\
                     Steps:\n1. Call list_services if the service id is unknown.\n\
                     2. Call get_service_options with the service (and plan if required) to get valid ids.\n\
                     3. Choose a configuration that matches the requirements; state every assumption.\n\
                     4. Call price_order and show the total, currency, and what is included.\n\
                     5. Ask me to confirm the purchase.\n\
                     6. Only after confirmation, call place_order, then get_order_proxies and show the connection details.\n\n\
                     Do not place the order before I confirm."
                )
            }
            "audit_proxies" => "\
                Call list_proxies and summarise the account. For each proxy report id, status, network type, \
                country, public IP, protocol, authentication method, expiry date, bandwidth used versus total, \
                and auto-extend state. Flag expired or soon-expiring proxies (within 7 days), inactive proxies, \
                proxies above 80 percent bandwidth, and proxies with auto-extend disabled. Do not change anything \
                without asking."
                .to_string(),
            other => {
                return Err(RpcError::invalid_params(format!("unknown prompt: {other}")));
            }
        };
        Ok(json!({
            "description": if name == "order_proxy" { "Guided Proxy-Cheap ordering workflow" } else { "Proxy fleet audit" },
            "messages": [{
                "role": "user",
                "content": { "type": "text", "text": text }
            }]
        }))
    }
}

fn prompts() -> Value {
    json!([
        {
            "name": "order_proxy",
            "title": "Order a proxy",
            "description": "Walks through service discovery, pricing, confirmation, and ordering.",
            "arguments": [
                { "name": "service_id", "description": "Optional service id, for example static-residential-ipv4.", "required": false },
                { "name": "requirements", "description": "Country, quantity, bandwidth, period, or other needs.", "required": false }
            ]
        },
        {
            "name": "audit_proxies",
            "title": "Audit proxies",
            "description": "Summarises status, expiry, bandwidth, and auto-extend across the account without changes.",
            "arguments": []
        }
    ])
}

fn success_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() }
    })
}

/// Build a parse-error response for transports that receive invalid JSON.
pub fn parse_error(detail: impl Into<String>) -> Value {
    error_response(
        Value::Null,
        -32700,
        format!("parse error: {}", detail.into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClientConfig;

    fn server() -> McpServer {
        let config = ClientConfig::public(Some("http://127.0.0.1:1".to_string())).unwrap();
        McpServer::new(Arc::new(ProxyCheapClient::new(&config).unwrap()))
    }

    #[tokio::test]
    async fn initializes_with_requested_version() {
        let response = server()
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name":"test"} }
            }))
            .await
            .unwrap();
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(response["result"]["serverInfo"]["name"], "proxy-cheap");
        assert_eq!(
            response["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
    }

    #[tokio::test]
    async fn falls_back_to_latest_version() {
        let response = server()
            .handle(json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "1999-01-01" }
            }))
            .await
            .unwrap();
        assert_eq!(
            response["result"]["protocolVersion"],
            LATEST_PROTOCOL_VERSION
        );
    }

    #[tokio::test]
    async fn lists_tools_and_resources() {
        let server = server();
        let tools = server
            .handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
            .await
            .unwrap();
        assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 25);
        let resources = server
            .handle(json!({"jsonrpc":"2.0","id":3,"method":"resources/list"}))
            .await
            .unwrap();
        assert_eq!(
            resources["result"]["resources"][0]["uri"],
            "proxy-cheap://skill"
        );
    }

    #[tokio::test]
    async fn reads_skill_resource() {
        let response = server()
            .handle(json!({
                "jsonrpc":"2.0","id":4,"method":"resources/read",
                "params":{"uri":"proxy-cheap://skill"}
            }))
            .await
            .unwrap();
        let text = response["result"]["contents"][0]["text"].as_str().unwrap();
        assert!(text.contains("proxy-cheap services list"));
    }

    #[tokio::test]
    async fn rejects_unknown_tools_and_methods() {
        let server = server();
        let tool = server
            .handle(json!({
                "jsonrpc":"2.0","id":5,"method":"tools/call",
                "params":{"name":"order_everything","arguments":{}}
            }))
            .await
            .unwrap();
        assert_eq!(tool["error"]["code"], -32602);
        let method = server
            .handle(json!({"jsonrpc":"2.0","id":6,"method":"no/such/method"}))
            .await
            .unwrap();
        assert_eq!(method["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn notifications_get_no_response() {
        let response = server()
            .handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
        assert!(response.is_none());
    }

    #[tokio::test]
    async fn handles_batches_and_drops_notifications() {
        let response = server()
            .handle(json!([
                {"jsonrpc": "2.0", "id": 1, "method": "ping"},
                {"jsonrpc": "2.0", "method": "notifications/initialized"},
                {"jsonrpc": "2.0", "id": 2, "method": "ping"}
            ]))
            .await
            .unwrap();
        let responses = response.as_array().unwrap();
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["id"], 1);
        assert_eq!(responses[1]["id"], 2);
    }

    #[tokio::test]
    async fn rejects_an_empty_batch() {
        let response = server().handle(json!([])).await.unwrap();
        assert_eq!(response["error"]["code"], -32600);
    }

    #[tokio::test]
    async fn serves_ordering_prompt() {
        let response = server()
            .handle(json!({
                "jsonrpc":"2.0","id":7,"method":"prompts/get",
                "params":{"name":"order_proxy","arguments":{"service_id":"rotating-residential"}}
            }))
            .await
            .unwrap();
        let text = response["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("price_order"));
        assert!(text.contains("rotating-residential"));
    }

    #[tokio::test]
    async fn tool_failure_is_a_tool_result_not_an_rpc_error() {
        let response = server()
            .handle(json!({
                "jsonrpc":"2.0","id":8,"method":"tools/call",
                "params":{"name":"get_proxy","arguments":{}}
            }))
            .await
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("invalid tool arguments")
        );
    }
}
