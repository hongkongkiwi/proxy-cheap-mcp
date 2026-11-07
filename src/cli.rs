//! Command-line interface.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use serde_json::json;
use tokio::io::AsyncReadExt;

use crate::api::{
    AuthType, ExtendPeriodRequest, LegacyOrderRequest, OrderRequest, Period, PeriodUnit,
    PriceRequest, Protocol, Proxy, ProxyCheapClient, Service,
};
use crate::browser::{Environment, LoginOptions};
use crate::config::{self, ClientConfig};
use crate::error::{Error, Result};
use crate::mcp::http::HttpConfig;
use crate::mcp::protocol::McpServer;
use crate::mcp::{stdio, tools};
use crate::{login, skill, webhook};

#[derive(Debug, Parser)]
#[command(
    name = "proxy-cheap",
    version,
    about = "CLI and MCP server for the Proxy-Cheap API",
    long_about = "Order and manage Proxy-Cheap proxies from the command line or through \
                  Model Context Protocol (stdio, Streamable HTTP, and legacy SSE).",
    propagate_version = true
)]
pub struct Cli {
    /// Proxy-Cheap API key (X-Api-Key).
    #[arg(
        long,
        global = true,
        env = "PROXY_CHEAP_API_KEY",
        hide_env_values = true
    )]
    pub api_key: Option<String>,

    /// Proxy-Cheap API secret (X-Api-Secret).
    #[arg(
        long,
        global = true,
        env = "PROXY_CHEAP_API_SECRET",
        hide_env_values = true
    )]
    pub api_secret: Option<String>,

    /// Dashboard session JWT captured by `proxy-cheap login`.
    #[arg(
        long,
        global = true,
        env = "PROXY_CHEAP_SESSION_TOKEN",
        hide_env_values = true
    )]
    pub session_token: Option<String>,

    /// Override the API base URL.
    #[arg(long, global = true, env = "PROXY_CHEAP_BASE_URL")]
    pub base_url: Option<String>,

    /// HTTP timeout in seconds.
    #[arg(long, global = true, default_value_t = config::DEFAULT_TIMEOUT_SECS)]
    pub timeout: u64,

    /// Retry count for transient failures (429 and, for GET, 5xx/network errors).
    #[arg(long, global = true, default_value_t = config::DEFAULT_RETRIES)]
    pub retries: u32,

    /// Print compact single-line JSON.
    #[arg(long, global = true)]
    pub compact: bool,

    /// Render list commands as tables.
    #[arg(long, global = true)]
    pub table: bool,

    /// Verbose logs on stderr (safe for stdio MCP).
    #[arg(long, short, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Account information.
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
    /// Discover services and plans, price orders, and place orders.
    Services {
        #[command(subcommand)]
        command: ServicesCommand,
    },
    /// Inspect orders and the proxies they created.
    Orders {
        #[command(subcommand)]
        command: OrdersCommand,
    },
    /// Inspect and manage purchased proxies.
    Proxies {
        #[command(subcommand)]
        command: ProxiesCommand,
    },
    /// Deprecated v1 ordering API.
    Legacy {
        #[command(subcommand)]
        command: LegacyCommand,
    },
    /// Model Context Protocol server.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Webhook signature verification and receiver.
    Webhook {
        #[command(subcommand)]
        command: WebhookCommand,
    },
    /// Print or install the bundled agent skill.
    Skill {
        #[command(subcommand)]
        command: Option<SkillCommand>,
    },
    /// Credentials and configuration file.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Log in through the Proxy-Cheap dashboard in a browser.
    Login {
        /// Proxy-Cheap environment to log in to.
        #[arg(long, value_parser = ["production", "staging", "local"])]
        environment: Option<String>,
        /// Override the dashboard URL opened in the browser.
        #[arg(long)]
        app_url: Option<String>,
        /// Override the session cookie name.
        #[arg(long)]
        cookie_name: Option<String>,
        /// Browser executable to launch.
        #[arg(long)]
        browser: Option<PathBuf>,
        /// Seconds to wait for the login to complete.
        #[arg(long, default_value_t = 300)]
        login_timeout: u64,
        /// Log in again even if a usable session token is already stored.
        #[arg(long)]
        force: bool,
        /// Dedicated browser profile directory.
        #[arg(long)]
        profile_dir: Option<PathBuf>,
    },
    /// Remove the stored session token (API keys are kept).
    Logout {
        /// Also delete the dedicated browser profile and its cookies.
        #[arg(long)]
        browser_data: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum AccountCommand {
    /// Get the account balance.
    Balance,
}

#[derive(Debug, Subcommand)]
pub enum ServicesCommand {
    /// List orderable services and their plans.
    List,
    /// Show the order options for a service.
    Options {
        /// Service id from `services list`.
        service_id: String,
        /// Plan id for services that require one.
        #[arg(long)]
        plan_id: Option<String>,
    },
    /// Calculate a price without ordering.
    Price {
        /// Service id from `services list`.
        service_id: String,
        #[command(flatten)]
        args: PriceArgs,
    },
    /// Place an order. Spends real account balance.
    Order {
        /// Service id from `services list`.
        service_id: String,
        #[command(flatten)]
        args: OrderArgs,
    },
}

#[derive(Debug, Clone, Default, Args)]
pub struct PriceArgs {
    /// Plan id from `services options`.
    #[arg(long)]
    pub plan_id: Option<String>,
    /// Package id for package-based services (IPv6 plans).
    #[arg(long)]
    pub package_id: Option<String>,
    /// Number of proxies for per-IP services.
    #[arg(long)]
    pub quantity: Option<u32>,
    /// Two-letter ISO country code.
    #[arg(long)]
    pub country: Option<String>,
    /// Region id (static residential IPv4 only).
    #[arg(long)]
    pub region: Option<String>,
    /// City id (static residential IPv4 only).
    #[arg(long)]
    pub city: Option<String>,
    /// ISP id from the setup response.
    #[arg(long)]
    pub isp_id: Option<String>,
    /// Billing period in months. Do not combine with --period-days.
    #[arg(long, conflicts_with = "period_days")]
    pub period_months: Option<u32>,
    /// Billing period in days. Do not combine with --period-months.
    #[arg(long)]
    pub period_days: Option<u32>,
    /// Traffic in GB for rotating plans.
    #[arg(long)]
    pub traffic: Option<f64>,
    /// Optional coupon code.
    #[arg(long)]
    pub coupon_code: Option<String>,
}

impl PriceArgs {
    fn period(&self) -> Option<Period> {
        if let Some(value) = self.period_months {
            Some(Period {
                unit: PeriodUnit::Months,
                value,
            })
        } else {
            self.period_days.map(|value| Period {
                unit: PeriodUnit::Days,
                value,
            })
        }
    }

    fn to_request(&self) -> PriceRequest {
        PriceRequest {
            plan_id: self.plan_id.clone(),
            package_id: self.package_id.clone(),
            quantity: self.quantity,
            country: self.country.clone(),
            region: self.region.clone(),
            city: self.city.clone(),
            isp_id: self.isp_id.clone(),
            period: self.period(),
            traffic: self.traffic,
            coupon_code: self.coupon_code.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Args)]
pub struct OrderArgs {
    #[command(flatten)]
    pub price: PriceArgs,
    /// Enable auto-extend when the service supports it.
    #[arg(long)]
    pub auto_extend: bool,
    /// Traffic threshold in GB for auto-extend.
    #[arg(long)]
    pub auto_extend_traffic: Option<f64>,
}

#[derive(Debug, Subcommand)]
pub enum OrdersCommand {
    /// Show order details.
    Get {
        /// Order id.
        order_id: String,
    },
    /// List proxies created by an order.
    Proxies {
        /// Order id.
        order_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProxiesCommand {
    /// List all active proxies.
    List,
    /// Show details for one proxy.
    Get {
        /// Proxy id.
        proxy_id: String,
    },
    /// Extend a proxy period. Spends account balance.
    Extend {
        /// Proxy id.
        proxy_id: String,
        /// Number of months to add.
        #[arg(long)]
        months: u32,
        /// Optional coupon code.
        #[arg(long)]
        coupon_code: Option<String>,
    },
    /// Price a period extension without extending.
    ExtendPrice {
        /// Proxy id.
        proxy_id: String,
        /// Number of months to price.
        #[arg(long)]
        months: u32,
    },
    /// Buy additional bandwidth. Spends account balance.
    BuyBandwidth {
        /// Proxy id.
        proxy_id: String,
        /// Bandwidth in GB.
        #[arg(long)]
        bandwidth: u32,
    },
    /// Price additional bandwidth without buying it.
    BandwidthPrice {
        /// Proxy id.
        proxy_id: String,
        /// Bandwidth in GB.
        #[arg(long)]
        bandwidth: u32,
    },
    /// Replace the proxy IP whitelist.
    Whitelist {
        /// Proxy id.
        proxy_id: String,
        /// IP address to whitelist; repeat or use commas.
        #[arg(long = "ip", value_delimiter = ',', required = true)]
        ips: Vec<String>,
    },
    /// Show the current and available protocols.
    Protocols {
        /// Proxy id.
        proxy_id: String,
    },
    /// Change the proxy protocol.
    SetProtocol {
        /// Proxy id.
        proxy_id: String,
        /// Target protocol.
        protocol: Protocol,
    },
    /// Show the current and available authentication methods.
    AuthTypes {
        /// Proxy id.
        proxy_id: String,
    },
    /// Change the proxy authentication method.
    SetAuthType {
        /// Proxy id.
        proxy_id: String,
        /// Target authentication method.
        auth_type: AuthType,
        /// IP address to whitelist; repeat or use commas.
        #[arg(long = "ip", value_delimiter = ',')]
        ips: Vec<String>,
    },
    /// Assign a new public IP (static mobile proxies only).
    RotateIp {
        /// Proxy id.
        proxy_id: String,
    },
    /// Enable or disable automatic period extension.
    AutoExtend {
        #[command(subcommand)]
        action: AutoExtendCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum AutoExtendCommand {
    /// Enable auto-extend.
    Enable {
        /// Proxy id.
        proxy_id: String,
    },
    /// Disable auto-extend.
    Disable {
        /// Proxy id.
        proxy_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum LegacyCommand {
    /// Deprecated: available options for a configuration.
    Configuration {
        #[command(flatten)]
        args: LegacyArgs,
    },
    /// Deprecated: price a configuration.
    Price {
        #[command(flatten)]
        args: LegacyArgs,
    },
    /// Deprecated: execute an order. Spends account balance.
    Execute {
        #[command(flatten)]
        args: LegacyArgs,
    },
}

#[derive(Debug, Clone, Default, Args)]
pub struct LegacyArgs {
    /// Network type, for example RESIDENTIAL_STATIC.
    #[arg(long)]
    pub network_type: Option<String>,
    /// IP version, for example IPv4.
    #[arg(long)]
    pub ip_version: Option<String>,
    /// Country code.
    #[arg(long)]
    pub country: Option<String>,
    /// Region.
    #[arg(long)]
    pub region: Option<String>,
    /// ISP.
    #[arg(long)]
    pub isp: Option<String>,
    /// Proxy protocol, for example HTTP.
    #[arg(long)]
    pub proxy_protocol: Option<String>,
    /// Authentication type, for example USERNAME_PASSWORD.
    #[arg(long)]
    pub authentication_type: Option<String>,
    /// Whitelisted IP; repeat or use commas.
    #[arg(long = "ip", value_delimiter = ',')]
    pub ip_whitelist: Vec<String>,
    /// Package identifier.
    #[arg(long)]
    pub package: Option<String>,
    /// Quantity.
    #[arg(long)]
    pub quantity: Option<u32>,
    /// Optional coupon code.
    #[arg(long)]
    pub coupon_code: Option<String>,
    /// Bandwidth.
    #[arg(long)]
    pub bandwidth: Option<f64>,
    /// Enable auto-extend.
    #[arg(long)]
    pub auto_extend_enabled: bool,
    /// Auto-extend bandwidth threshold.
    #[arg(long)]
    pub auto_extend_bandwidth: Option<f64>,
}

impl LegacyArgs {
    fn to_request(&self) -> LegacyOrderRequest {
        LegacyOrderRequest {
            network_type: self.network_type.clone(),
            ip_version: self.ip_version.clone(),
            country: self.country.clone(),
            region: self.region.clone(),
            isp: self.isp.clone(),
            proxy_protocol: self.proxy_protocol.clone(),
            authentication_type: self.authentication_type.clone(),
            ip_whitelist: if self.ip_whitelist.is_empty() {
                None
            } else {
                Some(self.ip_whitelist.clone())
            },
            package: self.package.clone(),
            quantity: self.quantity,
            coupon_code: self.coupon_code.clone(),
            bandwidth: self.bandwidth,
            is_auto_extend_enabled: self.auto_extend_enabled.then_some(true),
            auto_extend_bandwidth: self.auto_extend_bandwidth,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Run the MCP server.
    Serve {
        /// Transport to use.
        #[arg(long, default_value = "stdio", value_parser = ["stdio", "http", "sse", "both"])]
        transport: String,
        /// Bind address for HTTP or SSE transports.
        #[arg(long, default_value = "127.0.0.1:8088")]
        bind: String,
        /// Streamable HTTP endpoint path.
        #[arg(long, default_value = "/mcp")]
        path: String,
        /// Allow binding to a non-loopback address.
        #[arg(long)]
        allow_remote: bool,
        /// Allowed CORS origin; repeat or use * for any.
        #[arg(long = "cors-origin")]
        cors_origins: Vec<String>,
    },
    /// Print every tool definition and input schema as JSON.
    Tools,
    /// Call one MCP tool directly.
    Call {
        /// Tool name.
        tool: String,
        /// JSON arguments object.
        #[arg(long, conflicts_with = "args_file")]
        args: Option<String>,
        /// File containing a JSON arguments object.
        #[arg(long)]
        args_file: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum WebhookCommand {
    /// Run a receiver that verifies and prints webhook events.
    Listen {
        /// Bind address.
        #[arg(long, default_value = "127.0.0.1:8099")]
        bind: String,
        /// Receiver path.
        #[arg(long, default_value = "/webhook")]
        path: String,
        /// Webhook secret used to verify signatures.
        #[arg(long, env = "PROXY_CHEAP_WEBHOOK_SECRET", hide_env_values = true)]
        secret: Option<String>,
        /// Allow binding to a non-loopback address.
        #[arg(long)]
        allow_remote: bool,
    },
    /// Verify one delivery signature.
    Verify {
        /// Webhook secret.
        #[arg(long, env = "PROXY_CHEAP_WEBHOOK_SECRET", hide_env_values = true)]
        secret: String,
        /// Value of the Webhook-Event header.
        #[arg(long)]
        event: String,
        /// Value of the Webhook-Id header.
        #[arg(long, default_value = "")]
        event_id: String,
        /// Raw body as a string.
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// File containing the raw body.
        #[arg(long)]
        body_file: Option<PathBuf>,
        /// Value of the Webhook-Signature header.
        #[arg(long)]
        signature: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// Print the bundled SKILL.md.
    Print,
    /// Install SKILL.md into a skill directory.
    Install {
        /// Directory that should contain SKILL.md.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Print the default install path.
    Path,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Create or update the config file.
    Init {
        /// Proxy-Cheap API key.
        #[arg(long)]
        api_key: Option<String>,
        /// Proxy-Cheap API secret.
        #[arg(long)]
        api_secret: Option<String>,
        /// API base URL.
        #[arg(long)]
        base_url: Option<String>,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },
    /// Show the resolved configuration with secrets redacted.
    Show,
    /// Print the config file path.
    Path,
}

struct Output {
    compact: bool,
    table: bool,
}

impl Output {
    fn print<T: Serialize>(&self, value: &T) -> Result<()> {
        let text = if self.compact {
            serde_json::to_string(value)?
        } else {
            serde_json::to_string_pretty(value)?
        };
        println!("{text}");
        Ok(())
    }

    fn services(&self, services: &[Service]) -> Result<()> {
        if !self.table {
            return self.print(&services);
        }
        println!("{:<28} {:<32} PLANS", "ID", "LABEL");
        for service in services {
            let plans = service
                .plans
                .iter()
                .map(|plan| plan.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            println!("{:<28} {:<32} {}", service.id, service.label, plans);
        }
        Ok(())
    }

    fn proxies(&self, proxies: &[Proxy]) -> Result<()> {
        if !self.table {
            return self.print(&proxies);
        }
        println!(
            "{:<12} {:<10} {:<24} {:<4} {:<16} {:<6} {:<6} {:<8} EXPIRES",
            "ID", "STATUS", "NETWORK", "CC", "PUBLIC IP", "HTTP", "SOCKS", "PROTOCOL"
        );
        for proxy in proxies {
            let connection = proxy.connection.as_ref();
            let public_ip = connection
                .and_then(|connection| connection.public_ip.clone())
                .unwrap_or_default();
            let http = connection
                .and_then(|connection| connection.http_port)
                .map(|port| port.to_string())
                .unwrap_or_default();
            let socks = connection
                .and_then(|connection| connection.socks5_port)
                .map(|port| port.to_string())
                .unwrap_or_default();
            println!(
                "{:<12} {:<10} {:<24} {:<4} {:<16} {:<6} {:<6} {:<8} {}",
                proxy.id,
                proxy.status,
                proxy.network_type,
                proxy.country_code.as_deref().unwrap_or(""),
                public_ip,
                http,
                socks,
                proxy.proxy_type.as_deref().unwrap_or(""),
                proxy.expires_at.as_deref().unwrap_or("")
            );
        }
        Ok(())
    }
}

pub async fn run(cli: Cli) -> Result<()> {
    let config = ClientConfig::resolve(
        cli.api_key,
        cli.api_secret,
        cli.session_token,
        cli.base_url,
        cli.timeout,
        cli.retries,
    )?;
    let output = Output {
        compact: cli.compact,
        table: cli.table,
    };
    let client = Arc::new(ProxyCheapClient::new(&config)?);

    match cli.command {
        Command::Account { command } => match command {
            AccountCommand::Balance => output.print(&client.balance().await?),
        },
        Command::Services { command } => match command {
            ServicesCommand::List => output.services(&client.list_services().await?.services),
            ServicesCommand::Options {
                service_id,
                plan_id,
            } => output.print(
                &client
                    .service_options(&service_id, plan_id.as_deref())
                    .await?,
            ),
            ServicesCommand::Price { service_id, args } => {
                output.print(&client.price(&service_id, &args.to_request()).await?)
            }
            ServicesCommand::Order { service_id, args } => {
                let request = OrderRequest {
                    price: args.price.to_request(),
                    auto_extend: args.auto_extend.then_some(crate::api::AutoExtend {
                        is_enabled: true,
                        traffic: args.auto_extend_traffic,
                    }),
                };
                output.print(&client.place_order(&service_id, &request).await?)
            }
        },
        Command::Orders { command } => match command {
            OrdersCommand::Get { order_id } => output.print(&client.get_order(&order_id).await?),
            OrdersCommand::Proxies { order_id } => {
                output.print(&client.order_proxies(&order_id).await?)
            }
        },
        Command::Proxies { command } => match command {
            ProxiesCommand::List => output.proxies(&client.list_proxies().await?),
            ProxiesCommand::Get { proxy_id } => output.print(&client.get_proxy(&proxy_id).await?),
            ProxiesCommand::Extend {
                proxy_id,
                months,
                coupon_code,
            } => {
                let request = ExtendPeriodRequest {
                    period_in_months: months,
                    coupon_code,
                };
                output.print(&client.extend_proxy(&proxy_id, &request).await?)
            }
            ProxiesCommand::ExtendPrice { proxy_id, months } => {
                output.print(&client.period_extension_price(&proxy_id, months).await?)
            }
            ProxiesCommand::BuyBandwidth {
                proxy_id,
                bandwidth,
            } => output.print(&client.buy_bandwidth(&proxy_id, bandwidth).await?),
            ProxiesCommand::BandwidthPrice {
                proxy_id,
                bandwidth,
            } => output.print(&client.bandwidth_price(&proxy_id, bandwidth).await?),
            ProxiesCommand::Whitelist { proxy_id, ips } => {
                output.print(&client.update_whitelist(&proxy_id, &ips).await?)
            }
            ProxiesCommand::Protocols { proxy_id } => {
                output.print(&client.protocol_options(&proxy_id).await?)
            }
            ProxiesCommand::SetProtocol { proxy_id, protocol } => {
                client.change_protocol(&proxy_id, protocol).await?;
                output.print(&json!({"ok": true}))
            }
            ProxiesCommand::AuthTypes { proxy_id } => {
                output.print(&client.auth_type_options(&proxy_id).await?)
            }
            ProxiesCommand::SetAuthType {
                proxy_id,
                auth_type,
                ips,
            } => {
                client.change_auth_type(&proxy_id, auth_type, ips).await?;
                output.print(&json!({"ok": true}))
            }
            ProxiesCommand::RotateIp { proxy_id } => {
                client.rotate_ip(&proxy_id).await?;
                output.print(&json!({"ok": true}))
            }
            ProxiesCommand::AutoExtend { action } => {
                let (proxy_id, enabled) = match action {
                    AutoExtendCommand::Enable { proxy_id } => (proxy_id, true),
                    AutoExtendCommand::Disable { proxy_id } => (proxy_id, false),
                };
                client.set_auto_extend(&proxy_id, enabled).await?;
                output.print(&json!({"ok": true}))
            }
        },
        Command::Legacy { command } => match command {
            LegacyCommand::Configuration { args } => {
                output.print(&client.legacy_configuration(&args.to_request()).await?)
            }
            LegacyCommand::Price { args } => {
                output.print(&client.legacy_price(&args.to_request()).await?)
            }
            LegacyCommand::Execute { args } => {
                output.print(&client.legacy_execute(&args.to_request()).await?)
            }
        },
        Command::Mcp { command } => match command {
            McpCommand::Serve {
                transport,
                bind,
                path,
                allow_remote,
                cors_origins,
            } => {
                if !client.has_credentials() {
                    tracing::warn!(
                        "no API credentials configured; tools that require authentication will fail"
                    );
                }
                let server = Arc::new(McpServer::new(client));
                match transport.as_str() {
                    "stdio" => stdio::serve(server).await,
                    "http" => {
                        http_serve(server, bind, path, allow_remote, cors_origins, true, false)
                            .await
                    }
                    "sse" => {
                        http_serve(server, bind, path, allow_remote, cors_origins, false, true)
                            .await
                    }
                    "both" => {
                        http_serve(server, bind, path, allow_remote, cors_origins, true, true).await
                    }
                    other => Err(Error::InvalidInput(format!("unknown transport {other:?}"))),
                }
            }
            McpCommand::Tools => output.print(&tools::definitions()),
            McpCommand::Call {
                tool,
                args,
                args_file,
            } => {
                let arguments = match (args, args_file) {
                    (Some(args), _) => serde_json::from_str(&args)?,
                    (None, Some(path)) => {
                        serde_json::from_str(&std::fs::read_to_string(&path).map_err(|error| {
                            Error::InvalidInput(format!(
                                "could not read {}: {error}",
                                path.display()
                            ))
                        })?)?
                    }
                    (None, None) => json!({}),
                };
                match tools::call(client, &tool, Some(arguments)).await {
                    Ok(value) => output.print(&value),
                    Err(tools::ToolError::Unknown(name)) => {
                        Err(Error::InvalidInput(format!("unknown tool: {name}")))
                    }
                    Err(tools::ToolError::Failed(error)) => Err(error),
                }
            }
        },
        Command::Webhook { command } => match command {
            WebhookCommand::Listen {
                bind,
                path,
                secret,
                allow_remote,
            } => {
                if secret.is_none() {
                    tracing::warn!("webhook receiver started without a secret");
                }
                webhook::serve(webhook::WebhookConfig {
                    bind,
                    path,
                    secret,
                    allow_remote,
                })
                .await
            }
            WebhookCommand::Verify {
                secret,
                event,
                event_id,
                body,
                body_file,
                signature,
            } => {
                let body = match (body, body_file) {
                    (Some(body), _) => body,
                    (None, Some(path)) => std::fs::read_to_string(&path).map_err(|error| {
                        Error::InvalidInput(format!("could not read {}: {error}", path.display()))
                    })?,
                    (None, None) => {
                        let mut text = String::new();
                        tokio::io::stdin()
                            .read_to_string(&mut text)
                            .await
                            .map_err(|error| {
                                Error::InvalidInput(format!("could not read stdin: {error}"))
                            })?;
                        text
                    }
                };
                let expected = webhook::signature(&secret, &event, &event_id, body.as_bytes())?;
                let valid =
                    webhook::verify(&secret, &event, &event_id, body.as_bytes(), &signature);
                output.print(&json!({ "valid": valid, "expected": expected }))?;
                if valid {
                    Ok(())
                } else {
                    Err(Error::SignatureMismatch)
                }
            }
        },
        Command::Skill { command } => match command.unwrap_or(SkillCommand::Print) {
            SkillCommand::Print => {
                print!("{}", skill::SKILL_MD);
                Ok(())
            }
            SkillCommand::Install { dir } => {
                let path = skill::install(dir.as_deref())?;
                output.print(&json!({ "installed": path }))
            }
            SkillCommand::Path => output.print(&json!({
                "path": skill::default_skill_dir().join("SKILL.md")
            })),
        },
        Command::Login {
            environment,
            app_url,
            cookie_name,
            browser,
            login_timeout,
            force,
            profile_dir,
        } => {
            let explicit_environment = environment.is_some();
            let environment = match environment {
                Some(name) => Environment::from_name(&name)?,
                None => Environment::from_base_url(&config.base_url),
            };
            let mut effective = config.clone();
            if explicit_environment {
                effective.base_url = environment.api_base().to_string();
            }
            if !force {
                if let Some(token) = effective.session_token.as_deref() {
                    let unexpired =
                        login::jwt_expiry(token).is_some_and(|expiry| expiry > now_unix() + 60);
                    let same_environment =
                        effective.session_token_environment.as_deref() == Some(environment.label());
                    if unexpired && same_environment {
                        return output.print(&json!({
                            "loggedIn": true,
                            "alreadyLoggedIn": true,
                            "environment": environment.label(),
                            "authMethod": "session-token",
                            "expiresAt": effective.session_token_expires_at,
                            "configPath": config::config_path(),
                        }));
                    }
                }
            }
            let mut options = LoginOptions::new(environment);
            options.app_url = app_url;
            options.cookie_name = cookie_name;
            options.browser = browser;
            options.profile_dir = profile_dir.unwrap_or_else(config::browser_profile_dir);
            options.timeout = Duration::from_secs(login_timeout);
            let path = config::config_path();
            let outcome = login::capture_and_store(&effective, &path, &options).await?;
            let note = effective
                .has_api_key()
                .then_some("existing API keys take precedence over the session token");
            output.print(&json!({
                "loggedIn": true,
                "environment": outcome.environment.label(),
                "authMethod": "session-token",
                "expiresAt": outcome.expires_at,
                "configPath": path,
                "profileDir": outcome.profile_dir,
                "note": note,
            }))
        }
        Command::Logout { browser_data } => {
            let path = config::config_path();
            let mut file = config::load_for_update(&path)?.unwrap_or_default();
            let had_token = file.session_token.take().is_some();
            file.session_token_expires_at = None;
            file.session_token_environment = None;
            if had_token || path.exists() {
                config::save_to(&path, &file)?;
            }
            let profile = config::browser_profile_dir();
            let mut removed_profile = false;
            if browser_data && profile.exists() {
                std::fs::remove_dir_all(&profile).map_err(|error| {
                    Error::Config(format!("could not remove {}: {error}", profile.display()))
                })?;
                removed_profile = true;
            }
            let note = (!browser_data && profile.exists()).then_some(
                "browser profile and cookies were kept; pass --browser-data to delete them",
            );
            output.print(&json!({
                "loggedOut": true,
                "removedSessionToken": had_token,
                "removedBrowserProfile": removed_profile,
                "profileDir": profile,
                "note": note,
            }))
        }
        Command::Config { command } => match command {
            ConfigCommand::Init {
                api_key,
                api_secret,
                base_url,
                force,
            } => {
                let path = config::config_path();
                if path.exists() && !force {
                    return Err(Error::Config(format!(
                        "{} already exists; pass --force to overwrite",
                        path.display()
                    )));
                }
                let existing = config::load_for_update(&path)?.unwrap_or_default();
                let merged = config::FileConfig {
                    api_key: api_key.or(existing.api_key),
                    api_secret: api_secret.or(existing.api_secret),
                    session_token: existing.session_token,
                    session_token_expires_at: existing.session_token_expires_at,
                    session_token_environment: existing.session_token_environment,
                    base_url: base_url.or(existing.base_url),
                    extra: existing.extra,
                };
                if merged.api_key.is_none() || merged.api_secret.is_none() {
                    return Err(Error::InvalidInput(
                        "config init requires --api-key and --api-secret".to_string(),
                    ));
                }
                config::save_to(&path, &merged)?;
                output.print(&json!({
                    "path": path,
                    "apiKey": merged.api_key.as_deref().map(config::redact),
                    "apiSecret": merged.api_secret.as_deref().map(config::redact),
                    "baseUrl": merged.base_url.as_deref().unwrap_or(config::DEFAULT_BASE_URL),
                }))
            }
            ConfigCommand::Show => output.print(&json!({
                "configPath": config::config_path(),
                "baseUrl": config.base_url,
                "authMethod": config.auth_method(),
                "apiKey": config.api_key.as_deref().map(config::redact),
                "apiSecret": config.api_secret.as_deref().map(config::redact),
                "sessionToken": config.session_token.as_deref().map(config::redact),
                "sessionTokenExpiresAt": config.session_token_expires_at,
                "sessionTokenEnvironment": config.session_token_environment,
                "apiKeyConfigured": config.has_api_key(),
                "sessionTokenConfigured": config.has_session_token(),
                "timeoutSeconds": config.timeout_secs,
                "retries": config.max_retries,
                "credentialsConfigured": config.has_api_key() || config.has_session_token(),
            })),
            ConfigCommand::Path => output.print(&json!({ "path": config::config_path() })),
        },
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

async fn http_serve(
    server: Arc<McpServer>,
    bind: String,
    path: String,
    allow_remote: bool,
    cors_origins: Vec<String>,
    streamable: bool,
    sse: bool,
) -> Result<()> {
    crate::mcp::http::serve(
        server,
        HttpConfig {
            bind,
            path,
            allow_remote,
            cors_origins,
            streamable,
            sse,
        },
    )
    .await
}
