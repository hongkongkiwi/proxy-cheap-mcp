//! Proxy-Cheap REST API client.

pub mod types;

pub use types::*;

use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderValue, RETRY_AFTER};
use reqwest::{Client, Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::ClientConfig;
use crate::error::{Error, Result};

#[derive(Debug, Clone)]
struct Credentials {
    key: String,
    secret: String,
}

#[derive(Debug, Clone)]
pub struct ProxyCheapClient {
    http: Client,
    base_url: String,
    credentials: Option<Credentials>,
    max_retries: u32,
}

impl ProxyCheapClient {
    pub fn new(config: &ClientConfig) -> Result<Self> {
        let credentials = match (&config.api_key, &config.api_secret) {
            (Some(key), Some(secret)) if !key.is_empty() && !secret.is_empty() => {
                Some(Credentials {
                    key: key.clone(),
                    secret: secret.clone(),
                })
            }
            _ => None,
        };

        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(format!(
                "proxy-cheap-mcp/{} (+https://github.com/hongkongkiwi/proxy-cheap-mcp)",
                env!("CARGO_PKG_VERSION")
            ))
            .build()?;

        Ok(Self {
            http,
            base_url: config.base_url.clone(),
            credentials,
            max_retries: config.max_retries,
        })
    }

    pub fn has_credentials(&self) -> bool {
        self.credentials.is_some()
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    // -- Services and setup -------------------------------------------------

    /// List orderable services and their plans. Public endpoint.
    pub async fn list_services(&self) -> Result<ServicesResponse> {
        self.get_json("/v2/order", false).await
    }

    /// Fetch the order options for a service. Public endpoint.
    pub async fn service_options(
        &self,
        service_id: &str,
        plan_id: Option<&str>,
    ) -> Result<ServiceOptions> {
        let path = format!("/v2/order/{}", encode_segment(service_id));
        let body = match plan_id {
            Some(plan_id) if !plan_id.is_empty() => serde_json::json!({ "planId": plan_id }),
            _ => serde_json::json!({}),
        };
        self.post_json(&path, &body, false).await
    }

    /// Price an order configuration. Public endpoint.
    pub async fn price(&self, service_id: &str, request: &PriceRequest) -> Result<PriceResponse> {
        let path = format!("/v2/order/{}/price", encode_segment(service_id));
        self.post_json(&path, request, false).await
    }

    /// Place an order. Spends account balance.
    pub async fn place_order(
        &self,
        service_id: &str,
        request: &OrderRequest,
    ) -> Result<OrderResponse> {
        let path = format!("/v2/order/{}/execute", encode_segment(service_id));
        self.post_json(&path, request, true).await
    }

    // -- Orders -------------------------------------------------------------

    pub async fn get_order(&self, order_id: &str) -> Result<OrderResponse> {
        let path = format!("/orders/{}", encode_segment(order_id));
        self.get_json(&path, true).await
    }

    pub async fn order_proxies(&self, order_id: &str) -> Result<Vec<Proxy>> {
        let path = format!("/orders/{}/proxies", encode_segment(order_id));
        self.get_json(&path, true).await
    }

    // -- Proxies ------------------------------------------------------------

    pub async fn list_proxies(&self) -> Result<Vec<Proxy>> {
        self.get_json("/proxies", true).await
    }

    pub async fn get_proxy(&self, proxy_id: &str) -> Result<Proxy> {
        let path = format!("/proxies/{}", encode_segment(proxy_id));
        self.get_json(&path, true).await
    }

    pub async fn extend_proxy(
        &self,
        proxy_id: &str,
        request: &ExtendPeriodRequest,
    ) -> Result<Proxy> {
        let path = format!("/proxies/{}/extend-period", encode_segment(proxy_id));
        self.post_json(&path, request, true).await
    }

    pub async fn period_extension_price(&self, proxy_id: &str, months: u32) -> Result<PriceQuote> {
        let path = format!(
            "/proxies/{}/period-extension-price",
            encode_segment(proxy_id)
        );
        self.post_json(
            &path,
            &serde_json::json!({ "periodInMonths": months }),
            true,
        )
        .await
    }

    pub async fn buy_bandwidth(&self, proxy_id: &str, bandwidth_gb: u32) -> Result<Proxy> {
        let path = format!("/proxies/{}/buy-bandwidth", encode_segment(proxy_id));
        self.post_json(
            &path,
            &BandwidthRequest {
                bandwidth: bandwidth_gb,
            },
            true,
        )
        .await
    }

    pub async fn bandwidth_price(&self, proxy_id: &str, bandwidth_gb: u32) -> Result<PriceQuote> {
        let path = format!("/proxies/{}/bandwidth-price", encode_segment(proxy_id));
        self.post_json(
            &path,
            &BandwidthRequest {
                bandwidth: bandwidth_gb,
            },
            true,
        )
        .await
    }

    pub async fn update_whitelist(&self, proxy_id: &str, ips: &[String]) -> Result<Proxy> {
        let path = format!("/proxies/{}/whitelist-ip", encode_segment(proxy_id));
        let request = WhitelistRequest { ips: ips.to_vec() };
        self.post_json(&path, &request, true).await
    }

    pub async fn protocol_options(&self, proxy_id: &str) -> Result<ProtocolOptions> {
        let path = format!("/proxies/{}/change-protocol", encode_segment(proxy_id));
        self.get_json(&path, true).await
    }

    pub async fn change_protocol(&self, proxy_id: &str, protocol: Protocol) -> Result<()> {
        let path = format!("/proxies/{}/change-protocol", encode_segment(proxy_id));
        let request = ChangeProtocolRequest { new_type: protocol };
        self.post_empty(&path, &request, true).await
    }

    /// Assigns a new public IP. Static mobile proxies only.
    pub async fn rotate_ip(&self, proxy_id: &str) -> Result<()> {
        let path = format!("/proxies/{}/rotate-ip", encode_segment(proxy_id));
        self.post_empty(&path, &serde_json::json!({}), true).await
    }

    pub async fn auth_type_options(&self, proxy_id: &str) -> Result<AuthTypeOptions> {
        let path = format!(
            "/proxies/{}/change-authentication-type",
            encode_segment(proxy_id)
        );
        self.get_json(&path, true).await
    }

    pub async fn change_auth_type(
        &self,
        proxy_id: &str,
        auth_type: AuthType,
        ips: Vec<String>,
    ) -> Result<()> {
        let path = format!(
            "/proxies/{}/change-authentication-type",
            encode_segment(proxy_id)
        );
        let request = ChangeAuthTypeRequest {
            new_auth_type: auth_type,
            ips,
        };
        self.post_empty(&path, &request, true).await
    }

    pub async fn set_auto_extend(&self, proxy_id: &str, enabled: bool) -> Result<()> {
        let action = if enabled { "enable" } else { "disable" };
        let path = format!(
            "/proxies/{}/auto-extend/{}",
            encode_segment(proxy_id),
            action
        );
        self.post_empty(&path, &serde_json::json!({}), true).await
    }

    // -- Account ------------------------------------------------------------

    pub async fn balance(&self) -> Result<Balance> {
        self.get_json("/account/balance", true).await
    }

    // -- Deprecated v1 ordering API ----------------------------------------

    pub async fn legacy_configuration(
        &self,
        request: &LegacyOrderRequest,
    ) -> Result<LegacyConfigurationResponse> {
        self.post_json("/order/configuration", request, false).await
    }

    pub async fn legacy_price(&self, request: &LegacyOrderRequest) -> Result<PriceQuote> {
        self.post_json("/order/price", request, false).await
    }

    pub async fn legacy_execute(&self, request: &LegacyOrderRequest) -> Result<OrderResponse> {
        self.post_json("/order/execute", request, true).await
    }

    // -- Transport ----------------------------------------------------------

    async fn get_json<T: DeserializeOwned>(&self, path: &str, auth_required: bool) -> Result<T> {
        let (status, body) = self.send(Method::GET, path, None, auth_required).await?;
        parse_json(status, &body)
    }

    async fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &(impl Serialize + ?Sized),
        auth_required: bool,
    ) -> Result<T> {
        let body = serde_json::to_vec(body)?;
        let (status, bytes) = self
            .send(Method::POST, path, Some(body), auth_required)
            .await?;
        parse_json(status, &bytes)
    }

    async fn post_empty(
        &self,
        path: &str,
        body: &(impl Serialize + ?Sized),
        auth_required: bool,
    ) -> Result<()> {
        let body = serde_json::to_vec(body)?;
        let (status, bytes) = self
            .send(Method::POST, path, Some(body), auth_required)
            .await?;
        if !bytes.is_empty() && serde_json::from_slice::<serde_json::Value>(&bytes).is_err() {
            return Err(Error::from_response(status.as_u16(), &bytes));
        }
        Ok(())
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        auth_required: bool,
    ) -> Result<(StatusCode, Vec<u8>)> {
        if auth_required && self.credentials.is_none() {
            return Err(Error::MissingCredentials);
        }

        let url = format!("{}{}", self.base_url, path);
        let idempotent = method == Method::GET || method == Method::HEAD;
        let mut attempt = 0u32;

        loop {
            let mut request = self
                .http
                .request(method.clone(), &url)
                .header(ACCEPT, HeaderValue::from_static("application/json"));

            if let Some(credentials) = &self.credentials {
                request = request
                    .header("X-Api-Key", &credentials.key)
                    .header("X-Api-Secret", &credentials.secret);
            }
            if let Some(body) = &body {
                request = request
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .body(body.clone());
            }

            let retryable_status = |status: StatusCode| {
                status == StatusCode::TOO_MANY_REQUESTS || (idempotent && status.is_server_error())
            };

            match request.send().await {
                Ok(response) if response.status().is_success() => {
                    let status = response.status();
                    let bytes = response.bytes().await?;
                    return Ok((status, bytes.to_vec()));
                }
                Ok(response) => {
                    let status = response.status();
                    let retry_after = response
                        .headers()
                        .get(RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.trim().parse::<u64>().ok())
                        .map(Duration::from_secs);
                    let bytes = response.bytes().await?;
                    if retryable_status(status) && attempt < self.max_retries {
                        attempt += 1;
                        tokio::time::sleep(retry_after.unwrap_or_else(|| backoff(attempt))).await;
                        continue;
                    }
                    return Err(Error::from_response(status.as_u16(), &bytes));
                }
                Err(error)
                    if (error.is_timeout() || error.is_connect())
                        && idempotent
                        && attempt < self.max_retries =>
                {
                    attempt += 1;
                    tokio::time::sleep(backoff(attempt)).await;
                    continue;
                }
                Err(error) => return Err(Error::Network(error)),
            }
        }
    }
}

fn parse_json<T: DeserializeOwned>(status: StatusCode, body: &[u8]) -> Result<T> {
    if body.is_empty() {
        return Err(Error::Api {
            status: status.as_u16(),
            message: "the API returned an empty body".to_string(),
            body: None,
        });
    }
    serde_json::from_slice(body).map_err(|error| Error::Api {
        status: status.as_u16(),
        message: format!("could not parse the API response as JSON: {error}"),
        body: serde_json::from_slice(body).ok(),
    })
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(250u64.saturating_mul(2u64.saturating_pow(attempt.min(6))))
}

fn encode_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            other => {
                use std::fmt::Write as _;
                let _ = write!(encoded, "%{other:02X}");
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_path_segments() {
        assert_eq!(
            encode_segment("static-residential-ipv4"),
            "static-residential-ipv4"
        );
        assert_eq!(encode_segment("a/b c"), "a%2Fb%20c");
    }

    #[test]
    fn backoff_grows() {
        assert!(backoff(1) < backoff(3));
    }
}
