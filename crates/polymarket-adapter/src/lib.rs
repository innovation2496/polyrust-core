//! Polymarket CLOB Adapter
//!
//! Dual backend support:
//! - `httpws`: Custom REST + WebSocket implementation
//! - `rsclob`: Official rs-clob-client wrapper (requires feature flag)
//! - `gamma`: Gamma API client for market discovery and resolution
//!
//! # Official Documentation
//! - Endpoints: https://docs.polymarket.com/quickstart/reference/endpoints
//! - WSS Overview: https://docs.polymarket.com/developers/CLOB/websocket/wss-overview
//! - Market Channel: https://docs.polymarket.com/developers/CLOB/websocket/market-channel
//! - User Channel: https://docs.polymarket.com/developers/CLOB/websocket/user-channel
//! - Authentication: https://docs.polymarket.com/developers/CLOB/authentication
//! - Gamma Structure: https://docs.polymarket.com/developers/gamma-markets-api/gamma-structure

pub mod types;

#[cfg(feature = "httpws")]
pub mod httpws;

#[cfg(feature = "rsclob")]
pub mod rsclob;

#[cfg(feature = "httpws")]
pub mod gamma;

pub use types::*;

/// Official CLOB REST API base URL
/// Source: https://docs.polymarket.com/quickstart/reference/endpoints
pub const CLOB_REST_BASE: &str = "https://clob.polymarket.com";

/// Official Gamma API base URL (market discovery)
/// Source: https://docs.polymarket.com/quickstart/reference/endpoints
pub const GAMMA_API_BASE: &str = "https://gamma-api.polymarket.com";

/// Official CLOB WebSocket endpoint for Market Channel
/// Source: https://docs.polymarket.com/developers/CLOB/websocket/market-channel
/// Note: Use /ws/market for market channel, /ws/user for user channel
pub const CLOB_WSS_ENDPOINT: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";

/// Official Real-Time Data Stream endpoint
/// Source: https://docs.polymarket.com/quickstart/reference/endpoints
pub const RTDS_WSS_ENDPOINT: &str = "wss://ws-live-data.polymarket.com";
