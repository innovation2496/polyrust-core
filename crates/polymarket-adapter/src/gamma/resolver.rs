//! Market Resolver - Strict market selection for 15-minute rolling markets
//!
//! # Design Principles
//! 1. "宁可不交易，也不能选错" - Better to FREEZE than select wrong market
//! 2. Gamma API is the authority for market discovery
//! 3. CLOB API provides secondary validation (price check)
//! 4. All decisions are auditable via ResolveResult
//!
//! # Algorithm
//! 1. Generate candidate slugs based on time bucket
//! 2. Query Gamma for each slug
//! 3. Validate: clobTokenIds.len() == 2, active, time window
//! 4. Require exactly 1 valid candidate
//! 5. CLOB price check for both tokens
//! 6. Output ResolvedMarket or FREEZE

use anyhow::Result;
use chrono::{DateTime, Utc};
use tracing::{debug, info, warn};

use crate::gamma::GammaClient;
use crate::httpws::RestClient;
use crate::types::{GammaMarket, ResolveResult, ResolvedMarket, SelectionReason};

/// Supported market series
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarketSeries {
    /// BTC 15-minute up/down markets
    Btc15m,
    /// ETH 15-minute up/down markets (future)
    Eth15m,
}

impl MarketSeries {
    /// Generate slug patterns for this series
    /// Returns multiple patterns to handle format variations
    pub fn slug_patterns(&self) -> Vec<&'static str> {
        match self {
            MarketSeries::Btc15m => vec![
                "btc-updown-15m-{}",       // New format
                "btc-up-or-down-15m-{}",   // Old format
            ],
            MarketSeries::Eth15m => vec![
                "eth-updown-15m-{}",
                "eth-up-or-down-15m-{}",
            ],
        }
    }

    /// Parse from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "btc15m" | "btc-15m" | "btc_15m" => Some(MarketSeries::Btc15m),
            "eth15m" | "eth-15m" | "eth_15m" => Some(MarketSeries::Eth15m),
            _ => None,
        }
    }
}

/// Market Resolver configuration
pub struct ResolverConfig {
    /// Time bucket size in seconds (900 for 15 minutes)
    pub bucket_size_secs: i64,
    /// Tolerance for start/end time validation (seconds)
    pub time_tolerance_secs: i64,
    /// Whether to check adjacent buckets (prev/next)
    pub check_adjacent_buckets: bool,
    /// Whether to perform CLOB price validation
    pub clob_validation: bool,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            bucket_size_secs: 900,        // 15 minutes
            time_tolerance_secs: 120,     // 2 minutes tolerance
            check_adjacent_buckets: true, // Check prev/next buckets
            clob_validation: true,        // Enable CLOB price check
        }
    }
}

/// Market Resolver
/// Resolves the current active market for a given series
pub struct MarketResolver {
    gamma: GammaClient,
    clob: RestClient,
    config: ResolverConfig,
}

impl MarketResolver {
    /// Create a new resolver with default configuration
    pub fn new() -> Result<Self> {
        Ok(Self {
            gamma: GammaClient::new()?,
            clob: RestClient::new()?,
            config: ResolverConfig::default(),
        })
    }

    /// Create with custom configuration
    pub fn with_config(config: ResolverConfig) -> Result<Self> {
        Ok(Self {
            gamma: GammaClient::new()?,
            clob: RestClient::new()?,
            config,
        })
    }

