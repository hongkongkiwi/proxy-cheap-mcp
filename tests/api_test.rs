mod common;

use proxy_cheap_mcp::api::{ExtendPeriodRequest, Period, PeriodUnit, PriceRequest, Protocol};
use proxy_cheap_mcp::config::ClientConfig;
use proxy_cheap_mcp::{Error, ProxyCheapClient};

fn client(base_url: &str, credentials: bool, retries: u32) -> ProxyCheapClient {
    let config = ClientConfig {
        api_key: credentials.then(|| "key".to_string()),
        api_secret: credentials.then(|| "secret".to_string()),
        base_url: base_url.to_string(),
        timeout_secs: 5,
        max_retries: retries,
    };
    ProxyCheapClient::new(&config).unwrap()
}

#[tokio::test]
async fn public_endpoints_work_without_credentials() {
    let mock = common::start().await;
    let client = client(mock.base_url(), false, 0);

    let services = client.list_services().await.unwrap();
    assert_eq!(services.services.len(), 2);
    assert_eq!(services.services[0].id, "static-datacenter-ipv4");

    let captured = mock.last();
    assert_eq!(captured.method, "GET");
    assert_eq!(captured.path, "/v2/order");
    assert!(
        captured
            .header("accept")
            .unwrap()
            .contains("application/json")
    );
    assert!(captured.header("x-api-key").is_none());
}

#[tokio::test]
async fn sends_credentials_on_authenticated_calls() {
    let mock = common::start().await;
    let client = client(mock.base_url(), true, 0);

    let proxies = client.list_proxies().await.unwrap();
    assert_eq!(proxies.len(), 2);
    assert_eq!(proxies[0].id, "1410379");
    assert_eq!(
        proxies[0].connection.as_ref().unwrap().http_port,
        Some(8000)
    );

    let captured = mock.last();
    assert_eq!(captured.header("x-api-key").as_deref(), Some("key"));
    assert_eq!(captured.header("x-api-secret").as_deref(), Some("secret"));
}

#[tokio::test]
async fn setup_request_sends_only_the_plan() {
    let mock = common::start().await;
    let client = client(mock.base_url(), false, 0);

    let options = client
        .service_options("static-datacenter-ipv4", Some("basic"))
        .await
        .unwrap();
    assert_eq!(options.service_id, "static-datacenter-ipv4");
    assert_eq!(options.periods.unwrap().months, vec![1, 12]);

    let captured = mock.last();
    assert_eq!(captured.method, "POST");
    assert_eq!(captured.path, "/v2/order/static-datacenter-ipv4");
    assert_eq!(captured.json(), serde_json::json!({"planId": "basic"}));
}

#[tokio::test]
async fn prices_a_full_configuration() {
    let mock = common::start().await;
    let client = client(mock.base_url(), false, 0);

    let request = PriceRequest {
        plan_id: Some("basic".to_string()),
        quantity: Some(1),
        country: Some("US".to_string()),
        period: Some(Period {
            unit: PeriodUnit::Months,
            value: 1,
        }),
        ..Default::default()
    };
    let price = client
        .price("static-datacenter-ipv4", &request)
        .await
        .unwrap();
    assert_eq!(price.final_price, Some(3.39));
    assert_eq!(price.currency.as_deref(), Some("USD"));

    let captured = mock.last();
    let body = captured.json();
    assert_eq!(body["planId"], "basic");
    assert_eq!(body["quantity"], 1);
    assert_eq!(body["country"], "US");
    assert_eq!(
        body["period"],
        serde_json::json!({"unit": "months", "value": 1})
    );
    assert!(body.get("traffic").is_none());
    assert!(body.get("couponCode").is_none());
}

