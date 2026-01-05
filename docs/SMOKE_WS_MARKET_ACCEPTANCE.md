# WebSocket Market Channel Smoke Test Acceptance Criteria

## Endpoint & Protocol

| Item | Value |
|------|-------|
| Endpoint | `wss://ws-subscriptions-clob.polymarket.com/ws/market` |
| Auth | None required |
| Keepalive | Application-level `"PING"` text every 10 seconds |
| Response | Server echoes `"PONG"` text |

## Subscribe Request Format

```json
{
  "assets_ids": ["<token_id_1>", "<token_id_2>", ...],
  "type": "market"
}
```

**Important**: Field is `assets_ids` (with extra 's'), type is lowercase `"market"`.

## Message Types

| event_type | Description | Required Fields |
|------------|-------------|-----------------|
| `price_change` | Price level update | market, timestamp, price_changes[] |
| `book` | Full orderbook snapshot | asset_id, market, timestamp, bids[], asks[] |
| `last_trade_price` | Trade execution | asset_id, market, timestamp, price, size, side |
| `best_bid_ask` | BBO update | asset_id, market, timestamp, best_bid, best_ask |
| `new_market` | Market created | (varies) |
| `market_resolved` | Market resolved | (varies) |
| `tick_size_change` | Tick size change | asset_id, market, timestamp, old_tick_size, new_tick_size |

**Note**: All timestamp fields are strings (not integers).

## PASS/FAIL Criteria

### PASS (all must be true)

| Criterion | Threshold |
|-----------|-----------|
| `reconnects` | 0 (or max 1 with auto-recovery) |
| `unknown` | 0 |
| `total` | Continuously growing |
| File lines | Continuously growing |
| RSS memory | No monotonic increase (stable) |

### FAIL (any triggers investigation)

| Criterion | Condition |
|-----------|-----------|
| `total` | No growth for 60+ seconds |
| `reconnects` | Multiple consecutive (storm) |
| `unknown` | >0 and growing |
| RSS | Monotonic increase (leak) |

## One-Line Check Commands

### Progress Check
```bash
ssh -i "<key>" ubuntu@<host> "tail -5 ~/polyrust-core/data/<logfile>.log"
```

### File Stats
```bash
ssh -i "<key>" ubuntu@<host> "f=~/polyrust-core/data/<jsonl>; ls -lh \$f; wc -l \$f"
```

### Resource Check
```bash
ssh -i "<key>" ubuntu@<host> "ps -p <PID> -o pid,etime,%cpu,%mem,rss; free -h | head -2"
```

## Verified Test Results (2026-01-05)

### 60-Minute Smoke Test
- Duration: 61 minutes (3662s)
- Total messages: 502,996
- Parsed OK: 488,044 (97.03% before fix)
- Unknown: 14,952 → **0** (after fee_rate_bps fix)
- Reconnects: **0**
- Market boundaries crossed: 4

### 15-Minute Regression Test
- Duration: 17 minutes (1020s)
- Total messages: 152,191
- Parsed OK: 152,191 (**100%**)
- Unknown: **0**
- Reconnects: **0**
- RSS: 8064 KB (stable throughout)

## Key Fixes Applied

| Issue | Root Cause | Fix |
|-------|------------|-----|
| 404 Not Found | Wrong endpoint | `/ws/` → `/ws/market` |
| No messages | Missing keepalive | Add 10s `"PING"` text |
| Invalid subscribe | Wrong field names | `asset_ids` → `assets_ids`, `MARKET` → `market` |
| 96% parse rate | timestamp type | `i64` → `String` |
| 3% Unknown | fee_rate_bps type | `Option<i64>` → `Option<String>` |
| Snapshot pollution | Top-level array | Add `SnapshotArray` variant |

## Tag

```
ws_market_smoke_pass_20260105
```
