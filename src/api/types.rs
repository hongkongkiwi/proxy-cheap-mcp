//! Wire types for the Proxy-Cheap REST API.
//!
//! The API mixes numbers, numeric strings, snake_case identifiers, and
//! per-service response shapes. Deserialisers here are deliberately lenient so
//! that unknown fields do not break a command and numeric values encoded as
//! strings (for example `"3.39"`) are accepted.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};

mod flexible {
    use std::fmt;

    use serde::de::{self, Deserializer, Visitor};

    struct F64Visitor;

    impl<'de> Visitor<'de> for F64Visitor {
        type Value = f64;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a number or a numeric string")
        }

        fn visit_f64<E: de::Error>(self, value: f64) -> Result<f64, E> {
            Ok(value)
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<f64, E> {
            Ok(value as f64)
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<f64, E> {
            Ok(value as f64)
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<f64, E> {
            value.trim().parse().map_err(E::custom)
        }

        fn visit_string<E: de::Error>(self, value: String) -> Result<f64, E> {
            self.visit_str(&value)
        }
    }

    fn number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        deserializer.deserialize_any(F64Visitor)
    }

    struct OptF64Visitor;

    impl<'de> Visitor<'de> for OptF64Visitor {
        type Value = Option<f64>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a number, a numeric string, or null")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D: Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            number(deserializer).map(Some)
        }
    }

    pub fn opt_f64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
        deserializer.deserialize_option(OptF64Visitor)
    }

    struct U32Visitor;

    impl<'de> Visitor<'de> for U32Visitor {
        type Value = u32;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a non-negative integer or numeric string")
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<u32, E> {
            u32::try_from(value).map_err(|_| E::custom("integer out of range"))
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<u32, E> {
            u32::try_from(value).map_err(|_| E::custom("integer out of range"))
        }

        fn visit_f64<E: de::Error>(self, value: f64) -> Result<u32, E> {
            if value.fract() == 0.0 && value >= 0.0 {
                self.visit_u64(value as u64)
            } else {
                Err(E::custom("expected a whole number"))
            }
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<u32, E> {
            value.trim().parse().map_err(E::custom)
        }
    }

    struct OptU32Visitor;

    impl<'de> Visitor<'de> for OptU32Visitor {
        type Value = Option<u32>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a non-negative integer, numeric string, or null")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D: Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_any(U32Visitor).map(Some)
        }
    }

    pub fn opt_u32<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
        deserializer.deserialize_option(OptU32Visitor)
    }

    struct IdVisitor;

    impl<'de> Visitor<'de> for IdVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a string or numeric identifier")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
            Ok(value.to_string())
        }

        fn visit_string<E: de::Error>(self, value: String) -> Result<String, E> {
            Ok(value)
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<String, E> {
            Ok(value.to_string())
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<String, E> {
            Ok(value.to_string())
        }
    }

    pub fn id<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
        deserializer.deserialize_any(IdVisitor)
    }

    struct OptStringVisitor;

    impl<'de> Visitor<'de> for OptStringVisitor {
        type Value = Option<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a string, scalar, or null")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D: Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_any(StringVisitor).map(Some)
        }
    }

    struct StringVisitor;

    impl<'de> Visitor<'de> for StringVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a string or scalar value")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
            Ok(value.to_string())
        }

        fn visit_string<E: de::Error>(self, value: String) -> Result<String, E> {
            Ok(value)
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<String, E> {
            Ok(value.to_string())
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<String, E> {
            Ok(value.to_string())
        }

        fn visit_f64<E: de::Error>(self, value: f64) -> Result<String, E> {
            Ok(value.to_string())
        }

        fn visit_bool<E: de::Error>(self, value: bool) -> Result<String, E> {
            Ok(value.to_string())
        }
    }

    pub fn opt_string<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<String>, D::Error> {
        deserializer.deserialize_option(OptStringVisitor)
    }
}

