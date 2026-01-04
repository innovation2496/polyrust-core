//! REST client for Polymarket CLOB API
//!
//! Base URL: https://clob.polymarket.com
//!
//! # Public Endpoints (no auth required)
//! - GET /book - Get orderbook for a token
//! - GET /price - Get price for a token
//! - GET /markets - Get market info
//!
//! # Source
//! - Endpoints: https://docs.polymarket.com/quickstart/reference/endpoints

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;
use tracing::{debug, info};

use crate::CLOB_REST_BASE;

/// REST client for CLOB API
#[derive(Clone)]
pub struct RestClient {
    client: Client,
    base_url: String,
}

impl RestClient {
    /// Create a new REST client with default base URL
    pub fn new() -> Result<Self> {
        Self::with_base_url(CLOB_REST_BASE)
    }

    /// Create a new REST client with custom base URL
    pub fn with_base_url(base_url: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self { client, base_url: base_url.trim_end_matches('/').to_string() })
    }

    /// GET request returning raw JSON
    pub async fn get_raw(&self, path: &str) -> Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        debug!("GET {}", url);

        let response = self.client.get(&url).send().await.context("HTTP request failed")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("HTTP {} for {}: {}", status, url, body);
        }

        let json: Value = response.json().await.context("Failed to parse JSON")?;
        Ok(json)
    }

    /// Get orderbook for a token (asset_id)
    ///
    /// Endpoint: GET /book?token_id={asset_id}
    pub async fn get_book(&self, asset_id: &str) -> Result<Value> {
        let path = format!("/book?token_id={}", asset_id);
        self.get_raw(&path).await
    }

    /// Get price for a token
    ///
    /// Endpoint: GET /price?token_id={asset_id}&side={side}
    pub async fn get_price(&self, asset_id: &str, side: &str) -> Result<Value> {
        let path = format!("/price?token_id={}&side={}", asset_id, side);
        self.get_raw(&path).await
    }

    /// Get midpoint for a token
    ///
    /// Endpoint: GET /midpoint?token_id={asset_id}
    pub async fn get_midpoint(&self, asset_id: &str) -> Result<Value> {
        let path = format!("/midpoint?token_id={}", asset_id);
        self.get_raw(&path).await
    }

    /// Get spread for a token
    ///
    /// Endpoint: GET /spread?token_id={asset_id}
    pub async fn get_spread(&self, asset_id: &str) -> Result<Value> {
        let path = format!("/spread?token_id={}", asset_id);
        self.get_raw(&path).await
    }

    /// Get market info by condition_id
    ///
    /// Endpoint: GET /markets/{condition_id}
    pub async fn get_market(&self, condition_id: &str) -> Result<Value> {
        let path = format!("/markets/{}", condition_id);
        self.get_raw(&path).await
    }

    /// Get tick size for a token
    ///
    /// Endpoint: GET /tick-size?token_id={asset_id}
    pub async fn get_tick_size(&self, asset_id: &str) -> Result<Value> {
        let path = format!("/tick-size?token_id={}", asset_id);
        self.get_raw(&path).await
    }

    /// Simple connectivity test - try to hit a public endpoint
    pub async fn test_connectivity(&self) -> Result<()> {
        info!("Testing connectivity to {}", self.base_url);

        // Try to get server time or any simple endpoint
        // If no dedicated health endpoint, we'll just verify we can connect
        let url = format!("{}/", self.base_url);

        let response = self.client.get(&url).send().await.context("Connection test failed")?;

        let status = response.status();
        info!("Connectivity test: HTTP {}", status);

        // Even a 404 means we connected successfully
        Ok(())
    }
}

impl Default for RestClient {
    fn default() -> Self {
        Self::new().expect("Failed to create default RestClient")
    }
}

/// Smoke test helper - runs basic connectivity verification
pub async fn smoke_test_rest() -> Result<()> {
    let client = RestClient::new()?;

    info!("=== REST Smoke Test ===");
    info!("Base URL: {}", CLOB_REST_BASE);

    // Test connectivity
    client.test_connectivity().await?;
    info!("Connectivity: OK");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = RestClient::new();
        assert!(client.is_ok());
    }

    #[test]
    fn test_custom_base_url() {
        let client = RestClient::with_base_url("https://example.com/").unwrap();
        assert_eq!(client.base_url, "https://example.com");
    }
}
