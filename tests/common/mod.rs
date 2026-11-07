#![allow(dead_code)]

//! Shared mock Proxy-Cheap API used by the integration tests.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;

#[derive(Clone, Debug)]
pub struct Captured {
    pub method: String,
    pub path: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Captured {
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

struct Inner {
    requests: Mutex<Vec<Captured>>,
    failures: AtomicU32,
    flaky_services: bool,
}

pub struct MockApi {
    base_url: String,
    inner: Arc<Inner>,
}

impl MockApi {
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn requests(&self) -> Vec<Captured> {
        self.inner.requests.lock().unwrap().clone()
    }

    pub fn request_count(&self) -> usize {
        self.inner.requests.lock().unwrap().len()
    }

    pub fn last(&self) -> Captured {
        self.requests()
            .last()
            .cloned()
            .expect("no request captured")
    }
}

pub async fn start() -> MockApi {
    start_with(false).await
}

/// The first two `GET /v2/order` calls return 500, so a retrying client
/// succeeds on the third attempt.
pub async fn start_flaky() -> MockApi {
    start_with(true).await
}

async fn start_with(flaky_services: bool) -> MockApi {
    let inner = Arc::new(Inner {
        requests: Mutex::new(Vec::new()),
        failures: AtomicU32::new(0),
        flaky_services,
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().fallback(handle).with_state(inner.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    MockApi {
        base_url: format!("http://{address}"),
        inner,
    }
}

async fn handle(
    State(state): State<Arc<Inner>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    state.requests.lock().unwrap().push(Captured {
        method: method.to_string(),
        path: uri.path().to_string(),
        headers: headers.clone(),
        body: body.to_vec(),
    });

    let path = uri.path();
    let authorized = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        == Some("key");

    match (method.as_str(), path) {
        ("GET", "/v2/order") => {
            if state.flaky_services {
                let failures = state.failures.fetch_add(1, Ordering::SeqCst);
                if failures < 2 {
                    return (StatusCode::INTERNAL_SERVER_ERROR, "temporary").into_response();
                }
            }
            Json(json!({
                "services": [
                    {
                        "id": "static-datacenter-ipv4",
                        "label": "Static Datacenter (IPv4)",
                        "plans": [
                            {"id": "basic", "label": "Basic"},
                            {"id": "premium", "label": "Premium"}
                        ]
                    },
                    {"id": "rotating-residential", "label": "Rotating Residential"}
                ]
            }))
            .into_response()
        }
        ("POST", "/v2/order/static-datacenter-ipv4") => Json(json!({
            "serviceId": "static-datacenter-ipv4",
            "plans": [{"id": "basic", "name": "Basic"}],
            "countries": ["US", "DE"],
            "periods": {"months": [1, 12]}
        }))
        .into_response(),
        ("POST", "/v2/order/static-datacenter-ipv4/price") => Json(json!({
            "appliedDiscounts": [],
            "finalPrice": 3.39,
            "priceNoDiscounts": 3.39,
            "discount": 0,
            "unitPrice": 3.39,
            "unitPriceAfterDiscount": 3.39,
            "additionalAmount": 0,
            "additionalAmountAfterDiscount": 0,
            "paymentFee": 0,
            "subtotal": 3.39,
            "discountAmount": 0,
            "finalPriceInCurrency": 3.39,
            "currency": "USD"
        }))
        .into_response(),
        ("POST", "/v2/order/error-service/price") => {
            (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response()
        }
        ("POST", "/v2/order/static-datacenter-ipv4/execute") => Json(json!({
            "id": "00000000-0000-0000-0000-000000000000",
            "periodInMonths": "1",
            "bandwidth": "1",
            "totalPrice": "3.39"
        }))
        .into_response(),
        ("GET", "/orders/123") => Json(json!({
            "id": "123",
            "periodInMoths": 1,
            "bandwidth": 1,
            "totalPrice": 3.39
        }))
        .into_response(),
        ("GET", "/orders/123/proxies") => Json(json!([
            proxy_json("1410379", "ACTIVE"),
            proxy_json("1410380", "PENDING")
        ]))
        .into_response(),
        ("GET", "/proxies") => {
            if authorized {
                Json(json!([
                    proxy_json("1410379", "ACTIVE"),
                    proxy_json("1410380", "PENDING")
                ]))
                .into_response()
            } else {
                (
                    StatusCode::FOUND,
                    [(header::LOCATION, "https://app.proxy-cheap.com/login")],
                )
                    .into_response()
            }
        }
        ("GET", "/account/balance") => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message": "INVALID_CREDENTIALS"})),
        )
            .into_response(),
        ("GET", "/health") => Json(json!({"status": "ok"})).into_response(),
        _ if path.starts_with("/proxies/") => proxy_action(method.as_str(), path).into_response(),
        _ if path == "/order/configuration" => Json(json!({
            "supportedNetworkTypes": ["RESIDENTIAL_STATIC"],
            "supportedCountries": ["US"],
            "supportedRegions": ["New York"],
            "supportedAuthenticationTypes": ["USERNAME_PASSWORD"],
            "supportedIpWhitelistCount": 5,
            "supportedIpVersions": ["IPv4"],
            "supportedProtocols": ["HTTP", "SOCKS5"],
            "supportedISPs": ["isp-1"],
            "supportedPackages": [],
            "supportedQuantity": {"min": 1, "max": 100},
            "supportsAddingBandwidth": true,
            "supportsAutoExtending": true
        }))
        .into_response(),
        _ if path == "/order/price" => Json(json!({
            "finalPrice": 3.39,
            "priceNoDiscounts": 3.39,
            "discount": 0
        }))
        .into_response(),
        _ if path == "/order/execute" => Json(json!({
            "id": "order-1",
            "periodInMoths": 1,
            "bandwidth": 1,
            "totalPrice": 3.39
        }))
        .into_response(),
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({"message": format!("no mock for {method} {path}")})),
        )
            .into_response(),
    }
}

fn proxy_action(method: &str, path: &str) -> Response {
    if path.ends_with("/extend-period") && method == "POST" {
        if path.contains("/proxies/bad/") {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "errors": {
                        "periodInMonths": [{
                            "code": "NOT_ALLOWED",
                            "parameters": {"value": "0"}
                        }]
                    }
                })),
            )
                .into_response();
        }
        if path.contains("/proxies/html/") {
            return (StatusCode::OK, "<html>not json</html>").into_response();
        }
        return Json(proxy_json("1410379", "ACTIVE")).into_response();
    }
    if path.ends_with("/device") {
        return (StatusCode::NOT_FOUND, "nope").into_response();
    }
    match (method, path) {
        ("GET", p) if p.ends_with("/change-protocol") => Json(json!({
            "currentType": "HTTP",
            "availableTypes": ["HTTP", "SOCKS5"]
        }))
        .into_response(),
        ("POST", p) if p.ends_with("/change-protocol") => {
            (StatusCode::CREATED, Json(json!({}))).into_response()
        }
        ("POST", p) if p.ends_with("/rotate-ip") => StatusCode::NO_CONTENT.into_response(),
        ("POST", p)
            if p.ends_with("/auto-extend/enable") || p.ends_with("/auto-extend/disable") =>
        {
            StatusCode::NO_CONTENT.into_response()
        }
        ("GET", p) if p.ends_with("/change-authentication-type") => Json(json!({
            "currentAuthenticationType": "USERNAME_PASSWORD",
            "availableAuthenticationTypes": ["IP_WHITELIST", "USERNAME_PASSWORD"]
        }))
        .into_response(),
        ("POST", p) if p.ends_with("/change-authentication-type") => {
            (StatusCode::CREATED, Json(json!({}))).into_response()
        }
        ("POST", p) if p.ends_with("/whitelist-ip") => {
            Json(proxy_json("1410379", "ACTIVE")).into_response()
        }
        ("POST", p) if p.ends_with("/buy-bandwidth") => {
            Json(proxy_json("1410379", "ACTIVE")).into_response()
        }
        ("POST", p)
            if p.ends_with("/bandwidth-price") || p.ends_with("/period-extension-price") =>
        {
            Json(json!({"finalPrice": 1.5, "priceNoDiscounts": 1.5, "discount": 0})).into_response()
        }
        ("GET", p) if p.starts_with("/proxies/") && p.matches('/').count() == 2 => {
            Json(proxy_json("1410379", "ACTIVE")).into_response()
        }
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({"message": "unknown proxy action"})),
        )
            .into_response(),
    }
}

fn proxy_json(id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "status": status,
        "networkType": "RESIDENTIAL_STATIC",
        "countryCode": "US",
        "authentication": {
            "whitelistedIps": ["203.0.113.7"],
            "username": "user",
            "password": "pass"
        },
        "connection": {
            "publicIp": "203.0.113.10",
            "connectIp": "gate.proxy-cheap.com",
            "ipVersion": "IPv4",
            "httpPort": 8000,
            "httpsPort": 8001,
            "socks5Port": 8002
        },
        "proxyType": "HTTP",
        "createdAt": "2026-01-01T00:00:00+00:00",
        "expiresAt": "2026-02-01T00:00:00+00:00",
        "metadata": {"ispName": "Example ISP"},
        "autoExtendEnabled": true,
        "bandwidth": {"total": 5, "used": 1}
    })
}
