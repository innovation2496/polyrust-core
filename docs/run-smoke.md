# Polymarket Smoke Test Guide

## Prerequisites

1. **Rust toolchain**: Install via [rustup](https://rustup.rs/)
   ```bash
   rustup install stable
   ```

2. **Build the project**:
   ```bash
   cd polyrust
   cargo build --release
   ```

## Getting an Asset ID

To run the smoke test, you need an `asset_id` (token_id). You can find active markets at:
- https://polymarket.com (browse markets)
- Or use the Gamma API to discover markets

Example asset IDs for testing (verify they're still active):
- Check https://gamma-api.polymarket.com for current markets

## Commands

### 1. Market Channel Smoke Test (No Auth Required)

Connect to the market channel and collect orderbook/price messages.

```bash
# Basic usage
cargo run -p pm-smoke-cli -- market --asset-id <ASSET_ID>

# With custom output and limit
cargo run -p pm-smoke-cli -- market \
    --asset-id <ASSET_ID> \
    --out data/ws_market_raw.jsonl \
    --limit 500

# Debug logging
cargo run -p pm-smoke-cli -- --log-level debug market --asset-id <ASSET_ID>
```

**Arguments:**
| Argument | Default | Description |
|----------|---------|-------------|
| `--asset-id` | Required | Token ID to subscribe to |
| `--out` | `data/ws_market_raw.jsonl` | Output file path |
| `--limit` | `500` | Max messages (0 = unlimited) |
| `--enable-features` | `true` | Enable best_bid_ask, new_market, etc. |

**Output:**
- Raw JSONL file with one message per line
- Statistics printed to console

### 2. User Channel Smoke Test (Auth Required)

Connect to the user channel to receive order/trade updates.

**Required Environment Variables:**
```bash
export POLY_API_KEY="your_api_key"
export POLY_API_SECRET="your_secret"
export POLY_API_PASSPHRASE="your_passphrase"
```

```bash
# Run user channel smoke test
cargo run -p pm-smoke-cli -- user \
    --market-id <CONDITION_ID> \
    --out data/ws_user_raw.jsonl \
    --limit 200
```

**Arguments:**
| Argument | Default | Description |
|----------|---------|-------------|
| `--market-id` | Required | Condition ID to subscribe to |
| `--out` | `data/ws_user_raw.jsonl` | Output file path |
| `--limit` | `200` | Max messages (0 = unlimited) |

### 3. REST API Smoke Test

Test REST API connectivity and basic endpoints.

```bash
# Basic connectivity test
cargo run -p pm-smoke-cli -- rest

# With asset-id to test book/price endpoints
cargo run -p pm-smoke-cli -- rest --asset-id <ASSET_ID>
```

**Arguments:**
| Argument | Default | Description |
|----------|---------|-------------|
| `--asset-id` | Optional | Token ID for book/price queries |

## Expected Output

### Market Channel Success
```
=== Market Channel Smoke Test ===
Endpoint: wss://ws-subscriptions-clob.polymarket.com/ws/
Asset ID: <your_asset_id>
Output: data/ws_market_raw.jsonl
Limit: 500 (0 = unlimited)
Features enabled: true
Press Ctrl+C to stop

2024-01-04T12:00:00Z INFO Starting market channel client
2024-01-04T12:00:01Z INFO Connected and subscribed to market channel
2024-01-04T12:00:10Z DEBUG Collected 100 messages, 0 unknown

=== Summary ===
Total messages: 500
Parsed OK: 498
Unknown type count: 2
Parse errors: 0
Last message type: Some("price_change")

Message type distribution:
  price_change: 350
  book: 100
  last_trade_price: 48
  best_bid_ask: 2

Output written to: data/ws_market_raw.jsonl
```

### REST Success
```
=== REST API Smoke Test ===
Base URL: https://clob.polymarket.com

Testing connectivity...
Connectivity: OK

Fetching orderbook for asset: <your_asset_id>
Book response:
  Bids: 15 levels
    0: {"price": "0.45", "size": "1000"}
    1: {"price": "0.44", "size": "500"}
  Asks: 12 levels
    0: {"price": "0.46", "size": "800"}
    1: {"price": "0.47", "size": "300"}

Fetching midpoint...
Midpoint: {"mid": "0.455"}

REST smoke test complete
```

## Troubleshooting

### Connection Errors
- Check your internet connection
- Verify the endpoint is accessible: `curl https://clob.polymarket.com`
- Check firewall/proxy settings

### Authentication Errors (User Channel)
- Verify environment variables are set correctly
- Ensure credentials are valid (not expired)
- Check that API key has appropriate permissions

### Unknown Message Types
This is expected and handled gracefully. The system logs unknown types for analysis but continues running.

### No Messages Received
- Verify the asset_id/market_id is valid and active
- Some markets may have low activity
- Try a more popular market

## Output Format

### JSONL File
Each line is a complete JSON message from the WebSocket:
```json
{"event_type":"book","asset_id":"...","market":"...","timestamp":1704067200000,"buys":[...],"sells":[...]}
{"event_type":"price_change","market":"...","timestamp":1704067201000,"price_changes":[...]}
```

### Statistics
The CLI prints statistics including:
- Total messages received
- Successfully parsed messages
- Unknown message type count
- Distribution by message type

## Notes

1. **Rate Limits**: The Polymarket API may have rate limits. If you experience connection drops, try reducing request frequency.

2. **Market Hours**: Some markets may be more active during certain hours.

3. **Data Storage**: Large JSONL files can be compressed:
   ```bash
   gzip data/ws_market_raw.jsonl
   ```

4. **Interruption**: Press Ctrl+C to gracefully stop collection. The file will be properly flushed.
