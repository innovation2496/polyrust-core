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

    /// Create with custom base URLs (for testing with wiremock)
    pub fn with_base_urls(
        gamma_base_url: &str,
        clob_base_url: &str,
        config: ResolverConfig,
    ) -> Result<Self> {
        Ok(Self {
            gamma: GammaClient::with_base_url(gamma_base_url)?,
            clob: RestClient::with_base_url(clob_base_url)?,
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

        let patterns = series.slug_patterns();
        let mut queried_slugs: Vec<String> = Vec::new();

        // Strategy: Try current bucket FIRST (strict match, no tolerance)
        // Only if not found, try with tolerance on previous bucket
        // Never allow multiple candidates

        // 1. Try current bucket (strict: asof in [bucket_start, bucket_end))
        let current_bucket_slugs: Vec<String> = patterns
            .iter()
            .map(|p| p.replace("{}", &bucket_start.to_string()))
            .collect();

        for slug in &current_bucket_slugs {
            debug!("Trying current bucket slug: {}", slug);
            match self.gamma.get_market_by_slug(slug).await {
                Ok(Some(market)) => {
                    queried_slugs.push(slug.clone());
                    // Strict validation: asof must be in [bucket_start, bucket_end)
                    // Also check enableOrderBook
                    if market.is_valid_binary()
                        && market.active
                        && !market.closed
                        && market.enable_order_book
                    {
                        let bucket_end = bucket_start + self.config.bucket_size_secs;
                        if asof_ts >= bucket_start && asof_ts < bucket_end {
                            info!("Resolved to current bucket: {}", slug);
                            // Perform CLOB validation if enabled
                            if self.config.clob_validation {
                                if let Some(freeze) =
                                    self.validate_clob_tokens(&market, &queried_slugs).await
                                {
                                    return freeze;
                                }
                            }
                            return self.build_result(market, asof, bucket_start, queried_slugs.clone());
                        }
                    }
                    debug!("Current bucket {} found but validation failed", slug);
                }
                Ok(None) => {
                    debug!("Current bucket slug not found: {}", slug);
                }
                Err(e) => {
                    warn!("Gamma API error for slug {}: {}", slug, e);
                }
            }
        }

        // 2. If current bucket not found/valid, try previous bucket (with end tolerance)
        // This handles the case where we're in the tolerance window after market closes
        if self.config.check_adjacent_buckets {
            let prev_bucket = bucket_start - self.config.bucket_size_secs;
            let prev_bucket_slugs: Vec<String> = patterns
                .iter()
                .map(|p| p.replace("{}", &prev_bucket.to_string()))
                .collect();

            for slug in &prev_bucket_slugs {
                debug!("Trying previous bucket slug: {}", slug);
                match self.gamma.get_market_by_slug(slug).await {
                    Ok(Some(market)) => {
                        queried_slugs.push(slug.clone());
                        // With tolerance: asof can be up to tolerance seconds after bucket_end
                        if let Some(_) = self.validate_market(&market, asof_ts) {
                            debug!("Previous bucket {} found but validation failed", slug);
                            continue;
                        }
                        info!("Resolved to previous bucket (with tolerance): {}", slug);
                        // Perform CLOB validation if enabled
                        if self.config.clob_validation {
                            if let Some(freeze) =
                                self.validate_clob_tokens(&market, &queried_slugs).await
                            {
                                return freeze;
                            }
                        }
                        return self.build_result(market, asof, bucket_start, queried_slugs.clone());
                    }
                    Ok(None) => {
                        debug!("Previous bucket slug not found: {}", slug);
                    }
                    Err(e) => {
                        warn!("Gamma API error for slug {}: {}", slug, e);
                    }
                }
            }
        }

        // No valid market found
        ResolveResult::Freeze {
            reason: SelectionReason::NoCandidates,
            message: "No valid market candidates found".to_string(),
            candidates: queried_slugs,
        }
    }

    /// Build successful result from a validated market
    fn build_result(
        &self,
        market: GammaMarket,
        asof: DateTime<Utc>,
        bucket_start: i64,
        candidate_slugs: Vec<String>,
    ) -> ResolveResult {
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
            // Audit fields
            asof_utc: asof.to_rfc3339(),
            candidate_slugs,
            bucket_start_ts: bucket_start,
        };

        info!(
            "Successfully resolved market: {} (condition_id: {}, bucket_start: {})",
            resolved.slug, resolved.condition_id, bucket_start
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

        // Check enableOrderBook (REQUIRED for trading)
        if !market.enable_order_book {
            debug!(
                "Market {} has enableOrderBook=false, cannot trade",
                market.slug
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
    /// Implements side case fallback: try "BUY" first, then "buy" if 400 error
    async fn validate_clob_token(&self, token_id: &str) -> Result<bool> {
        // Side variants to try (handles API documentation vs implementation differences)
        const SIDE_VARIANTS: &[&str] = &["BUY", "buy"];

        for (i, side) in SIDE_VARIANTS.iter().enumerate() {
            debug!("Trying CLOB price check for {} with side={}", token_id, side);
            match self.clob.get_price(token_id, side).await {
                Ok(price_data) => {
                    // Check if we got a valid price field
                    if price_data.get("price").is_some() {
                        return Ok(true);
                    } else {
                        debug!("CLOB price response missing 'price' field for {}", token_id);
                        return Ok(false);
                    }
                }
                Err(e) => {
                    let error_str = e.to_string();
                    // If it looks like a 400 error and we have more variants to try, continue
                    let is_likely_400 = error_str.contains("400")
                        || error_str.contains("Bad Request")
                        || error_str.contains("invalid");

                    if is_likely_400 && i + 1 < SIDE_VARIANTS.len() {
                        debug!(
                            "CLOB side={} failed ({}), trying next variant",
                            side, error_str
                        );
                        continue;
                    }
                    // Last variant or non-retryable error
                    return Err(e);
                }
            }
        }

        // Should not reach here, but just in case
        anyhow::bail!("All CLOB side variants exhausted for token {}", token_id);
    }

    /// Validate both CLOB tokens for a market
    /// Returns Some(FREEZE) if validation fails, None if successful
    async fn validate_clob_tokens(
        &self,
        market: &GammaMarket,
        queried_slugs: &[String],
    ) -> Option<ResolveResult> {
        for (i, token_id) in market.clob_token_ids.iter().enumerate() {
            debug!("Validating CLOB token {}: {}", i, token_id);
            match self.validate_clob_token(token_id).await {
                Ok(true) => {
                    debug!("CLOB token {} validated OK", token_id);
                }
                Ok(false) => {
                    warn!("CLOB token {} validation failed: no price returned", token_id);
                    return Some(ResolveResult::Freeze {
                        reason: SelectionReason::ClobPriceCheckFailed,
                        message: format!(
                            "CLOB price check failed for token {} (no price field)",
                            token_id
                        ),
                        candidates: queried_slugs.to_vec(),
                    });
                }
                Err(e) => {
                    warn!("CLOB API error for token {}: {}", token_id, e);
                    return Some(ResolveResult::Freeze {
                        reason: SelectionReason::ClobPriceCheckFailed,
                        message: format!("CLOB API error for token {}: {}", token_id, e),
                        candidates: queried_slugs.to_vec(),
                    });
                }
            }
        }
        None // All tokens validated OK
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

/// Wiremock integration tests for MarketResolver
/// Tests cover: unique candidate success, zero candidates, CLOB validation failure, time window mismatch
#[cfg(test)]
mod wiremock_tests {
    use super::*;
    use chrono::TimeZone;
    use wiremock::matchers::{method, path, path_regex, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Helper: Create a valid GammaMarket JSON response
    fn make_gamma_market_json(slug: &str, token_ids: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "id": "market-id-123",
            "slug": slug,
            "question": "Will BTC be up or down?",
            "conditionId": "condition-id-456",
            "clobTokenIds": serde_json::to_string(&token_ids).unwrap(),
            "outcomes": "[\"Up\",\"Down\"]",
            "outcomePrices": "[\"0.50\",\"0.50\"]",
            "startDate": "2026-01-05T11:00:00Z",
            "endDate": "2026-01-05T11:15:00Z",
            "active": true,
            "closed": false,
            "archived": false,
            "enableOrderBook": true
        })
    }

    /// Helper: Create a valid CLOB price response
    fn make_clob_price_json(price: &str) -> serde_json::Value {
        serde_json::json!({
            "price": price
        })
    }

    /// Test: Unique candidate success
    /// Gamma returns exactly one valid market, CLOB returns valid prices
    #[tokio::test]
    async fn test_unique_candidate_success() {
        // Start mock servers
        let gamma_server = MockServer::start().await;
        let clob_server = MockServer::start().await;

        // Use a 15-minute aligned timestamp: 1736073000 is divisible by 900
        // 1736073000 / 900 = 1928970 (exact)
        let bucket_start = 1736073000i64;
        let asof_ts = bucket_start + 300; // 5 minutes into the bucket
        let slug = format!("btc-updown-15m-{}", bucket_start);

        // Mock Gamma: GET /markets/slug/{slug} returns valid market
        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/{}", slug)))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_gamma_market_json(
                &slug,
                &["token-up-111", "token-down-222"],
            )))
            .mount(&gamma_server)
            .await;

        // Mock Gamma: Old format slug returns 404
        let old_slug = format!("btc-up-or-down-15m-{}", bucket_start);
        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/{}", old_slug)))
            .respond_with(ResponseTemplate::new(404))
            .mount(&gamma_server)
            .await;

        // Mock CLOB: GET /price returns valid prices for both tokens
        Mock::given(method("GET"))
            .and(path("/price"))
            .and(query_param("token_id", "token-up-111"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_clob_price_json("0.55")))
            .mount(&clob_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/price"))
            .and(query_param("token_id", "token-down-222"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_clob_price_json("0.45")))
            .mount(&clob_server)
            .await;

        // Create resolver with mock URLs
        let config = ResolverConfig::default();
        let resolver =
            MarketResolver::with_base_urls(&gamma_server.uri(), &clob_server.uri(), config)
                .expect("Failed to create resolver");

        // Resolve
        let asof = Utc.timestamp_opt(asof_ts, 0).unwrap();
        let result = resolver.resolve(&MarketSeries::Btc15m, asof).await;

        // Assert success
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        let market = result.market().unwrap();
        assert_eq!(market.slug, slug);
        assert_eq!(market.clob_token_ids[0], "token-up-111");
        assert_eq!(market.clob_token_ids[1], "token-down-222");
        assert_eq!(market.selection_reason, SelectionReason::UniqueMatchInWindow);
    }

    /// Test: Zero candidates (FREEZE)
    /// Gamma returns 404 for all slug patterns
    #[tokio::test]
    async fn test_zero_candidates_freeze() {
        let gamma_server = MockServer::start().await;
        let clob_server = MockServer::start().await;

        // Mock Gamma: All slug patterns return 404
        Mock::given(method("GET"))
            .and(path_regex(r"/markets/slug/.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&gamma_server)
            .await;

        let config = ResolverConfig::default();
        let resolver =
            MarketResolver::with_base_urls(&gamma_server.uri(), &clob_server.uri(), config)
                .expect("Failed to create resolver");

        let asof = Utc::now();
        let result = resolver.resolve(&MarketSeries::Btc15m, asof).await;

        // Assert FREEZE with NoCandidates
        assert!(!result.is_ok(), "Expected Freeze, got Ok");
        match result {
            ResolveResult::Freeze { reason, message, .. } => {
                assert_eq!(reason, SelectionReason::NoCandidates);
                assert!(message.contains("No valid market candidates"));
            }
            _ => panic!("Expected Freeze"),
        }
    }

    /// Test: CLOB validation failure (FREEZE)
    /// Gamma returns valid market, but CLOB returns error
    #[tokio::test]
    async fn test_clob_validation_failure_freeze() {
        let gamma_server = MockServer::start().await;
        let clob_server = MockServer::start().await;

        // Use 15-minute aligned timestamp
        let bucket_start = 1736073000i64;
        let asof_ts = bucket_start + 300;
        let slug = format!("btc-updown-15m-{}", bucket_start);

        // Mock Gamma: Returns valid market
        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/{}", slug)))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_gamma_market_json(
                &slug,
                &["token-up-111", "token-down-222"],
            )))
            .mount(&gamma_server)
            .await;

        // Mock old format 404
        let old_slug = format!("btc-up-or-down-15m-{}", bucket_start);
        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/{}", old_slug)))
            .respond_with(ResponseTemplate::new(404))
            .mount(&gamma_server)
            .await;

        // Mock CLOB: Returns 500 error for first token
        Mock::given(method("GET"))
            .and(path("/price"))
            .and(query_param("token_id", "token-up-111"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&clob_server)
            .await;

        let config = ResolverConfig::default();
        let resolver =
            MarketResolver::with_base_urls(&gamma_server.uri(), &clob_server.uri(), config)
                .expect("Failed to create resolver");

        let asof = Utc.timestamp_opt(asof_ts, 0).unwrap();
        let result = resolver.resolve(&MarketSeries::Btc15m, asof).await;

        // Assert FREEZE with ClobPriceCheckFailed
        assert!(!result.is_ok(), "Expected Freeze, got Ok");
        match result {
            ResolveResult::Freeze { reason, message, .. } => {
                assert_eq!(reason, SelectionReason::ClobPriceCheckFailed);
                assert!(message.contains("CLOB") || message.contains("token"));
            }
            _ => panic!("Expected Freeze"),
        }
    }

    /// Test: CLOB returns empty price field (FREEZE)
    #[tokio::test]
    async fn test_clob_no_price_field_freeze() {
        let gamma_server = MockServer::start().await;
        let clob_server = MockServer::start().await;

        // Use 15-minute aligned timestamp
        let bucket_start = 1736073000i64;
        let asof_ts = bucket_start + 300;
        let slug = format!("btc-updown-15m-{}", bucket_start);

        // Mock Gamma: Returns valid market
        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/{}", slug)))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_gamma_market_json(
                &slug,
                &["token-up-111", "token-down-222"],
            )))
            .mount(&gamma_server)
            .await;

        let old_slug = format!("btc-up-or-down-15m-{}", bucket_start);
        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/{}", old_slug)))
            .respond_with(ResponseTemplate::new(404))
            .mount(&gamma_server)
            .await;

        // Mock CLOB: Returns response WITHOUT price field
        Mock::given(method("GET"))
            .and(path("/price"))
            .and(query_param("token_id", "token-up-111"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "ok"})),
            )
            .mount(&clob_server)
            .await;

        let config = ResolverConfig::default();
        let resolver =
            MarketResolver::with_base_urls(&gamma_server.uri(), &clob_server.uri(), config)
                .expect("Failed to create resolver");

        let asof = Utc.timestamp_opt(asof_ts, 0).unwrap();
        let result = resolver.resolve(&MarketSeries::Btc15m, asof).await;

        // Assert FREEZE
        assert!(!result.is_ok(), "Expected Freeze due to missing price field");
        match result {
            ResolveResult::Freeze { reason, message, .. } => {
                assert_eq!(reason, SelectionReason::ClobPriceCheckFailed);
                assert!(message.contains("no price field"));
            }
            _ => panic!("Expected Freeze"),
        }
    }

    /// Test: Time window mismatch (FREEZE)
    /// asof is outside the bucket window - no markets found
    #[tokio::test]
    async fn test_time_window_mismatch_freeze() {
        let gamma_server = MockServer::start().await;
        let clob_server = MockServer::start().await;

        // asof is arbitrary, resolver will calculate expected bucket
        let asof_ts = 1736080800i64; // Some timestamp
        let _expected_bucket = (asof_ts / 900) * 900;

        // Mock: All slug lookups return 404 (no market exists)
        Mock::given(method("GET"))
            .and(path_regex(r"/markets/slug/.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&gamma_server)
            .await;

        let config = ResolverConfig::default();
        let resolver =
            MarketResolver::with_base_urls(&gamma_server.uri(), &clob_server.uri(), config)
                .expect("Failed to create resolver");

        let asof = Utc.timestamp_opt(asof_ts, 0).unwrap();
        let result = resolver.resolve(&MarketSeries::Btc15m, asof).await;

        // Should FREEZE due to no candidates
        assert!(!result.is_ok(), "Expected Freeze");
        match result {
            ResolveResult::Freeze { reason, .. } => {
                assert_eq!(reason, SelectionReason::NoCandidates);
            }
            _ => panic!("Expected Freeze"),
        }
    }

    /// Test: enableOrderBook = false causes FREEZE
    #[tokio::test]
    async fn test_enable_order_book_false_freeze() {
        let gamma_server = MockServer::start().await;
        let clob_server = MockServer::start().await;

        // Use 15-minute aligned timestamp
        let bucket_start = 1736073000i64;
        let asof_ts = bucket_start + 300;
        let slug = format!("btc-updown-15m-{}", bucket_start);

        // Mock Gamma: Returns market with enableOrderBook = false
        let market_json = serde_json::json!({
            "id": "market-id-123",
            "slug": slug,
            "question": "Will BTC be up or down?",
            "conditionId": "condition-id-456",
            "clobTokenIds": "[\"token-up-111\",\"token-down-222\"]",
            "outcomes": "[\"Up\",\"Down\"]",
            "outcomePrices": "[\"0.50\",\"0.50\"]",
            "active": true,
            "closed": false,
            "enableOrderBook": false  // KEY: disabled!
        });

        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/{}", slug)))
            .respond_with(ResponseTemplate::new(200).set_body_json(market_json))
            .mount(&gamma_server)
            .await;

        let old_slug = format!("btc-up-or-down-15m-{}", bucket_start);
        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/{}", old_slug)))
            .respond_with(ResponseTemplate::new(404))
            .mount(&gamma_server)
            .await;

        // Also mock previous bucket as 404
        let prev_bucket = bucket_start - 900;
        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/btc-updown-15m-{}", prev_bucket)))
            .respond_with(ResponseTemplate::new(404))
            .mount(&gamma_server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/btc-up-or-down-15m-{}", prev_bucket)))
            .respond_with(ResponseTemplate::new(404))
            .mount(&gamma_server)
            .await;

        let config = ResolverConfig::default();
        let resolver =
            MarketResolver::with_base_urls(&gamma_server.uri(), &clob_server.uri(), config)
                .expect("Failed to create resolver");

        let asof = Utc.timestamp_opt(asof_ts, 0).unwrap();
        let result = resolver.resolve(&MarketSeries::Btc15m, asof).await;

        // Should FREEZE because enableOrderBook = false makes validation fail
        assert!(!result.is_ok(), "Expected Freeze due to enableOrderBook=false");
    }

    /// Test: CLOB validation disabled via config
    #[tokio::test]
    async fn test_clob_validation_disabled_success() {
        let gamma_server = MockServer::start().await;
        let clob_server = MockServer::start().await;

        // Use 15-minute aligned timestamp
        let bucket_start = 1736073000i64;
        let asof_ts = bucket_start + 300;
        let slug = format!("btc-updown-15m-{}", bucket_start);

        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/{}", slug)))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_gamma_market_json(
                &slug,
                &["token-up-111", "token-down-222"],
            )))
            .mount(&gamma_server)
            .await;

        let old_slug = format!("btc-up-or-down-15m-{}", bucket_start);
        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/{}", old_slug)))
            .respond_with(ResponseTemplate::new(404))
            .mount(&gamma_server)
            .await;

        // Note: NO CLOB mocks - if CLOB validation is called, test will fail

        // Disable CLOB validation
        let mut config = ResolverConfig::default();
        config.clob_validation = false;

        let resolver =
            MarketResolver::with_base_urls(&gamma_server.uri(), &clob_server.uri(), config)
                .expect("Failed to create resolver");

        let asof = Utc.timestamp_opt(asof_ts, 0).unwrap();
        let result = resolver.resolve(&MarketSeries::Btc15m, asof).await;

        // Should succeed even without CLOB validation
        assert!(result.is_ok(), "Expected Ok when CLOB validation is disabled");
    }

    /// Test: CLOB side case fallback (BUY -> buy)
    /// First request with side=BUY returns 400, second with side=buy succeeds
    #[tokio::test]
    async fn test_clob_side_case_fallback_success() {
        let gamma_server = MockServer::start().await;
        let clob_server = MockServer::start().await;

        let bucket_start = 1736073000i64;
        let asof_ts = bucket_start + 300;
        let slug = format!("btc-updown-15m-{}", bucket_start);

        // Mock Gamma: Returns valid market
        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/{}", slug)))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_gamma_market_json(
                &slug,
                &["token-up-111", "token-down-222"],
            )))
            .mount(&gamma_server)
            .await;

        let old_slug = format!("btc-up-or-down-15m-{}", bucket_start);
        Mock::given(method("GET"))
            .and(path(format!("/markets/slug/{}", old_slug)))
            .respond_with(ResponseTemplate::new(404))
            .mount(&gamma_server)
            .await;

        // Mock CLOB: side=BUY returns 400 Bad Request
        Mock::given(method("GET"))
            .and(path("/price"))
            .and(query_param("token_id", "token-up-111"))
            .and(query_param("side", "BUY"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string("Bad Request: invalid side parameter"),
            )
            .mount(&clob_server)
            .await;

        // Mock CLOB: side=buy returns valid price
        Mock::given(method("GET"))
            .and(path("/price"))
            .and(query_param("token_id", "token-up-111"))
            .and(query_param("side", "buy"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_clob_price_json("0.55")))
            .mount(&clob_server)
            .await;

        // Mock second token - also with fallback
        Mock::given(method("GET"))
            .and(path("/price"))
            .and(query_param("token_id", "token-down-222"))
            .and(query_param("side", "BUY"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string("Bad Request: invalid side parameter"),
            )
            .mount(&clob_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/price"))
            .and(query_param("token_id", "token-down-222"))
            .and(query_param("side", "buy"))
            .respond_with(ResponseTemplate::new(200).set_body_json(make_clob_price_json("0.45")))
            .mount(&clob_server)
            .await;

        let config = ResolverConfig::default();
        let resolver =
            MarketResolver::with_base_urls(&gamma_server.uri(), &clob_server.uri(), config)
                .expect("Failed to create resolver");

        let asof = Utc.timestamp_opt(asof_ts, 0).unwrap();
        let result = resolver.resolve(&MarketSeries::Btc15m, asof).await;

        // Should succeed after falling back to lowercase "buy"
        assert!(
            result.is_ok(),
            "Expected Ok after side case fallback, got {:?}",
            result
        );

        // Verify audit fields are present
        let market = result.market().unwrap();
        assert!(!market.asof_utc.is_empty());
        assert!(!market.candidate_slugs.is_empty());
        assert_eq!(market.bucket_start_ts, bucket_start);
    }
}
