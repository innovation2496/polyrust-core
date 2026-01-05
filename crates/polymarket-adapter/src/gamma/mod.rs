//! Gamma API client and Market Resolver
//!
//! # Components
//! - `GammaClient`: REST client for Gamma API (market discovery)
//! - `MarketResolver`: Resolves current 15-minute market with strict validation
//!
//! # Source
//! - Gamma Structure: https://docs.polymarket.com/developers/gamma-markets-api/gamma-structure
//! - Gamma Endpoints: https://docs.polymarket.com/developers/gamma-markets-api/markets

mod client;
pub mod resolver;

pub use client::GammaClient;
pub use resolver::{MarketResolver, MarketSeries, ResolverConfig};