// ---------------------------------------------------------------------------
// Services and setup
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServicesResponse {
    #[serde(default)]
    pub services: Vec<Service>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    #[serde(deserialize_with = "flexible::id")]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plans: Vec<Plan>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    #[serde(deserialize_with = "flexible::id")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Plan {
    pub fn display_name(&self) -> &str {
        self.label
            .as_deref()
            .or(self.name.as_deref())
            .unwrap_or(&self.id)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceOptions {
    #[serde(default)]
    pub service_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plans: Vec<Plan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<Package>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub countries: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub regions: HashMap<String, Vec<Region>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub cities: HashMap<String, Vec<City>>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub isps: HashMap<String, Vec<Isp>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub periods: Option<Periods>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    #[serde(deserialize_with = "flexible::id")]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    #[serde(deserialize_with = "flexible::id")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub isps: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct City {
    #[serde(deserialize_with = "flexible::id")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub isps: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Isp {
    #[serde(deserialize_with = "flexible::id")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Periods {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub months: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub days: Vec<u32>,
}

// ---------------------------------------------------------------------------
// Ordering
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PeriodUnit {
    Months,
    Days,
}

impl fmt::Display for PeriodUnit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Months => formatter.write_str("months"),
            Self::Days => formatter.write_str("days"),
        }
    }
}

impl FromStr for PeriodUnit {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "months" | "month" => Ok(Self::Months),
            "days" | "day" => Ok(Self::Days),
            other => Err(Error::InvalidInput(format!(
                "invalid period unit {other:?}: expected months or days"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Period {
    pub unit: PeriodUnit,
    pub value: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isp_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coupon_code: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoExtend {
    pub is_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traffic: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderRequest {
    #[serde(flatten)]
    pub price: PriceRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_extend: Option<AutoExtend>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedDiscount {
    #[serde(default, deserialize_with = "flexible::id")]
    pub id: String,
    #[serde(default, deserialize_with = "flexible::opt_string")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceResponse {
    #[serde(default)]
    pub applied_discounts: Vec<AppliedDiscount>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub final_price: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub price_no_discounts: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub discount: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub unit_price: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub unit_price_after_discount: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub additional_amount: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub additional_amount_after_discount: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub payment_fee: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub subtotal: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub discount_amount: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub final_price_in_currency: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceQuote {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub final_price: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub price_no_discounts: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub discount: Option<f64>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderResponse {
    #[serde(deserialize_with = "flexible::id")]
    pub id: String,
    #[serde(
        default,
        alias = "periodInMoths",
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_u32"
    )]
    pub period_in_months: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub bandwidth: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub total_price: Option<f64>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

// ---------------------------------------------------------------------------
// Proxies
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proxy {
    #[serde(deserialize_with = "flexible::id")]
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub network_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication: Option<Authentication>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<Connection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_type: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_string"
    )]
    pub created_at: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_string"
    )]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ProxyMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_extend_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bandwidth: Option<Bandwidth>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Authentication {
    #[serde(default)]
    pub whitelisted_ips: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_version: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_u32"
    )]
    pub http_port: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_u32"
    )]
    pub https_port: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_u32"
    )]
    pub socks5_port: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isp_name: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bandwidth {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub total: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub used: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Protocol {
    #[serde(rename = "HTTP")]
    Http,
    #[serde(rename = "SOCKS5")]
    Socks5,
}

impl fmt::Display for Protocol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http => formatter.write_str("HTTP"),
            Self::Socks5 => formatter.write_str("SOCKS5"),
        }
    }
}

