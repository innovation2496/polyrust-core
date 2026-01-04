# Polymarket External Protocol Contract

> **Authority**: All information in this document is derived from official Polymarket documentation.
> **Date**: 2026-01-04
> **Version**: 1.0.0

---

## 1. REST API Endpoints

| Name | Base URL | Purpose | Authentication |
|------|----------|---------|----------------|
| CLOB API | `https://clob.polymarket.com` | Order management, prices, orderbooks | L2 (for write ops) |
| Gamma API | `https://gamma-api.polymarket.com` | Market discovery, metadata, events | None |
| Data API | `https://data-api.polymarket.com` | User positions, trading history | L2 |

**Source**: https://docs.polymarket.com/quickstart/reference/endpoints

---

## 2. WebSocket Endpoints

| Name | URL | Purpose |
|------|-----|---------|
| CLOB WebSocket | `wss://ws-subscriptions-clob.polymarket.com/ws/` | Orderbook updates, order status |
| Real-Time Data Stream (RTDS) | `wss://ws-live-data.polymarket.com` | Low-latency crypto prices, comments |

**Source**: https://docs.polymarket.com/quickstart/reference/endpoints

---

## 3. WebSocket Channels

### 3.1 Channel Types

| Channel | Type Value | Authentication | Purpose |
|---------|------------|----------------|---------|
| Market | `"MARKET"` | None | Market-wide events (orderbook, trades, prices) |
| User | `"USER"` | Required (apiKey/secret/passphrase) | User-specific events (orders, fills) |

**Source**: https://docs.polymarket.com/developers/CLOB/websocket/wss-overview

### 3.2 Subscription Format

To subscribe to a channel, send a JSON message with:

```json
{
  "auth": { ... },           // Auth object (required for user channel only)
  "markets": ["..."],        // Condition IDs (for user channel)
  "asset_ids": ["..."],      // Token IDs (for market channel)
  "type": "MARKET",          // Channel type: "MARKET" or "USER"
  "custom_feature_enabled": true  // Optional: enable feature-flagged messages
}
```

### 3.3 Dynamic Subscription (after initial connection)

```json
{
  "asset_ids": ["..."],      // Token IDs to add/remove
  "operation": "subscribe",  // or "unsubscribe"
  "custom_feature_enabled": true
}
```

**Source**: https://docs.polymarket.com/developers/CLOB/websocket/wss-overview

---

## 4. Market Channel Message Types

All messages include an `event_type` field to identify the message type.

### 4.1 `book` Message
Emitted when first subscribing or when trades affect the orderbook.

| Field | Type | Description |
|-------|------|-------------|
| `event_type` | string | `"book"` |
| `asset_id` | string | Token identifier |
| `market` | string | Condition ID |
| `timestamp` | number | Unix timestamp (milliseconds) |
| `hash` | string | Orderbook content hash |
| `buys` | array | Array of OrderSummary (price, size pairs) |
| `sells` | array | Array of OrderSummary (price, size pairs) |

### 4.2 `price_change` Message
Emitted when orders are placed or cancelled.

| Field | Type | Description |
|-------|------|-------------|
| `event_type` | string | `"price_change"` |
| `market` | string | Condition ID |
| `timestamp` | number | Unix timestamp (milliseconds) |
| `price_changes` | array | Array of price change objects |

Price change object fields:
- `asset_id`, `price`, `size`, `side` (BUY/SELL)
- `hash`, `best_bid`, `best_ask`

### 4.3 `tick_size_change` Message
Emitted when minimum tick size adjusts (price >0.96 or <0.04).

| Field | Type | Description |
|-------|------|-------------|
| `event_type` | string | `"tick_size_change"` |
| `asset_id` | string | Token identifier |
| `market` | string | Condition ID |
| `timestamp` | number | Unix timestamp (milliseconds) |
| `old_tick_size` | string | Previous tick size |
| `new_tick_size` | string | New tick size |
| `side` | string | buy/sell indicator |

### 4.4 `last_trade_price` Message
Emitted when maker-taker orders match.

| Field | Type | Description |
|-------|------|-------------|
| `event_type` | string | `"last_trade_price"` |
| `asset_id` | string | Token identifier |
| `market` | string | Condition ID |
| `timestamp` | number | Unix timestamp (milliseconds) |
| `price` | string | Trade price |
| `size` | string | Trade size |
| `side` | string | BUY/SELL |
| `fee_rate_bps` | number | Fee rate in basis points |

### 4.5 `best_bid_ask` Message (feature-flagged)
Emitted when best bid/ask prices change. Requires `custom_feature_enabled: true`.

| Field | Type | Description |
|-------|------|-------------|
| `event_type` | string | `"best_bid_ask"` |
| `asset_id` | string | Token identifier |
| `market` | string | Condition ID |
| `timestamp` | number | Unix timestamp (milliseconds) |
| `best_bid` | string | Best bid price |
| `best_ask` | string | Best ask price |
| `spread` | string | Spread |

### 4.6 `new_market` Message (feature-flagged)
Emitted when a new market is created. Requires `custom_feature_enabled: true`.

