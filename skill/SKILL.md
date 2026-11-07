---
name: proxy-cheap
description: Order and manage Proxy-Cheap proxies (static residential/ISP, datacenter IPv4/IPv6, rotating residential, mobile) with the proxy-cheap CLI or its MCP server. Use when the user asks to buy proxies, compare proxy prices, inspect proxy status, rotate IPs, extend a proxy, add bandwidth, change whitelists/protocols/authentication, or verify Proxy-Cheap webhooks.
---

# Proxy-Cheap

`proxy-cheap` is both a command-line client and an MCP server for the Proxy-Cheap
API. Every REST endpoint in the official collection is implemented, plus webhook
signature verification and a webhook receiver.

## Authenticate

### Browser login

```bash
proxy-cheap login
```

This opens a compact Chrome or Edge app window at the dashboard login page. Log
in normally, including any Cloudflare check. The CLI waits for the dashboard
session cookie (`JWTproduction`), closes the window, validates the token against
`GET /account/balance`, and stores it in
`~/.config/proxy-cheap/config.toml` with mode 0600. The token is sent as
`Authorization: Bearer`.

```bash
proxy-cheap login --environment staging   # staging dashboard
proxy-cheap login --force                 # replace an existing token
proxy-cheap login --browser "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"
proxy-cheap login --profile-dir /path     # alternate browser profile
proxy-cheap logout                        # remove the token
proxy-cheap logout --browser-data         # also delete the browser profile
```

The login window uses its own profile at
`~/.config/proxy-cheap/browser-profile`, so it never touches the normal browser
session. The first login may show a Cloudflare check once; the clearance is kept
in that profile. The environment and API base are stored with the token, so
`login --environment staging` switches later commands to staging. The token
expires after roughly 30 days; run `login` again when `config show` reports an
expiry in the past.

### API keys

Create keys at https://app.proxy-cheap.com/api-keys.

```bash
export PROXY_CHEAP_API_KEY=...
export PROXY_CHEAP_API_SECRET=...
```

Or persist them with `proxy-cheap config init --api-key ... --api-secret ...`.
API keys take precedence over a stored session token. `PROXY_CHEAP_SESSION_TOKEN`
can supply a token directly.

Precedence: flags, then environment, then `~/.config/proxy-cheap/config.toml`.
Service listing, setup options, and price calculation are public and work without
credentials; ordering and all account/proxy calls require them.

## Discover before ordering

```bash
proxy-cheap services list
proxy-cheap services options static-residential-ipv4 --plan-id basic
proxy-cheap services options static-datacenter-ipv6
```

Setup responses differ per service:

- `rotating-residential` and `rotating-mobile` need only `--traffic` (GB).
- `static-datacenter-*` and `static-residential-*` need a plan, country, and
  either `--quantity` or `--package-id`, plus a period.
- `static-residential-ipv4` can narrow to `--region` and `--city`.
- Periods are `--period-months N` or `--period-days N`; only send one, and only
  if the service lists that unit.
- The price endpoint wants `packageId`; setup responses label the field
  `packages`. The CLI maps this correctly.

## Price, then order

Always price before ordering. Both commands take the same location flags.

```bash
proxy-cheap services price static-datacenter-ipv4 --plan-id basic \
  --country US --quantity 1 --period-months 1

proxy-cheap services price static-residential-ipv4 --plan-id basic \
  --country US --region "New York" --city "New York City" --period-months 1

proxy-cheap services price rotating-residential --traffic 10
```

Placing an order spends real account balance:

```bash
proxy-cheap services order static-datacenter-ipv4 --plan-id basic \
  --country US --quantity 1 --period-months 1
```

Add `--coupon-code CODE` to price or order. Add `--auto-extend` and optionally
`--auto-extend-traffic GB` when ordering. For rotating traffic plans, pass
`--traffic GB`.

## Manage proxies