impl FromStr for Protocol {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_uppercase().as_str() {
            "HTTP" => Ok(Self::Http),
            "SOCKS5" | "SOCKS" => Ok(Self::Socks5),
            other => Err(Error::InvalidInput(format!(
                "invalid protocol {other:?}: expected HTTP or SOCKS5"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AuthType {
    #[serde(rename = "USERNAME_PASSWORD")]
    UsernamePassword,
    #[serde(rename = "IP_WHITELIST")]
    IpWhitelist,
}

impl fmt::Display for AuthType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UsernamePassword => formatter.write_str("USERNAME_PASSWORD"),
            Self::IpWhitelist => formatter.write_str("IP_WHITELIST"),
        }
    }
}

impl FromStr for AuthType {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_uppercase().replace('-', "_").as_str() {
            "USERNAME_PASSWORD" | "USERNAME" | "PASSWORD" => Ok(Self::UsernamePassword),
            "IP_WHITELIST" | "WHITELIST" | "IP" => Ok(Self::IpWhitelist),
            other => Err(Error::InvalidInput(format!(
                "invalid authentication type {other:?}: expected USERNAME_PASSWORD or IP_WHITELIST"
            ))),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtendPeriodRequest {
    pub period_in_months: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coupon_code: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BandwidthRequest {
    pub bandwidth: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct WhitelistRequest {
    pub ips: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeProtocolRequest {
    pub new_type: Protocol,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeAuthTypeRequest {
    pub new_auth_type: AuthType,
    pub ips: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolOptions {
    #[serde(default)]
    pub current_type: String,
    #[serde(default)]
    pub available_types: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthTypeOptions {
    #[serde(default)]
    pub current_authentication_type: String,
    #[serde(default)]
    pub available_authentication_types: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Balance {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "flexible::opt_f64"
    )]
    pub balance: Option<f64>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

// ---------------------------------------------------------------------------
// Deprecated v1 ordering API
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LegacyOrderRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_whitelist: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coupon_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bandwidth: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_auto_extend_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_extend_bandwidth: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyConfigurationResponse {
    #[serde(default)]
    pub supported_network_types: Vec<String>,
    #[serde(default)]
    pub supported_countries: Vec<String>,
    #[serde(default)]
    pub supported_regions: Vec<String>,
    #[serde(default)]
    pub supported_authentication_types: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_ip_whitelist_count: Option<u32>,
    #[serde(default)]
    pub supported_ip_versions: Vec<String>,
    #[serde(default)]
    pub supported_protocols: Vec<String>,
    #[serde(default)]
    pub supported_isps: Vec<String>,
    #[serde(default)]
    pub supported_packages: Vec<Value>,
    #[serde(default)]
    pub supported_quantity: HashMap<String, u32>,
    #[serde(default)]
    pub supports_adding_bandwidth: bool,
    #[serde(default)]
    pub supports_auto_extending: bool,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_strings_and_ids() {
        let raw = r#"{
            "id": 42,
            "periodInMonths": "1",
            "bandwidth": "3",
            "totalPrice": "3.39"
        }"#;
        let order: OrderResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(order.id, "42");
        assert_eq!(order.period_in_months, Some(1));
        assert_eq!(order.bandwidth, Some(3.0));
        assert_eq!(order.total_price, Some(3.39));
    }

    #[test]
    fn accepts_documented_period_typo() {
        let raw = r#"{"id":"abc","periodInMoths":2,"totalPrice":1.5}"#;
        let order: OrderResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(order.period_in_months, Some(2));
    }

    #[test]
    fn parses_setup_with_isps_map_and_extra_metadata() {
        let raw = r#"{
            "serviceId":"static-residential-ipv6",
            "packages":[{"id":"5","name":"5 proxies"}],
            "countries":["US"],
            "isps":{"US":[{"id":"x","label":"Lumina"}]},
            "periods":{"days":[1,7],"months":[1,12]},
            "metadata":{"availability":{}}
        }"#;
        let options: ServiceOptions = serde_json::from_str(raw).unwrap();
        assert_eq!(options.packages[0].id, "5");
        assert_eq!(options.isps["US"][0].label.as_deref(), Some("Lumina"));
        assert_eq!(options.periods.as_ref().unwrap().days, vec![1, 7]);
        assert!(options.metadata.is_some());
    }

    #[test]
    fn price_request_omits_unset_fields() {
        let request = PriceRequest {
            plan_id: Some("basic".to_string()),
            quantity: Some(1),
            ..Default::default()
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["planId"], "basic");
        assert_eq!(value["quantity"], 1);
        assert!(value.get("packageId").is_none());
        assert!(value.get("couponCode").is_none());
    }

    #[test]
    fn parses_protocol_and_auth_names() {
        assert_eq!("socks5".parse::<Protocol>().unwrap(), Protocol::Socks5);
        assert_eq!(
            "ip-whitelist".parse::<AuthType>().unwrap(),
            AuthType::IpWhitelist
        );
        assert_eq!("months".parse::<PeriodUnit>().unwrap(), PeriodUnit::Months);
    }

    #[test]
    fn proxy_accepts_optional_fields() {
        let raw = r#"{
            "id":"1410379","status":"ACTIVE","networkType":"RESIDENTIAL_STATIC",
            "countryCode":"US",
            "authentication":{"whitelistedIps":[],"username":"u","password":"p"},
            "connection":{"publicIp":"1.2.3.4","connectIp":"1.2.3.4","httpPort":8000,
                          "httpsPort":8001,"socks5Port":8002},
            "proxyType":"HTTPS","createdAt":"2026-01-01T00:00:00Z",
            "expiresAt":"2026-02-01T00:00:00Z","metadata":{"ispName":"ISP"},
            "autoExtendEnabled":true,"bandwidth":{"total":5,"used":1}
        }"#;
        let proxy: Proxy = serde_json::from_str(raw).unwrap();
        assert_eq!(proxy.id, "1410379");
        assert_eq!(proxy.connection.unwrap().http_port, Some(8000));
        assert_eq!(proxy.bandwidth.unwrap().total, Some(5.0));
    }
}