#[tokio::test]
async fn places_an_order_and_parses_numeric_strings() {
    let mock = common::start().await;
    let client = client(mock.base_url(), true, 0);

    let request = proxy_cheap_mcp::api::OrderRequest {
        price: PriceRequest {
            plan_id: Some("basic".to_string()),
            country: Some("US".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    let order = client
        .place_order("static-datacenter-ipv4", &request)
        .await
        .unwrap();
    assert_eq!(order.total_price, Some(3.39));
    assert_eq!(order.period_in_months, Some(1));
}

#[tokio::test]
async fn maps_validation_errors() {
    let mock = common::start().await;
    let client = client(mock.base_url(), true, 0);

    let error = client
        .extend_proxy(
            "bad",
            &ExtendPeriodRequest {
                period_in_months: 0,
                coupon_code: None,
            },
        )
        .await
        .unwrap_err();
    match &error {
        Error::Api { status, .. } => assert_eq!(*status, 422),
        other => panic!("unexpected error: {other}"),
    }
    assert!(error.to_string().contains("periodInMonths: NOT_ALLOWED"));
    assert_eq!(error.exit_code(), 4);
}

#[tokio::test]
async fn maps_unauthorized_responses() {
    let mock = common::start().await;
    let client = client(mock.base_url(), true, 0);

    let error = client.balance().await.unwrap_err();
    assert!(error.to_string().contains("INVALID_CREDENTIALS"));
    assert_eq!(error.exit_code(), 3);
}

#[tokio::test]
async fn explains_login_redirects_as_auth_failures() {
    let mock = common::start().await;
    let config = ClientConfig {
        api_key: Some("wrong".to_string()),
        api_secret: Some("wrong".to_string()),
        base_url: mock.base_url().to_string(),
        timeout_secs: 5,
        max_retries: 0,
    };
    let client = ProxyCheapClient::new(&config).unwrap();

    let error = client.list_proxies().await.unwrap_err();
    assert!(error.to_string().contains("redirected"));
    assert_eq!(error.exit_code(), 3);
}

#[tokio::test]
async fn retries_idempotent_gets_on_server_errors() {
    let mock = common::start_flaky().await;
    let client = client(mock.base_url(), false, 2);

    let services = client.list_services().await.unwrap();
    assert_eq!(services.services.len(), 2);
    assert_eq!(mock.request_count(), 3);
}

#[tokio::test]
async fn does_not_retry_posts_that_may_have_spent_money() {
    let mock = common::start().await;
    let client = client(mock.base_url(), false, 2);

    let error = client
        .price(
            "error-service",
            &PriceRequest {
                plan_id: Some("basic".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Api { status: 500, .. }));
    assert_eq!(mock.request_count(), 1);
}

#[tokio::test]
async fn refuses_authenticated_calls_without_credentials() {
    let mock = common::start().await;
    let client = client(mock.base_url(), false, 0);

    let error = client.list_proxies().await.unwrap_err();
    assert!(matches!(error, Error::MissingCredentials));
    assert_eq!(mock.request_count(), 0);
}

#[tokio::test]
async fn handles_empty_success_responses() {
    let mock = common::start().await;
    let client = client(mock.base_url(), true, 0);

    client
        .change_protocol("1410379", Protocol::Socks5)
        .await
        .unwrap();
    let captured = mock.last();
    assert_eq!(captured.path, "/proxies/1410379/change-protocol");
    assert_eq!(captured.json(), serde_json::json!({"newType": "SOCKS5"}));
}

#[tokio::test]
async fn reports_non_json_success_bodies() {
    let mock = common::start().await;
    let client = client(mock.base_url(), true, 0);

    let error = client
        .extend_proxy(
            "html",
            &ExtendPeriodRequest {
                period_in_months: 1,
                coupon_code: None,
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("could not parse"));
}

#[tokio::test]
async fn covers_order_and_legacy_endpoints() {
    let mock = common::start().await;
    let client = client(mock.base_url(), true, 0);

    let order = client.get_order("123").await.unwrap();
    assert_eq!(order.id, "123");
    assert_eq!(order.period_in_months, Some(1));

    let proxies = client.order_proxies("123").await.unwrap();
    assert_eq!(proxies.len(), 2);

    let configuration = client
        .legacy_configuration(&Default::default())
        .await
        .unwrap();
    assert_eq!(configuration.supported_protocols, vec!["HTTP", "SOCKS5"]);

    let quote = client.legacy_price(&Default::default()).await.unwrap();
    assert_eq!(quote.final_price, Some(3.39));

    let order = client.legacy_execute(&Default::default()).await.unwrap();
    assert_eq!(order.id, "order-1");
}