    /// Resolve the current market for a series
    ///
    /// # Arguments
    /// * `series` - Market series to resolve (e.g., Btc15m)
    /// * `asof` - Reference time (typically now)
    ///
    /// # Returns
    /// * `ResolveResult::Ok(market)` - Successfully resolved
    /// * `ResolveResult::Freeze { .. }` - Resolution failed, do NOT trade
    pub async fn resolve(&self, series: &MarketSeries, asof: DateTime<Utc>) -> ResolveResult {
        let asof_ts = asof.timestamp();
        let bucket_start = (asof_ts / self.config.bucket_size_secs) * self.config.bucket_size_secs;

        info!(
            "Resolving market for {:?}, asof={}, bucket_start={}",
            series, asof, bucket_start
        );

        // Generate candidate slugs
        let mut candidate_slugs = Vec::new();
        let patterns = series.slug_patterns();

        // Current bucket
        for pattern in &patterns {
            candidate_slugs.push(pattern.replace("{}", &bucket_start.to_string()));
        }

        // Adjacent buckets (optional)
        if self.config.check_adjacent_buckets {
            let prev_bucket = bucket_start - self.config.bucket_size_secs;
            let next_bucket = bucket_start + self.config.bucket_size_secs;

            for pattern in &patterns {
                candidate_slugs.push(pattern.replace("{}", &prev_bucket.to_string()));
                candidate_slugs.push(pattern.replace("{}", &next_bucket.to_string()));
            }
        }

        debug!("Candidate slugs: {:?}", candidate_slugs);

        // Query Gamma for each candidate
        let mut valid_markets: Vec<GammaMarket> = Vec::new();
        let mut queried_slugs: Vec<String> = Vec::new();

        for slug in &candidate_slugs {
            match self.gamma.get_market_by_slug(slug).await {
                Ok(Some(market)) => {
                    queried_slugs.push(slug.clone());

                    // Validate market
                    if let Some(reason) = self.validate_market(&market, asof_ts) {
                        debug!("Market {} failed validation: {:?}", slug, reason);
                        continue;
                    }

                    info!("Valid candidate found: {}", slug);
                    valid_markets.push(market);
                }
                Ok(None) => {
                    debug!("Slug not found: {}", slug);
                }
                Err(e) => {
                    warn!("Gamma API error for slug {}: {}", slug, e);
                    // Continue trying other slugs
                }
            }
        }

        // Uniqueness check
        if valid_markets.is_empty() {
            return ResolveResult::Freeze {
                reason: SelectionReason::NoCandidates,
                message: "No valid market candidates found".to_string(),
                candidates: queried_slugs,
            };
        }

        if valid_markets.len() > 1 {
            return ResolveResult::Freeze {
                reason: SelectionReason::AmbiguousCandidates,
                message: format!("Found {} valid candidates, expected exactly 1", valid_markets.len()),
                candidates: valid_markets.iter().map(|m| m.slug.clone()).collect(),
            };
        }

        let market = valid_markets.remove(0);

        // CLOB price validation
        if self.config.clob_validation {
            for token_id in &market.clob_token_ids {
                match self.validate_clob_token(token_id).await {
                    Ok(true) => {
                        debug!("CLOB token {} validated OK", token_id);
                    }
                    Ok(false) => {
                        return ResolveResult::Freeze {
                            reason: SelectionReason::ClobPriceCheckFailed,
                            message: format!("CLOB price check failed for token {}", token_id),
                            candidates: vec![market.slug.clone()],
                        };
                    }
                    Err(e) => {
                        warn!("CLOB validation error for {}: {}", token_id, e);
                        return ResolveResult::Freeze {
                            reason: SelectionReason::ClobPriceCheckFailed,
                            message: format!("CLOB API error: {}", e),
                            candidates: vec![market.slug.clone()],
                        };
                    }
                }
            }
        }

        // Build ResolvedMarket
        let now_ms = Utc::now().timestamp_millis();

        // Convert clob_token_ids to fixed array
        let clob_token_ids: [String; 2] = match market.clob_token_ids.as_slice() {
            [a, b] => [a.clone(), b.clone()],
            _ => {
                return ResolveResult::Freeze {
                    reason: SelectionReason::ValidationFailed,
                    message: "clobTokenIds is not exactly 2 elements".to_string(),
                    candidates: vec![market.slug.clone()],
                };
            }
        };

        // Convert outcomes to fixed array
        let outcomes: [String; 2] = match market.outcomes.as_slice() {
            [a, b] => [a.clone(), b.clone()],
            [] => ["Up".to_string(), "Down".to_string()], // Default for binary
            _ => {
                return ResolveResult::Freeze {
                    reason: SelectionReason::ValidationFailed,
                    message: format!("Unexpected outcomes count: {}", market.outcomes.len()),
                    candidates: vec![market.slug.clone()],
                };
            }
        };

        let resolved = ResolvedMarket {
            gamma_market_id: market.id.clone(),
            condition_id: market.condition_id.clone(),
            clob_token_ids,
            slug: market.slug.clone(),
            question: market.question.clone(),
            start_date: market.start_date.unwrap_or_default(),
            end_date: market.end_date.unwrap_or_default(),
            selected_at_ms: now_ms,
            selection_reason: SelectionReason::UniqueMatchInWindow,
            outcomes,
        };

        info!(
            "Successfully resolved market: {} (condition_id: {})",
            resolved.slug, resolved.condition_id
        );

        ResolveResult::Ok(resolved)
    }