### 4.7 `market_resolved` Message (feature-flagged)
Emitted when a market is resolved. Requires `custom_feature_enabled: true`.

**Source**: https://docs.polymarket.com/developers/CLOB/websocket/market-channel

---

## 5. User Channel Message Types

### 5.1 `trade` Message
Emitted when trades occur or change status.

Triggered when:
- Market order is matched ("MATCHED")
- Limit order is included in a trade ("MATCHED")
- Trade status changes ("MINED", "CONFIRMED", "RETRYING", "FAILED")

| Field | Type | Description |
|-------|------|-------------|
| `event_type` | string | `"trade"` |
| `id` | string | Trade identifier |
| `asset_id` | string | Token identifier |
| `market` | string | Condition ID |
| `matchtime` | string | When trade was matched |
| `outcome` | string | Trade outcome |
| `price` | string | Trade price |
| `side` | string | BUY/SELL |
| `size` | string | Trade quantity |
| `status` | string | Current trade status |
| `maker_orders` | array | Details of maker orders involved |
| `taker_order_id` | string | Associated taker order |

### 5.2 `order` Message
Emitted when orders are placed, updated, or cancelled.

Triggered when:
- Order is placed (PLACEMENT)
- Order is updated/partially filled (UPDATE)
- Order is canceled (CANCELLATION)

| Field | Type | Description |
|-------|------|-------------|
| `event_type` | string | `"order"` |
| `id` | string | Order identifier |
| `asset_id` | string | Token identifier |
| `market` | string | Condition ID |
| `original_size` | string | Initial order size |
| `outcome` | string | Market outcome |
| `price` | string | Order price |
| `side` | string | BUY/SELL |
| `size_matched` | string | Matched quantity |
| `type` | string | PLACEMENT/UPDATE/CANCELLATION |

**Source**: https://docs.polymarket.com/developers/CLOB/websocket/user-channel

---

## 6. Authentication

### 6.1 L1 Authentication (Private Key)
- Uses wallet's private key to sign an EIP-712 message
- Used for: Creating/deriving API credentials, signing orders locally
- Private key remains under user control (non-custodial)

### 6.2 L2 Authentication (API Credentials)
- Consists of API credentials generated through L1 authentication
- Fields: `apiKey`, `secret`, `passphrase`
- Used via HMAC-SHA256 signing for CLOB API requests
- Used for: Posting orders, managing orders, checking balances, canceling orders

### 6.3 No Authentication Required
- Public market data access (orderbooks, prices, market info)
- Market channel WebSocket subscription

**Source**: https://docs.polymarket.com/developers/CLOB/authentication

---

## 7. WebSocket Authentication (User Channel)

Only `user` channel connections require authentication.

### 7.1 Required Fields

| Field | Type | Description |
|-------|------|-------------|
| `apiKey` | string | CLOB API key |
| `secret` | string | CLOB API secret |
| `passphrase` | string | CLOB API passphrase |

These fields are passed in the `auth` object when subscribing to the user channel.

```json
{
  "auth": {
    "apiKey": "YOUR_API_KEY",
    "secret": "YOUR_SECRET",
    "passphrase": "YOUR_PASSPHRASE"
  },
  "type": "USER",
  "markets": ["condition_id_1", "condition_id_2"]
}
```

**Source**: https://docs.polymarket.com/developers/CLOB/websocket/wss-auth

---

## 8. Official Rust Client

### 8.1 Package Information
- **Crate Name**: `polymarket-client-sdk`
- **Version**: `0.3.1` (as of 2025-12-31)
- **MSRV**: 1.88

### 8.2 Dependency
```toml
[dependencies]
polymarket-client-sdk = "0.3"
```

### 8.3 Features
- `ws`: WebSocket streaming for orderbook and user events
- `data`: Analytics (positions, trades, leaderboards)
- `gamma`: Market/event discovery
- `bridge`: Cross-chain deposits
- `tracing`: Structured logging

**Source**: https://github.com/Polymarket/rs-clob-client

---

## 9. Important Notes

1. **Price/Size Fields**: Many fields are returned as strings (not numbers) to preserve precision. Always parse carefully.

2. **Timestamps**: All timestamps are in Unix milliseconds.

3. **Token vs Condition IDs**:
   - `asset_id` = Token ID (specific outcome token)
   - `market` = Condition ID (the market/question)

4. **Tick Size**: Markets have variable tick sizes. When price > 0.96 or < 0.04, tick size may change.

5. **Feature Flags**: Some message types require `custom_feature_enabled: true` to receive.

---

## References

- Endpoints: https://docs.polymarket.com/quickstart/reference/endpoints
- WSS Overview: https://docs.polymarket.com/developers/CLOB/websocket/wss-overview
- Market Channel: https://docs.polymarket.com/developers/CLOB/websocket/market-channel
- User Channel: https://docs.polymarket.com/developers/CLOB/websocket/user-channel
- Authentication: https://docs.polymarket.com/developers/CLOB/authentication
- WSS Auth: https://docs.polymarket.com/developers/CLOB/websocket/wss-auth
- Official Rust Client: https://github.com/Polymarket/rs-clob-client