```bash
proxy-cheap proxies list
proxy-cheap proxies get <proxy-id>
proxy-cheap orders get <order-id>
proxy-cheap orders proxies <order-id>
```

```bash
proxy-cheap proxies extend <proxy-id> --months 1 --coupon-code CODE
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
```

`rotate-ip` works for static mobile proxies only. Extending, buying bandwidth,
and changing settings spend money or change live connectivity.

## Account and legacy API

```bash
proxy-cheap account balance
proxy-cheap legacy configuration --network-type RESIDENTIAL_STATIC --country US
proxy-cheap legacy price --network-type ... --country US --quantity 1
proxy-cheap legacy execute --network-type ... --country US --quantity 1
```

The legacy `order` endpoints are deprecated upstream. Prefer the `services`
commands.

## Webhooks

Verify a delivery by hand:

```bash
proxy-cheap webhook verify --secret "$WEBHOOK_SECRET" \
  --event proxy.status.changed --event-id evt_123 \
  --body '{"proxyId":1410379,"oldStatus":"ACTIVE","status":"CANCELED"}' \
  --signature sha256=...
```

Run a receiver that validates and prints each event as JSON:

```bash
proxy-cheap webhook listen --bind 127.0.0.1:8099 --path /webhook \
  --secret "$WEBHOOK_SECRET"
```

The signature covers `event name + event id + raw body` with HMAC-SHA256. Keep
the raw body unchanged when verifying.

## MCP server

```bash
proxy-cheap mcp serve                          # stdio (Claude Desktop, ZCode, etc.)
proxy-cheap mcp serve --transport http         # Streamable HTTP at /mcp
proxy-cheap mcp serve --transport sse          # legacy HTTP+SSE (/sse + /message)
proxy-cheap mcp serve --transport both --bind 127.0.0.1:8088
```

Example stdio client config:

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

If you have run `proxy-cheap login`, the config file already contains the
session token; the `env` block above can be omitted. The MCP server reads the
same config file as the CLI.

Tools cover the whole API: `list_services`, `get_service_options`,
`price_order`, `place_order`, `get_order`, `get_order_proxies`, `list_proxies`,
`get_proxy`, `extend_proxy`, `get_period_extension_price`,
`buy_proxy_bandwidth`, `get_bandwidth_price`, `update_proxy_whitelist`,
`get_proxy_protocols`, `change_proxy_protocol`, `rotate_proxy_ip`,
`get_proxy_auth_types`, `change_proxy_auth_type`, `enable_proxy_auto_extend`,
`disable_proxy_auto_extend`, `get_account_balance`, and the three
`legacy_*` order tools. `verify_webhook_signature` checks a delivery.

This document is also exposed as the MCP resource `proxy-cheap://skill`.

## Rules for agents

1. Call `list_services` and `get_service_options` before guessing parameters.
2. Always call `price_order` and show the total before `place_order`.
3. Confirm with the user before any tool that spends money: `place_order`,
   `extend_proxy`, `buy_proxy_bandwidth`, `legacy_place_order`.
4. Do not rotate IPs, change protocols, or change authentication methods without
   explicit confirmation; these break live integrations.
5. Treat returned proxy credentials and the stored session token as secrets. Do
   not paste them into logs, commits, or third-party services.
6. Quote API errors verbatim. Validation errors name the failing field and code.
7. `proxy-cheap login` is an interactive CLI action, not an MCP tool. Never try
   to log in by sending credentials through a tool call.

## Output and scripting

All commands print JSON (pretty by default, `--compact` for one line). Global
flags: `--api-key`, `--api-secret`, `--session-token`, `--base-url`,
`--timeout SECONDS`, `--retries N`, `--compact`, `--table`, `--verbose`.
`login` adds `--environment`, `--browser`, `--profile-dir`, `--force`, and
`--login-timeout SECONDS`.

Exit codes: `0` success, `2` invalid input, `3` credentials/config, `4` API
error, `5` network error, `6` webhook signature failure.
