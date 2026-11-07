# proxy-cheap-mcp

A Rust CLI and [Model Context Protocol](https://modelcontextprotocol.io/) server for the [Proxy-Cheap](https://proxy-cheap.com/) API.

Every REST endpoint in the official Proxy-Cheap API collection is implemented. The same binary works as:

- a normal command-line client
- an MCP server over **stdio**, **Streamable HTTP**, and the legacy **HTTP+SSE** transport
- a webhook receiver with HMAC-SHA256 signature verification
- a portable agent skill

This project is unofficial and not affiliated with Proxy-Cheap.

## Install

From source:

```bash
git clone https://github.com/hongkongkiwi/proxy-cheap-mcp
cd proxy-cheap-mcp
cargo install --path .
```

Or build a release binary:

```bash
cargo build --release
./target/release/proxy-cheap --help
```

Rust 1.85 or newer is required.

## Authenticate

### Browser login

```bash
proxy-cheap login
```

The command opens a compact Chrome or Edge app window at the Proxy-Cheap dashboard login page. Log in normally, including any Cloudflare check. The CLI waits for the `JWTproduction` session cookie, closes the window, validates the token with `GET /account/balance`, and stores it in `~/.config/proxy-cheap/config.toml` with mode 0600. The token is sent as `Authorization: Bearer` and works on the documented REST endpoints.

```bash
proxy-cheap login --environment staging   # staging dashboard
proxy-cheap login --force                 # replace an existing token
proxy-cheap login --browser "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"
proxy-cheap login --profile-dir /path     # alternate browser profile
proxy-cheap config show                   # auth method and token expiry
proxy-cheap logout                        # remove the token
proxy-cheap logout --browser-data         # also delete the browser profile
```

The login window uses a dedicated profile at `~/.config/proxy-cheap/browser-profile`, separate from the normal browser. Chrome, Edge, Brave, and Chromium are detected; override with `--browser` or `PROXY_CHEAP_BROWSER`. The first login may require a Cloudflare check once, after which the clearance is kept in that profile. Tokens expire after roughly 30 days; run `login` again when needed.

The environment and API base are stored with the token, so later commands use the same host. `login --environment staging` switches the stored token to staging; the next login check detects the mismatch instead of reusing a production token. On macOS, an older config at `~/Library/Application Support/proxy-cheap/config.toml` is migrated to `~/.config/proxy-cheap/config.toml` on first load.

A session token is a dashboard credential, not a scoped API key. Keep the config file private; it is written atomically with mode 0600 on Unix. `logout` removes the token but keeps API keys; `logout --browser-data` also deletes the dedicated profile and its cookies.

### API keys

Create keys at https://app.proxy-cheap.com/api-keys.

```bash
export PROXY_CHEAP_API_KEY=...
export PROXY_CHEAP_API_SECRET=...
```

Or persist them in the config file:

```bash
proxy-cheap config init --api-key ... --api-secret ...
```

API keys take precedence over a stored session token. Use one or the other; if both exist, `login` reports the API base your keys will be sent to and warns about the precedence. `PROXY_CHEAP_SESSION_TOKEN` can supply a token directly.

Resolution order: command-line flags, environment, `~/.config/proxy-cheap/config.toml`, then defaults. Service discovery, setup options, and pricing are public and work without credentials. Ordering and all account or proxy calls need them.

## CLI

```bash
# Login
proxy-cheap login
proxy-cheap config show
proxy-cheap logout

# Discovery and pricing
proxy-cheap services list
proxy-cheap services options static-residential-ipv4 --plan-id basic
proxy-cheap services price static-datacenter-ipv4 --plan-id basic --country US --quantity 1 --period-months 1
proxy-cheap services price rotating-residential --traffic 10

# Ordering (spends real account balance)
proxy-cheap services order static-datacenter-ipv4 --plan-id basic --country US --quantity 1 --period-months 1

# Orders
proxy-cheap orders get <order-id>
proxy-cheap orders proxies <order-id>

# Proxies
proxy-cheap proxies list
proxy-cheap proxies get <proxy-id>
proxy-cheap proxies extend <proxy-id> --months 1
proxy-cheap proxies extend-price <proxy-id> --months 1
proxy-cheap proxies buy-bandwidth <proxy-id> --bandwidth 5
proxy-cheap proxies bandwidth-price <proxy-id> --bandwidth 5
proxy-cheap proxies whitelist <proxy-id> --ip 203.0.113.7 --ip 198.51.100.9
proxy-cheap proxies protocols <proxy-id>
proxy-cheap proxies set-protocol <proxy-id> socks5
proxy-cheap proxies auth-types <proxy-id>
proxy-cheap proxies set-auth-type <proxy-id> ip-whitelist --ip 203.0.113.7
proxy-cheap proxies auto-extend enable <proxy-id>
proxy-cheap proxies auto-extend disable <proxy-id>
proxy-cheap proxies rotate-ip <proxy-id>

# Account and the deprecated v1 ordering API
proxy-cheap account balance
proxy-cheap legacy configuration --network-type RESIDENTIAL_STATIC --country US
proxy-cheap legacy price --network-type RESIDENTIAL_STATIC --country US --quantity 1
proxy-cheap legacy execute --network-type RESIDENTIAL_STATIC --country US --quantity 1
```

All commands print JSON. Use `--compact` for one line, `--table` for list views, and `--verbose` for logs on stderr.

Set `PROXY_CHEAP_BASE_URL` or `--base-url` to target a mock or proxy.

## MCP server

### stdio

```bash
proxy-cheap mcp serve
```

Client configuration:

```json
{
  "mcpServers": {
    "proxy-cheap": {
      "command": "proxy-cheap",
      "args": ["mcp", "serve"],
      "env": {
        "PROXY_CHEAP_API_KEY": "...",
        "PROXY_CHEAP_API_SECRET": "..."
      }
    }
  }
}
```

If you have run `proxy-cheap login`, the config file already contains the session token, so the `env` block can be omitted. The MCP server reads the same `~/.config/proxy-cheap/config.toml` as the CLI.

### Streamable HTTP

```bash
proxy-cheap mcp serve --transport http --bind 127.0.0.1:8088
```

Endpoints:

- `POST /mcp` for JSON-RPC requests. Notifications return `202`.
- `GET /mcp` for the server-sent-events stream.
- `DELETE /mcp` to end a session.
- `GET /health` for a health check.

```bash
curl -s http://127.0.0.1:8088/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

### Legacy HTTP+SSE

```bash
proxy-cheap mcp serve --transport sse --bind 127.0.0.1:8088
```

`GET /sse` opens the event stream and announces the `POST /message?sessionId=...` endpoint. Responses are delivered over the stream.

### Both transports on one port

```bash
proxy-cheap mcp serve --transport both --bind 127.0.0.1:8088
```

Both transports bind to loopback by default. Pass `--allow-remote` to bind a public interface, and `--cors-origin <origin>` (repeatable, or `*`) to serve browser clients. CORS is disabled by default.

### Tools

The MCP server exposes 25 tools:

| Area | Tools |
| --- | --- |
| Discovery | `list_services`, `get_service_options` |
| Ordering | `price_order`, `place_order` |
| Orders | `get_order`, `get_order_proxies` |
| Proxies | `list_proxies`, `get_proxy`, `extend_proxy`, `get_period_extension_price`, `buy_proxy_bandwidth`, `get_bandwidth_price`, `update_proxy_whitelist`, `get_proxy_protocols`, `change_proxy_protocol`, `rotate_proxy_ip`, `get_proxy_auth_types`, `change_proxy_auth_type`, `enable_proxy_auto_extend`, `disable_proxy_auto_extend` |
| Account | `get_account_balance` |
| Legacy | `legacy_get_order_configuration`, `legacy_price_order`, `legacy_place_order` |
| Webhooks | `verify_webhook_signature` |

The skill is also available to MCP clients as the resource `proxy-cheap://skill`. Two prompts are included: `order_proxy` and `audit_proxies`.

Inspect tools without a client:

```bash
proxy-cheap mcp tools
proxy-cheap mcp call price_order --args '{"serviceId":"static-datacenter-ipv4","planId":"basic","country":"US","quantity":1,"period":{"unit":"months","value":1}}'
```

## Agent skill

```bash
proxy-cheap skill                 # print SKILL.md
proxy-cheap skill install         # ~/.agents/skills/proxy-cheap/SKILL.md
proxy-cheap skill install --dir ~/.claude/skills/proxy-cheap
```

The skill documents the ordering workflow, every command, the MCP setup, and the safety rules for spend actions.

## Webhooks

Proxy-Cheap signs `event name + event id + raw body` with HMAC-SHA256 and sends `Webhook-Signature: sha256=<hex>`.

Verify one delivery:

```bash
proxy-cheap webhook verify \
  --secret "$PROXY_CHEAP_WEBHOOK_SECRET" \
  --event proxy.status.changed \
  --event-id evt_123 \
  --body '{"proxyId":1410379,"oldStatus":"ACTIVE","status":"CANCELED"}' \
  --signature sha256=...
```

Run a receiver that prints each verified event as one JSON line:

```bash
proxy-cheap webhook listen --bind 127.0.0.1:8099 --path /webhook \
  --secret "$PROXY_CHEAP_WEBHOOK_SECRET"
```

If a secret is configured, missing or invalid signatures are rejected with `401`. Without a secret the receiver accepts events but marks them `"verified": false`.

Supported events: `proxy.maintenance_window.created`, `proxy.maintenance_window.cancelled`, `proxy.status.changed`, `proxy.bandwidth.added`.

## API coverage

| Operation | CLI | MCP tool | Endpoint |
| --- | --- | --- | --- |
| List services | `services list` | `list_services` | `GET /v2/order` |
| Service options | `services options` | `get_service_options` | `POST /v2/order/{serviceId}` |
| Price order | `services price` | `price_order` | `POST /v2/order/{serviceId}/price` |
| Place order | `services order` | `place_order` | `POST /v2/order/{serviceId}/execute` |
| Order details | `orders get` | `get_order` | `GET /orders/{id}` |
| Order proxies | `orders proxies` | `get_order_proxies` | `GET /orders/{id}/proxies` |
| List proxies | `proxies list` | `list_proxies` | `GET /proxies` |
| Proxy details | `proxies get` | `get_proxy` | `GET /proxies/{id}` |
| Extend proxy | `proxies extend` | `extend_proxy` | `POST /proxies/{id}/extend-period` |
| Extension price | `proxies extend-price` | `get_period_extension_price` | `POST /proxies/{id}/period-extension-price` |
| Buy bandwidth | `proxies buy-bandwidth` | `buy_proxy_bandwidth` | `POST /proxies/{id}/buy-bandwidth` |
| Bandwidth price | `proxies bandwidth-price` | `get_bandwidth_price` | `POST /proxies/{id}/bandwidth-price` |
| Update whitelist | `proxies whitelist` | `update_proxy_whitelist` | `POST /proxies/{id}/whitelist-ip` |
| Available protocols | `proxies protocols` | `get_proxy_protocols` | `GET /proxies/{id}/change-protocol` |
| Change protocol | `proxies set-protocol` | `change_proxy_protocol` | `POST /proxies/{id}/change-protocol` |
| Rotate IP | `proxies rotate-ip` | `rotate_proxy_ip` | `POST /proxies/{id}/rotate-ip` |
| Auth methods | `proxies auth-types` | `get_proxy_auth_types` | `GET /proxies/{id}/change-authentication-type` |
| Change auth | `proxies set-auth-type` | `change_proxy_auth_type` | `POST /proxies/{id}/change-authentication-type` |
| Enable auto-extend | `proxies auto-extend enable` | `enable_proxy_auto_extend` | `POST /proxies/{id}/auto-extend/enable` |
| Disable auto-extend | `proxies auto-extend disable` | `disable_proxy_auto_extend` | `POST /proxies/{id}/auto-extend/disable` |
| Balance | `account balance` | `get_account_balance` | `GET /account/balance` |
| Legacy options | `legacy configuration` | `legacy_get_order_configuration` | `POST /order/configuration` |
| Legacy price | `legacy price` | `legacy_price_order` | `POST /order/price` |
| Legacy order | `legacy execute` | `legacy_place_order` | `POST /order/execute` |

## Behaviour notes

The published API documentation lags the live API in a few places. This client handles the differences:

- Setup responses use `packages`, but pricing and ordering expect `packageId`. The CLI maps them.
- `static-residential-ipv4` accepts `region` and `city`; other services reject them.
- Rotating plans take `traffic` and reject `period`. Static plans take `period` and reject `traffic`.
- `periods` can contain `days` as well as `months`.
- Order and proxy identifiers can be numbers, numeric strings, or UUIDs. Numbers and numeric strings are accepted everywhere.
- `GET /orders/{id}` misspells `periodInMoths`. Both spellings are accepted.
- `GET /proxies` returns `{"proxies":[...]}` on the live API, while the documentation and `GET /orders/{id}/proxies` use a bare array. Both shapes are accepted.
- Dashboard session JWTs captured by `proxy-cheap login` are accepted as `Authorization: Bearer` on the documented REST endpoints. This was verified live against balance, proxy listing, order details, proxy details, and protocol/authentication options.
- Several authenticated endpoints answer bad credentials with a `302` HTML redirect to the login page instead of `401`. Redirects are not followed; they are reported as authentication failures.
- Retries: `429`, `5xx`, and network errors are retried only for read-only or idempotent calls (service discovery, setup, price calculation, protocol or whitelist changes). POSTs that spend money or rotate an IP are never retried, because a timed-out or rate-limited request may already have been processed.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 2 | Invalid input |
| 3 | Credentials or configuration |
| 4 | API error |
| 5 | Network error |
| 6 | Webhook signature mismatch |

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Tests use a mock API server. No credentials or network access are required.

## License

MIT. See [LICENSE](LICENSE).
