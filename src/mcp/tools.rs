//! MCP tool definitions and dispatch.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::api::{
    AuthType, AutoExtend, ExtendPeriodRequest, LegacyOrderRequest, OrderRequest, Period,
    PriceRequest, Protocol, ProxyCheapClient,
};
use crate::error::{Error, Result};
use crate::webhook;

pub type ToolFuture = Pin<Box<dyn Future<Output = Result<Value>> + Send>>;

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Value,
    pub call: fn(Arc<ProxyCheapClient>, Value) -> ToolFuture,
}

#[derive(Debug)]
pub enum ToolError {
    Unknown(String),
    Failed(Error),
}

macro_rules! tool {
    ($name:literal, $description:literal, $args:ty, $handler:path) => {
        ToolSpec {
            name: $name,
            description: $description,
            schema: || schema::<$args>(),
            call: |client, arguments| Box::pin($handler(client, arguments)),
        }
    };
}

/// All tools, in declaration order.
pub fn specs() -> &'static [ToolSpec] {
    static SPECS: OnceLock<Vec<ToolSpec>> = OnceLock::new();
    SPECS.get_or_init(|| {
        vec![
            tool!(
                "list_services",
                "List the services and plans that Proxy-Cheap can order. Public endpoint. Start here to discover service ids.",
                EmptyArgs,
                handle_list_services
            ),
            tool!(
                "get_service_options",
                "Fetch the order options for a service: plans, packages, countries, regions, cities, ISPs and available periods. Public endpoint. Use the returned ids as order parameters.",
                ServiceArgs,
                handle_get_service_options
            ),
            tool!(
                "price_order",
                "Calculate the price of a Proxy-Cheap order without placing it. Public endpoint. Run this before place_order and show the total to the user.",
                PriceArgs,
                handle_price_order
            ),
            tool!(
                "place_order",
                "Purchase proxies from Proxy-Cheap. This spends real account balance. Run price_order first and get explicit user confirmation.",
                OrderArgs,
                handle_place_order
            ),
            tool!(
                "get_order",
                "Get details for an order by id.",
                OrderIdArgs,
                handle_get_order
            ),
            tool!(
                "get_order_proxies",
                "List the proxies created by a specific order.",
                OrderIdArgs,
                handle_get_order_proxies
            ),
            tool!(
                "list_proxies",
                "List all active proxies on the account, including connection details and bandwidth.",
                EmptyArgs,
                handle_list_proxies
            ),
            tool!(
                "get_proxy",
                "Get details for one proxy by id, including authentication, connection ports, expiry, bandwidth, and auto-extend state.",
                ProxyIdArgs,
                handle_get_proxy
            ),
            tool!(
                "extend_proxy",
                "Extend the paid period of a proxy. Spends account balance.",
                ExtendArgs,
                handle_extend_proxy
            ),
            tool!(
                "get_period_extension_price",
                "Calculate the price of extending a proxy period, without extending it.",
                ExtendPriceArgs,
                handle_period_extension_price
            ),
            tool!(
                "buy_proxy_bandwidth",
                "Buy additional traffic bandwidth (GB) for a proxy. Spends account balance.",
                BandwidthArgs,
                handle_buy_bandwidth
            ),
            tool!(
                "get_bandwidth_price",
                "Calculate the price of additional traffic bandwidth for a proxy, without buying it.",
                BandwidthArgs,
                handle_bandwidth_price
            ),
            tool!(
                "update_proxy_whitelist",
                "Replace the whitelisted IP addresses for a proxy. Changes live access control.",
                WhitelistArgs,
                handle_update_whitelist
            ),
            tool!(
                "get_proxy_protocols",
                "Get the current proxy protocol and the protocols it can switch to.",
                ProxyIdArgs,
                handle_get_protocols
            ),
            tool!(
                "change_proxy_protocol",
                "Change a proxy protocol between HTTP and SOCKS5. Changes live connectivity.",
                ChangeProtocolArgs,
                handle_change_protocol
            ),
            tool!(
                "rotate_proxy_ip",
                "Assign a new public IP to a static mobile proxy. Changes live connectivity.",
                ProxyIdArgs,
                handle_rotate_ip
            ),
            tool!(
                "get_proxy_auth_types",
                "Get the current authentication method and available methods for a proxy.",
                ProxyIdArgs,
                handle_get_auth_types
            ),
            tool!(
                "change_proxy_auth_type",
                "Change a proxy authentication method between username/password and IP whitelist. Changes live connectivity.",
                ChangeAuthArgs,
                handle_change_auth_type
            ),
            tool!(
                "enable_proxy_auto_extend",
                "Enable automatic period extension for a proxy.",
                ProxyIdArgs,
                handle_enable_auto_extend
            ),
            tool!(
                "disable_proxy_auto_extend",
                "Disable automatic period extension for a proxy.",
                ProxyIdArgs,
                handle_disable_auto_extend
            ),
            tool!(
                "get_account_balance",
                "Get the account balance available for orders.",
                EmptyArgs,
                handle_balance
            ),
            tool!(
                "verify_webhook_signature",
                "Verify a Proxy-Cheap webhook delivery. The signature covers event name, event id and the raw body using HMAC-SHA256.",
                VerifyWebhookArgs,
                handle_verify_webhook
            ),
            tool!(
                "legacy_get_order_configuration",
                "Deprecated v1 API: fetch available order options for a configuration. Prefer get_service_options.",
                LegacyOrderRequest,
                handle_legacy_configuration
            ),
            tool!(
                "legacy_price_order",
                "Deprecated v1 API: calculate the price of an order configuration. Prefer price_order.",
                LegacyOrderRequest,
                handle_legacy_price
            ),
            tool!(
                "legacy_place_order",
                "Deprecated v1 API: place an order. Spends real account balance. Prefer place_order.",
                LegacyOrderRequest,
                handle_legacy_execute
            ),
        ]
    })
}