    /// Validate a market against selection criteria
    /// Returns Some(reason) if invalid, None if valid
    fn validate_market(&self, market: &GammaMarket, asof_ts: i64) -> Option<SelectionReason> {
        // Check binary market (2 tokens)
        if !market.is_valid_binary() {
            debug!(
                "Market {} has {} tokens, expected 2",
                market.slug,
                market.clob_token_ids.len()
            );
            return Some(SelectionReason::ValidationFailed);
        }

        // Check active and not closed
        if !market.active || market.closed {
            debug!(
                "Market {} is not active or is closed (active={}, closed={})",
                market.slug, market.active, market.closed
            );
            return Some(SelectionReason::ValidationFailed);
        }

        // Extract trading window timestamp from slug
        // Format: btc-updown-15m-{timestamp} or btc-up-or-down-15m-{timestamp}
        // Note: API's startDate is market creation time, NOT the trading window!
        let bucket_ts = self.extract_bucket_timestamp(&market.slug);
        if bucket_ts.is_none() {
            debug!("Market {} slug does not contain valid timestamp", market.slug);
            return Some(SelectionReason::ValidationFailed);
        }

        let bucket_start = bucket_ts.unwrap();
        let bucket_end = bucket_start + self.config.bucket_size_secs;

        // asof should be within [bucket_start, bucket_end + tolerance)
        // Note: We use strict start (no tolerance) to avoid selecting future buckets,
        // but allow tolerance at the end for markets that are about to close.
        // This ensures we only select ONE bucket at any given time.
        if asof_ts < bucket_start || asof_ts >= bucket_end + self.config.time_tolerance_secs {
            debug!(
                "Market {} time window mismatch: asof={} not in [{}, {})",
                market.slug, asof_ts, bucket_start, bucket_end + self.config.time_tolerance_secs
            );
            return Some(SelectionReason::ValidationFailed);
        }

        None // Valid
    }

    /// Extract bucket timestamp from slug
    /// e.g., "btc-updown-15m-1767603600" -> Some(1767603600)
    fn extract_bucket_timestamp(&self, slug: &str) -> Option<i64> {
        // Find the last segment after "-" and try to parse as timestamp
        slug.rsplit('-').next().and_then(|s| s.parse::<i64>().ok())
    }

    /// Validate a CLOB token by checking if we can get a price
    async fn validate_clob_token(&self, token_id: &str) -> Result<bool> {
        // Try to get price - if we get a response, token is valid
        match self.clob.get_price(token_id, "buy").await {
            Ok(price_data) => {
                // Check if we got a valid price field
                if price_data.get("price").is_some() {
                    Ok(true)
                } else {
                    debug!("CLOB price response missing 'price' field for {}", token_id);
                    Ok(false)
                }
            }
            Err(e) => {
                // CLOB API error
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_market_series_slug_patterns() {
        let btc = MarketSeries::Btc15m;
        let patterns = btc.slug_patterns();
        assert_eq!(patterns.len(), 2);
        assert!(patterns[0].contains("btc"));
    }

    #[test]
    fn test_market_series_from_str() {
        assert_eq!(MarketSeries::from_str("btc15m"), Some(MarketSeries::Btc15m));
        assert_eq!(MarketSeries::from_str("BTC-15M"), Some(MarketSeries::Btc15m));
        assert_eq!(MarketSeries::from_str("invalid"), None);
    }

    #[test]
    fn test_slug_generation() {
        let series = MarketSeries::Btc15m;
        let patterns = series.slug_patterns();

        // Simulate bucket_start = 1767301200 (example timestamp)
        let bucket_start = 1767301200i64;
        let slugs: Vec<String> = patterns
            .iter()
            .map(|p| p.replace("{}", &bucket_start.to_string()))
            .collect();

        assert!(slugs.contains(&"btc-updown-15m-1767301200".to_string()));
        assert!(slugs.contains(&"btc-up-or-down-15m-1767301200".to_string()));
    }
}
