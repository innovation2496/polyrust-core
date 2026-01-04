//! Protocol types for Polymarket CLOB WebSocket and REST APIs
//!
//! # Design Principles
//! 1. All numeric fields use String to preserve precision (avoid f64 parsing errors)
//! 2. Unknown message types fall back to `Unknown { raw: Value }` - never panic
//! 3. Known types with unrecognized fields use `#[serde(flatten)] extra` to preserve data
//! 4. All field names match official documentation exactly
//!
//! # Sources
//! - Market Channel: https://docs.polymarket.com/developers/CLOB/websocket/market-channel
//! - User Channel: https://docs.polymarket.com/developers/CLOB/websocket/user-channel
//! - WSS Auth: https://docs.polymarket.com/developers/CLOB/websocket/wss-auth

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

// ============================================================================
// WebSocket Subscription Messages (Outbound)
// ============================================================================

/// Channel type for WebSocket subscription
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/wss-overview
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum ChannelType {
    Market,
    User,
}

/// Authentication object for user channel
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/wss-auth
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WsAuth {
    /// CLOB API key
    pub api_key: String,
    /// CLOB API secret
    pub secret: String,
    /// CLOB API passphrase
    pub passphrase: String,
}

/// Initial subscription request
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/wss-overview
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscribeRequest {
    /// Authentication (required for user channel only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<WsAuth>,

    /// Condition IDs (for user channel)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markets: Option<Vec<String>>,

    /// Token IDs (for market channel)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_ids: Option<Vec<String>>,

    /// Channel type: "MARKET" or "USER"
    #[serde(rename = "type")]
    pub channel_type: ChannelType,

    /// Enable feature-flagged messages (best_bid_ask, new_market, market_resolved)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_feature_enabled: Option<bool>,

    /// Extra fields for forward compatibility
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl SubscribeRequest {
    /// Create a market channel subscription request
    pub fn market(asset_ids: Vec<String>, enable_features: bool) -> Self {
        Self {
            auth: None,
            markets: None,
            asset_ids: Some(asset_ids),
            channel_type: ChannelType::Market,
            custom_feature_enabled: if enable_features { Some(true) } else { None },
            extra: HashMap::new(),
        }
    }

    /// Create a user channel subscription request
    pub fn user(auth: WsAuth, markets: Vec<String>) -> Self {
        Self {
            auth: Some(auth),
            markets: Some(markets),
            asset_ids: None,
            channel_type: ChannelType::User,
            custom_feature_enabled: None,
            extra: HashMap::new(),
        }
    }
}

/// Dynamic subscription change (after initial connection)
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/wss-overview
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscriptionChange {
    /// Token IDs to add/remove
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset_ids: Option<Vec<String>>,

    /// Condition IDs to add/remove
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markets: Option<Vec<String>>,

    /// Operation: "subscribe" or "unsubscribe"
    pub operation: String,

    /// Feature toggle
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_feature_enabled: Option<bool>,

    /// Extra fields
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

// ============================================================================
// WebSocket Inbound Messages (from server)
// ============================================================================

/// Inbound WebSocket message - parsed with fallback to Unknown
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WsInboundMessage {
    /// Successfully parsed market channel message
    Market(MarketMessage),
    /// Successfully parsed user channel message
    User(UserMessage),
    /// Unknown or unparseable message - raw JSON preserved
    Unknown(UnknownMessage),
}

/// Unknown message container - preserves raw JSON
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnknownMessage {
    pub raw: Value,
}

// ============================================================================
// Market Channel Messages
// Source: https://docs.polymarket.com/developers/CLOB/websocket/market-channel
// ============================================================================

/// Market channel message types
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum MarketMessage {
    /// Orderbook snapshot
    Book(BookMessage),
    /// Price level changes
    PriceChange(PriceChangeMessage),
    /// Tick size change
    TickSizeChange(TickSizeChangeMessage),
    /// Last trade price
    LastTradePrice(LastTradePriceMessage),
    /// Best bid/ask update (feature-flagged)
    BestBidAsk(BestBidAskMessage),
    /// New market created (feature-flagged)
    NewMarket(NewMarketMessage),
    /// Market resolved (feature-flagged)
    MarketResolved(MarketResolvedMessage),
}

/// Order summary (price level)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderSummary {
    pub price: String,
    pub size: String,
    /// Extra fields
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Book message - full orderbook snapshot
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/market-channel
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BookMessage {
    /// Token identifier
    pub asset_id: String,
    /// Condition ID
    pub market: String,
    /// Unix timestamp in milliseconds
    pub timestamp: i64,
    /// Orderbook content hash
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    /// Bid levels (price, size pairs)
    #[serde(default)]
    pub buys: Vec<OrderSummary>,
    /// Ask levels (price, size pairs)
    #[serde(default)]
    pub sells: Vec<OrderSummary>,
    /// Extra fields
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Price change entry
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PriceChangeEntry {
    pub asset_id: String,
    pub price: String,
    pub size: String,
    pub side: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_bid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_ask: Option<String>,
    /// Extra fields
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Price change message
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/market-channel
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PriceChangeMessage {
    /// Condition ID
    pub market: String,
    /// Unix timestamp in milliseconds
    pub timestamp: i64,
    /// Array of price changes
    #[serde(default)]
    pub price_changes: Vec<PriceChangeEntry>,
    /// Extra fields
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Tick size change message
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/market-channel
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TickSizeChangeMessage {
    /// Token identifier
    pub asset_id: String,
    /// Condition ID
    pub market: String,
    /// Unix timestamp in milliseconds
    pub timestamp: i64,
    /// Previous tick size
    pub old_tick_size: String,
    /// New tick size
    pub new_tick_size: String,
    /// Side indicator
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    /// Extra fields
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Last trade price message
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/market-channel
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LastTradePriceMessage {
    /// Token identifier
    pub asset_id: String,
    /// Condition ID
    pub market: String,
    /// Unix timestamp in milliseconds
    pub timestamp: i64,
    /// Trade price
    pub price: String,
    /// Trade size
    pub size: String,
    /// Trade side (BUY/SELL)
    pub side: String,
    /// Fee rate in basis points
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_rate_bps: Option<i64>,
    /// Extra fields
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Best bid/ask message (feature-flagged)
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/market-channel
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BestBidAskMessage {
    /// Token identifier
    pub asset_id: String,
    /// Condition ID
    pub market: String,
    /// Unix timestamp in milliseconds
    pub timestamp: i64,
    /// Best bid price
    pub best_bid: String,
    /// Best ask price
    pub best_ask: String,
    /// Spread
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spread: Option<String>,
    /// Extra fields
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// New market message (feature-flagged)
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/market-channel
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewMarketMessage {
    /// All fields preserved as raw JSON since schema is not fully documented
    #[serde(flatten)]
    pub data: Map<String, Value>,
}

/// Market resolved message (feature-flagged)
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/market-channel
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarketResolvedMessage {
    /// All fields preserved as raw JSON since schema is not fully documented
    #[serde(flatten)]
    pub data: Map<String, Value>,
}

// ============================================================================
// User Channel Messages
// Source: https://docs.polymarket.com/developers/CLOB/websocket/user-channel
// ============================================================================

/// User channel message types
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum UserMessage {
    /// Trade event
    Trade(TradeMessage),
    /// Order event
    Order(OrderMessage),
}

/// Maker order details in a trade
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MakerOrderDetail {
    #[serde(flatten)]
    pub data: Map<String, Value>,
}

/// Trade message
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/user-channel
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TradeMessage {
    /// Trade identifier
    pub id: String,
    /// Token identifier
    pub asset_id: String,
    /// Condition ID
    pub market: String,
    /// When trade was matched
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matchtime: Option<String>,
    /// Trade outcome
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Trade price
    pub price: String,
    /// Trade side (BUY/SELL)
    pub side: String,
    /// Trade quantity
    pub size: String,
    /// Current trade status (MATCHED, MINED, CONFIRMED, RETRYING, FAILED)
    pub status: String,
    /// Details of maker orders involved
    #[serde(default)]
    pub maker_orders: Vec<MakerOrderDetail>,
    /// Associated taker order ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub taker_order_id: Option<String>,
    /// Owner identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    /// Extra fields
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Order message
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/user-channel
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OrderMessage {
    /// Order identifier
    pub id: String,
    /// Token identifier
    pub asset_id: String,
    /// Condition ID
    pub market: String,
    /// Initial order size
    pub original_size: String,
    /// Market outcome
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Order price
    pub price: String,
    /// Order side (BUY/SELL)
    pub side: String,
    /// Matched quantity
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_matched: Option<String>,
    /// Event type (PLACEMENT/UPDATE/CANCELLATION)
    #[serde(rename = "type")]
    pub order_type: String,
    /// Owner identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    /// Extra fields
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

// ============================================================================
// Helper Functions
// ============================================================================

impl WsInboundMessage {
    /// Try to parse a JSON string into a WsInboundMessage
    /// Never panics - falls back to Unknown on parse failure
    pub fn parse(json_str: &str) -> Self {
        // First try to parse as Value to preserve raw JSON
        let raw: Value = match serde_json::from_str(json_str) {
            Ok(v) => v,
            Err(_) => {
                // Even JSON parsing failed - store as string in Value
                return WsInboundMessage::Unknown(UnknownMessage {
                    raw: Value::String(json_str.to_string()),
                });
            }
        };

        // Try to determine message type from event_type field
        if let Some(event_type) = raw.get("event_type").and_then(|v| v.as_str()) {
            // Try market channel messages
            match event_type {
                "book" | "price_change" | "tick_size_change" | "last_trade_price"
                | "best_bid_ask" | "new_market" | "market_resolved" => {
                    if let Ok(msg) = serde_json::from_value::<MarketMessage>(raw.clone()) {
                        return WsInboundMessage::Market(msg);
                    }
                }
                "trade" | "order" => {
                    if let Ok(msg) = serde_json::from_value::<UserMessage>(raw.clone()) {
                        return WsInboundMessage::User(msg);
                    }
                }
                _ => {}
            }
        }

        // Fallback to Unknown
        WsInboundMessage::Unknown(UnknownMessage { raw })
    }

    /// Get the event type string if available
    pub fn event_type(&self) -> Option<&str> {
        match self {
            WsInboundMessage::Market(m) => Some(match m {
                MarketMessage::Book(_) => "book",
                MarketMessage::PriceChange(_) => "price_change",
                MarketMessage::TickSizeChange(_) => "tick_size_change",
                MarketMessage::LastTradePrice(_) => "last_trade_price",
                MarketMessage::BestBidAsk(_) => "best_bid_ask",
                MarketMessage::NewMarket(_) => "new_market",
                MarketMessage::MarketResolved(_) => "market_resolved",
            }),
            WsInboundMessage::User(u) => Some(match u {
                UserMessage::Trade(_) => "trade",
                UserMessage::Order(_) => "order",
            }),
            WsInboundMessage::Unknown(u) => u.raw.get("event_type").and_then(|v| v.as_str()),
        }
    }

    /// Check if this is an unknown message type
    pub fn is_unknown(&self) -> bool {
        matches!(self, WsInboundMessage::Unknown(_))
    }
}

// ============================================================================
// Statistics Tracking
// ============================================================================

/// Statistics for message parsing
#[derive(Clone, Debug, Default)]
pub struct MessageStats {
    pub total_messages: u64,
    pub parsed_ok: u64,
    pub unknown_type_count: u64,
    pub parse_error_count: u64,
    pub type_counts: HashMap<String, u64>,
    pub last_message_type: Option<String>,
}

impl MessageStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, msg: &WsInboundMessage) {
        self.total_messages += 1;

        match msg {
            WsInboundMessage::Unknown(_) => {
                self.unknown_type_count += 1;
            }
            _ => {
                self.parsed_ok += 1;
            }
        }

        if let Some(event_type) = msg.event_type() {
            *self.type_counts.entry(event_type.to_string()).or_insert(0) += 1;
            self.last_message_type = Some(event_type.to_string());
        } else {
            *self.type_counts.entry("_no_type".to_string()).or_insert(0) += 1;
            self.last_message_type = Some("_no_type".to_string());
        }
    }

    pub fn record_parse_error(&mut self) {
        self.total_messages += 1;
        self.parse_error_count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_book_message() {
        let json = r#"{
            "event_type": "book",
            "asset_id": "token123",
            "market": "condition456",
            "timestamp": 1704067200000,
            "hash": "abc123",
            "buys": [{"price": "0.50", "size": "100"}],
            "sells": [{"price": "0.51", "size": "200"}]
        }"#;

        let msg = WsInboundMessage::parse(json);
        assert!(!msg.is_unknown());
        assert_eq!(msg.event_type(), Some("book"));
    }

    #[test]
    fn test_parse_unknown_message() {
        let json = r#"{"event_type": "some_future_type", "data": "test"}"#;
        let msg = WsInboundMessage::parse(json);
        assert!(msg.is_unknown());
        assert_eq!(msg.event_type(), Some("some_future_type"));
    }

    #[test]
    fn test_parse_invalid_json() {
        let msg = WsInboundMessage::parse("not valid json");
        assert!(msg.is_unknown());
    }

    #[test]
    fn test_subscribe_request_market() {
        let req = SubscribeRequest::market(vec!["asset1".to_string()], true);
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("MARKET"));
        assert!(json.contains("asset1"));
        assert!(json.contains("custom_feature_enabled"));
    }

    #[test]
    fn test_subscribe_request_user() {
        let auth = WsAuth {
            api_key: "key".to_string(),
            secret: "secret".to_string(),
            passphrase: "pass".to_string(),
        };
        let req = SubscribeRequest::user(auth, vec!["market1".to_string()]);
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("USER"));
        assert!(json.contains("apiKey"));
    }
}