pub fn definitions() -> Vec<Value> {
    specs()
        .iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "description": spec.description,
                "inputSchema": (spec.schema)(),
            })
        })
        .collect()
}

pub async fn call(
    client: Arc<ProxyCheapClient>,
    name: &str,
    arguments: Option<Value>,
) -> std::result::Result<Value, ToolError> {
    let spec = specs()
        .iter()
        .find(|spec| spec.name == name)
        .ok_or_else(|| ToolError::Unknown(name.to_string()))?;
    let arguments = arguments.unwrap_or_else(|| json!({}));
    (spec.call)(client, arguments)
        .await
        .map_err(ToolError::Failed)
}

fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T))
        .unwrap_or_else(|_| json!({ "type": "object", "properties": {} }))
}

fn parse_args<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value)
        .map_err(|error| Error::InvalidInput(format!("invalid tool arguments: {error}")))
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}

// ---------------------------------------------------------------------------
// Tool arguments
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmptyArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceArgs {
    /// Service id from list_services, for example "static-residential-ipv4".
    pub service_id: String,
    /// Plan id for services that have plans.
    #[serde(default)]
    pub plan_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PriceArgs {
    /// Service id from list_services.
    pub service_id: String,
    /// Plan id from get_service_options, for services with plans.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// Package id for package-based services such as the IPv6 plans.
    #[serde(default)]
    pub package_id: Option<String>,
    /// Number of proxies for per-IP services.
    #[serde(default)]
    pub quantity: Option<u32>,
    /// Two-letter ISO country code, for example "US".
    #[serde(default)]
    pub country: Option<String>,
    /// Region id from get_service_options. Static residential IPv4 only.
    #[serde(default)]
    pub region: Option<String>,
    /// City id from get_service_options. Static residential IPv4 only.
    #[serde(default)]
    pub city: Option<String>,
    /// ISP id from get_service_options.
    #[serde(default)]
    pub isp_id: Option<String>,
    /// Billing period. Omit for rotating traffic plans.
    #[serde(default)]
    pub period: Option<Period>,
    /// Traffic in GB for rotating plans.
    #[serde(default)]
    pub traffic: Option<f64>,
    /// Optional coupon code.
    #[serde(default)]
    pub coupon_code: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrderArgs {
    /// Service id from list_services.
    pub service_id: String,
    /// Plan id from get_service_options, for services with plans.
    #[serde(default)]
    pub plan_id: Option<String>,
    /// Package id for package-based services such as the IPv6 plans.
    #[serde(default)]
    pub package_id: Option<String>,
    /// Number of proxies for per-IP services.
    #[serde(default)]
    pub quantity: Option<u32>,
    /// Two-letter ISO country code.
    #[serde(default)]
    pub country: Option<String>,
    /// Region id from get_service_options. Static residential IPv4 only.
    #[serde(default)]
    pub region: Option<String>,
    /// City id from get_service_options. Static residential IPv4 only.
    #[serde(default)]
    pub city: Option<String>,
    /// ISP id from get_service_options.
    #[serde(default)]
    pub isp_id: Option<String>,
    /// Billing period. Omit for rotating traffic plans.
    #[serde(default)]
    pub period: Option<Period>,
    /// Traffic in GB for rotating plans.
    #[serde(default)]
    pub traffic: Option<f64>,
    /// Optional coupon code.
    #[serde(default)]
    pub coupon_code: Option<String>,
    /// Enable auto-extend for services that support it.
    #[serde(default)]
    pub auto_extend: Option<bool>,
    /// Traffic threshold in GB that triggers auto-extend.
    #[serde(default)]
    pub auto_extend_traffic: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrderIdArgs {
    /// Order id returned by place_order.
    pub order_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProxyIdArgs {
    /// Proxy id from list_proxies or get_order_proxies.
    pub proxy_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtendArgs {
    /// Proxy id.
    pub proxy_id: String,
    /// Number of months to add.
    pub months: u32,
    /// Optional coupon code.
    #[serde(default)]
    pub coupon_code: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtendPriceArgs {
    /// Proxy id.
    pub proxy_id: String,
    /// Number of months to price.
    pub months: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BandwidthArgs {
    /// Proxy id.
    pub proxy_id: String,
    /// Additional bandwidth in GB.
    pub bandwidth_gb: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WhitelistArgs {
    /// Proxy id.
    pub proxy_id: String,
    /// Replacement list of whitelisted IPv4 or IPv6 addresses.
    pub ips: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeProtocolArgs {
    /// Proxy id.
    pub proxy_id: String,
    /// Target protocol: HTTP or SOCKS5.
    pub protocol: Protocol,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChangeAuthArgs {
    /// Proxy id.
    pub proxy_id: String,
    /// Target authentication method.
    pub auth_type: AuthType,
    /// IPs to whitelist when switching to IP_WHITELIST.
    #[serde(default)]
    pub ips: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifyWebhookArgs {
    /// Webhook secret configured in the Proxy-Cheap dashboard.
    pub secret: String,
    /// Value of the Webhook-Event header.
    pub event: String,
    /// Value of the Webhook-Id header.
    #[serde(default)]
    pub event_id: Option<String>,
    /// Raw request body exactly as received.
    pub body: String,
    /// Value of the Webhook-Signature header, for example "sha256=...".
    pub signature: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn handle_list_services(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let _: EmptyArgs = parse_args(arguments)?;
    to_value(client.list_services().await?)
}

async fn handle_get_service_options(
    client: Arc<ProxyCheapClient>,
    arguments: Value,
) -> Result<Value> {
    let args: ServiceArgs = parse_args(arguments)?;
    to_value(
        client
            .service_options(&args.service_id, args.plan_id.as_deref())
            .await?,
    )
}

fn price_request(args: &PriceArgs) -> PriceRequest {
    PriceRequest {
        plan_id: args.plan_id.clone(),
        package_id: args.package_id.clone(),
        quantity: args.quantity,
        country: args.country.clone(),
        region: args.region.clone(),
        city: args.city.clone(),
        isp_id: args.isp_id.clone(),
        period: args.period,
        traffic: args.traffic,
        coupon_code: args.coupon_code.clone(),
    }
}

async fn handle_price_order(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: PriceArgs = parse_args(arguments)?;
    to_value(
        client
            .price(&args.service_id, &price_request(&args))
            .await?,
    )
}

async fn handle_place_order(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: OrderArgs = parse_args(arguments)?;
    let request = OrderRequest {
        price: PriceRequest {
            plan_id: args.plan_id,
            package_id: args.package_id,
            quantity: args.quantity,
            country: args.country,
            region: args.region,
            city: args.city,
            isp_id: args.isp_id,
            period: args.period,
            traffic: args.traffic,
            coupon_code: args.coupon_code,
        },
        auto_extend: args.auto_extend.map(|enabled| AutoExtend {
            is_enabled: enabled,
            traffic: args.auto_extend_traffic,
        }),
    };
    to_value(client.place_order(&args.service_id, &request).await?)
}

async fn handle_get_order(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: OrderIdArgs = parse_args(arguments)?;
    to_value(client.get_order(&args.order_id).await?)
}

async fn handle_get_order_proxies(
    client: Arc<ProxyCheapClient>,
    arguments: Value,
) -> Result<Value> {
    let args: OrderIdArgs = parse_args(arguments)?;
    to_value(client.order_proxies(&args.order_id).await?)
}

async fn handle_list_proxies(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let _: EmptyArgs = parse_args(arguments)?;
    to_value(client.list_proxies().await?)
}

async fn handle_get_proxy(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: ProxyIdArgs = parse_args(arguments)?;
    to_value(client.get_proxy(&args.proxy_id).await?)
}

async fn handle_extend_proxy(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: ExtendArgs = parse_args(arguments)?;
    let request = ExtendPeriodRequest {
        period_in_months: args.months,
        coupon_code: args.coupon_code,
    };
    to_value(client.extend_proxy(&args.proxy_id, &request).await?)
}

async fn handle_period_extension_price(
    client: Arc<ProxyCheapClient>,
    arguments: Value,
) -> Result<Value> {
    let args: ExtendPriceArgs = parse_args(arguments)?;
    to_value(
        client
            .period_extension_price(&args.proxy_id, args.months)
            .await?,
    )
}

async fn handle_buy_bandwidth(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: BandwidthArgs = parse_args(arguments)?;
    to_value(
        client
            .buy_bandwidth(&args.proxy_id, args.bandwidth_gb)
            .await?,
    )
}

async fn handle_bandwidth_price(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: BandwidthArgs = parse_args(arguments)?;
    to_value(
        client
            .bandwidth_price(&args.proxy_id, args.bandwidth_gb)
            .await?,
    )
}

async fn handle_update_whitelist(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: WhitelistArgs = parse_args(arguments)?;
    to_value(client.update_whitelist(&args.proxy_id, &args.ips).await?)
}

async fn handle_get_protocols(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: ProxyIdArgs = parse_args(arguments)?;
    to_value(client.protocol_options(&args.proxy_id).await?)
}

async fn handle_change_protocol(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: ChangeProtocolArgs = parse_args(arguments)?;
    client
        .change_protocol(&args.proxy_id, args.protocol)
        .await?;
    Ok(json!({ "ok": true }))
}

async fn handle_rotate_ip(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: ProxyIdArgs = parse_args(arguments)?;
    client.rotate_ip(&args.proxy_id).await?;
    Ok(json!({ "ok": true }))
}

async fn handle_get_auth_types(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: ProxyIdArgs = parse_args(arguments)?;
    to_value(client.auth_type_options(&args.proxy_id).await?)
}

async fn handle_change_auth_type(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: ChangeAuthArgs = parse_args(arguments)?;
    client
        .change_auth_type(&args.proxy_id, args.auth_type, args.ips.unwrap_or_default())
        .await?;
    Ok(json!({ "ok": true }))
}

async fn handle_enable_auto_extend(
    client: Arc<ProxyCheapClient>,
    arguments: Value,
) -> Result<Value> {
    let args: ProxyIdArgs = parse_args(arguments)?;
    client.set_auto_extend(&args.proxy_id, true).await?;
    Ok(json!({ "ok": true }))
}

async fn handle_disable_auto_extend(
    client: Arc<ProxyCheapClient>,
    arguments: Value,
) -> Result<Value> {
    let args: ProxyIdArgs = parse_args(arguments)?;
    client.set_auto_extend(&args.proxy_id, false).await?;
    Ok(json!({ "ok": true }))
}

async fn handle_balance(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let _: EmptyArgs = parse_args(arguments)?;
    to_value(client.balance().await?)
}

async fn handle_verify_webhook(_client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let args: VerifyWebhookArgs = parse_args(arguments)?;
    let valid = webhook::verify(
        &args.secret,
        &args.event,
        args.event_id.as_deref().unwrap_or(""),
        args.body.as_bytes(),
        &args.signature,
    );
    Ok(json!({ "valid": valid }))
}

async fn handle_legacy_configuration(
    client: Arc<ProxyCheapClient>,
    arguments: Value,
) -> Result<Value> {
    let request: LegacyOrderRequest = parse_args(arguments)?;
    to_value(client.legacy_configuration(&request).await?)
}

async fn handle_legacy_price(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let request: LegacyOrderRequest = parse_args(arguments)?;
    to_value(client.legacy_price(&request).await?)
}

async fn handle_legacy_execute(client: Arc<ProxyCheapClient>, arguments: Value) -> Result<Value> {
    let request: LegacyOrderRequest = parse_args(arguments)?;
    to_value(client.legacy_execute(&request).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_every_api_operation() {
        let names: Vec<&str> = specs().iter().map(|spec| spec.name).collect();
        assert_eq!(names.len(), 25, "expected 25 tools, got {names:?}");
        for required in [
            "list_services",
            "get_service_options",
            "price_order",
            "place_order",
            "get_order",
            "get_order_proxies",
            "list_proxies",
            "get_proxy",
            "extend_proxy",
            "get_period_extension_price",
            "buy_proxy_bandwidth",
            "get_bandwidth_price",
            "update_proxy_whitelist",
            "get_proxy_protocols",
            "change_proxy_protocol",
            "rotate_proxy_ip",
            "get_proxy_auth_types",
            "change_proxy_auth_type",
            "enable_proxy_auto_extend",
            "disable_proxy_auto_extend",
            "get_account_balance",
            "legacy_get_order_configuration",
            "legacy_price_order",
            "legacy_place_order",
            "verify_webhook_signature",
        ] {
            assert!(names.contains(&required), "missing tool {required}");
        }
    }

    #[test]
    fn schemas_are_objects_with_documented_fields() {
        let definitions = definitions();
        let price = definitions
            .iter()
            .find(|definition| definition["name"] == "price_order")
            .unwrap();
        let schema = &price["inputSchema"];
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["serviceId"].is_object());
        assert!(schema["properties"]["country"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&Value::String("serviceId".to_string()))
        );
    }

    #[test]
    fn rejects_unknown_arguments() {
        let result: Result<PriceArgs> = parse_args(json!({"serviceId":"x","nonsense":true}));
        assert!(result.is_err());
    }
}
