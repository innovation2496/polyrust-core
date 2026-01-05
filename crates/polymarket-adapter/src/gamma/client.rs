//! Gamma API REST client
//!
//! Base URL: https://gamma-api.polymarket.com
//!
//! # Endpoints
//! - GET /markets - List markets with filters
//! - GET /markets/{id} - Get market by ID
//! - GET /markets/slug/{slug} - Get market by slug (most reliable for exact match)
//!
//! # Source
//! - https://docs.polymarket.com/developers/gamma-markets-api/markets

use anyhow::{Context, Result};
use reqwest::Client;
use tracing::{debug, info};

use crate::types::GammaMarket;
use crate::GAMMA_API_BASE;

/// Gamma API REST client
#[derive(Clone)]
pub struct GammaClient {
    client: Client,
    base_url: String,
}

impl GammaClient {
    /// Create a new Gamma client with default base URL
    pub fn new() -> Result<Self> {
        Self::with_base_url(GAMMA_API_BASE)
    }

    /// Create a new Gamma client with custom base URL
    pub fn with_base_url(base_url: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self { client, base_url: base_url.trim_end_matches('/').to_string() })
    }

    /// GET /markets/slug/{slug} - Get market by slug (most reliable)
    /// Returns None if 404 or empty array, errors on other failures
    ///
    /// Note: The Gamma API returns an array even for slug queries, so we take the first element.
    pub async fn get_market_by_slug(&self, slug: &str) -> Result<Option<GammaMarket>> {
        let url = format!("{}/markets/slug/{}", self.base_url, slug);
        debug!("GET {}", url);

        let response = self.client.get(&url).send().await.context("HTTP request failed")?;

        let status = response.status();

        // 404 = market not found (normal case for wrong slug)
        if status == reqwest::StatusCode::NOT_FOUND {
            debug!("Market not found for slug: {}", slug);
            return Ok(None);
        }

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("HTTP {} for {}: {}", status, url, body);
        }

        // API returns an array, take first element
        let markets: Vec<GammaMarket> = response.json().await.context("Failed to parse GammaMarket array")?;
        Ok(markets.into_iter().next())
    }

    /// GET /markets?slug={slug} - Fallback query by slug
    /// Returns empty vec if no matches
    pub async fn query_markets_by_slug(&self, slug: &str) -> Result<Vec<GammaMarket>> {
        let url = format!("{}/markets?slug={}", self.base_url, slug);
        debug!("GET {}", url);

        let response = self.client.get(&url).send().await.context("HTTP request failed")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("HTTP {} for {}: {}", status, url, body);
        }

        let markets: Vec<GammaMarket> =
            response.json().await.context("Failed to parse market list")?;
        Ok(markets)
    }

    /// GET /markets/{id} - Get market by ID
    pub async fn get_market_by_id(&self, id: &str) -> Result<Option<GammaMarket>> {
        let url = format!("{}/markets/{}", self.base_url, id);
        debug!("GET {}", url);

        let response = self.client.get(&url).send().await.context("HTTP request failed")?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("HTTP {} for {}: {}", status, url, body);
        }

        let market: GammaMarket = response.json().await.context("Failed to parse GammaMarket")?;
        Ok(Some(market))
    }

    /// List active markets with filters
    /// Useful for discovery, but less reliable for exact matching
    pub async fn list_markets(
        &self,
        active: bool,
        closed: bool,
        limit: u32,
    ) -> Result<Vec<GammaMarket>> {
        let url = format!(
            "{}/markets?active={}&closed={}&limit={}",
            self.base_url, active, closed, limit
        );
        debug!("GET {}", url);

        let response = self.client.get(&url).send().await.context("HTTP request failed")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("HTTP {} for {}: {}", status, url, body);
        }

        let markets: Vec<GammaMarket> =
            response.json().await.context("Failed to parse market list")?;
        Ok(markets)
    }

    /// Test connectivity to Gamma API
    pub async fn test_connectivity(&self) -> Result<()> {
        info!("Testing connectivity to {}", self.base_url);

        let url = format!("{}/markets?limit=1", self.base_url);
        let response = self.client.get(&url).send().await.context("Connection test failed")?;

        let status = response.status();
        info!("Gamma connectivity test: HTTP {}", status);

        if !status.is_success() {
            anyhow::bail!("Gamma API returned HTTP {}", status);
        }

        Ok(())
    }
}

impl Default for GammaClient {
    fn default() -> Self {
        Self::new().expect("Failed to create default GammaClient")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = GammaClient::new();
        assert!(client.is_ok());
    }

    #[test]
    fn test_custom_base_url() {
        let client = GammaClient::with_base_url("https://example.com/").unwrap();
        assert_eq!(client.base_url, "https://example.com");
    }
}
