//! HTTP/WebSocket backend implementation
//!
//! Custom implementation using reqwest + tokio-tungstenite.
//! This backend provides full control over connection handling,
//! reconnection logic, and message parsing.

pub mod auth;
pub mod rest;
pub mod ws_market;
pub mod ws_user;

pub use auth::*;
pub use rest::*;
pub use ws_market::*;
pub use ws_user::*;
