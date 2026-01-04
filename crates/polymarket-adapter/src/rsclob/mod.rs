//! Official rs-clob-client wrapper backend
//!
//! This module wraps the official Polymarket Rust client:
//! https://github.com/Polymarket/rs-clob-client
//!
//! # Dependency
//! ```toml
//! [dependencies]
//! polymarket-client-sdk = "0.3"
//! ```
//!
//! # Usage
//! Enable the `rsclob` feature to use this backend:
//! ```toml
//! [dependencies]
//! polymarket-adapter = { path = "...", features = ["rsclob"] }
//! ```
//!
//! # Priority
//! For trading operations (order placement, cancellation), prefer using
//! this official client over custom implementation.

pub mod client;

pub use client::*;
