//! Switch Controller - Two-Phase Market Switch Safety Rails
//!
//! # Design Principles
//! 1. "宁可延迟切换，也不选错市场" - Better to delay than select wrong market
//! 2. Pre-resolve next market before boundary (lead_time)
//! 3. Require N consecutive consistent resolutions (debounce)
//! 4. Overlap old/new subscriptions during switch
//! 5. Monotonicity: next.bucket_start must equal current.bucket_start + 900
//! 6. Commit-time CLOB validation: re-validate tokens before switching
//!
//! # State Machine
//! Stable -> Prepare (lead_time before boundary)
//! Prepare -> Ready (N consecutive matches)
//! Ready -> Committing (boundary reached + CLOB check)
//! Committing -> Stable (overlap complete)

use std::time::Instant;

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use tracing::{debug, error, info, warn};

use super::resolver::{MarketResolver, MarketSeries, ResolverConfig};
use crate::types::{ResolveResult, ResolvedMarket, SwitchAction, SwitchConfig, SwitchPhase, SwitchStats};

/// Bucket size in seconds (15 minutes)
const BUCKET_SIZE_SECS: i64 = 900;

/// Candidate for next market (during Prepare phase)
#[derive(Clone, Debug)]
pub struct NextCandidate {
    /// Resolved market
    pub market: ResolvedMarket,
    /// When this candidate was first seen
    pub first_seen_at: Instant,
    /// Consecutive matching resolutions
    pub consecutive_matches: u32,
}

/// Pending unsubscribe action (after overlap)
#[derive(Clone, Debug)]
struct PendingUnsubscribe {
    tokens: [String; 2],
    slug: String,
    scheduled_at: Instant,
}

/// Switch Controller - manages market transitions with safety guarantees
pub struct SwitchController {
    resolver: MarketResolver,
    series: MarketSeries,
    config: SwitchConfig,

    // State
    phase: SwitchPhase,
    current: Option<ResolvedMarket>,
    next_candidate: Option<NextCandidate>,
    pending_unsubscribe: Option<PendingUnsubscribe>,

    // Stats
    stats: SwitchStats,
    last_resolve_ok_at: Option<Instant>,
    boundary_reached_at: Option<Instant>,
}

impl SwitchController {
    /// Create a new switch controller
    pub fn new(series: MarketSeries, config: SwitchConfig) -> Result<Self> {
        Ok(Self {
            resolver: MarketResolver::new()?,
            series,
            config,
            phase: SwitchPhase::Stable,
            current: None,
            next_candidate: None,
            pending_unsubscribe: None,
            stats: SwitchStats::default(),
            last_resolve_ok_at: None,
            boundary_reached_at: None,
        })
    }

    /// Create with custom resolver config
    pub fn with_resolver_config(
        series: MarketSeries,
        switch_config: SwitchConfig,
        resolver_config: ResolverConfig,
    ) -> Result<Self> {
        Ok(Self {
            resolver: MarketResolver::with_config(resolver_config)?,
            series,
            config: switch_config,
            phase: SwitchPhase::Stable,
            current: None,
            next_candidate: None,
            pending_unsubscribe: None,
            stats: SwitchStats::default(),
            last_resolve_ok_at: None,
            boundary_reached_at: None,
        })
    }

    /// Get current phase
    pub fn phase(&self) -> &SwitchPhase {
        &self.phase
    }

    /// Get current market (if any)
    pub fn current(&self) -> Option<&ResolvedMarket> {
        self.current.as_ref()
    }

    /// Get next candidate (if any)
    pub fn next_candidate(&self) -> Option<&NextCandidate> {
        self.next_candidate.as_ref()
    }

    /// Get statistics
    pub fn stats(&self) -> &SwitchStats {
        &self.stats
    }

    /// Initialize controller by resolving current market
    pub async fn init(&mut self) -> Result<SwitchAction> {
        info!("Initializing SwitchController for {:?}", self.series);
        let now = Utc::now();

        match self.resolver.resolve(&self.series, now).await {
            ResolveResult::Ok(market) => {
                info!("Initialized with market: {} (bucket_start: {})", market.slug, market.bucket_start_ts);
                let tokens = market.clob_token_ids.clone();
                let slug = market.slug.clone();
                self.current = Some(market);
                self.last_resolve_ok_at = Some(Instant::now());
                self.phase = SwitchPhase::Stable;
                Ok(SwitchAction::SubscribeNew { tokens, slug })
            }
            ResolveResult::Freeze { reason, message, .. } => {
                warn!("Init failed: {:?} - {}", reason, message);
                self.stats.freeze_count += 1;
                Ok(SwitchAction::Freeze {
                    reason: format!("{:?}", reason),
                    message,
                })
            }
        }
    }

    /// Poll for state updates - call this periodically (every poll_interval_ms)
    pub async fn poll(&mut self) -> SwitchAction {
        // Check for pending unsubscribe first
        if let Some(pending) = &self.pending_unsubscribe {
            let elapsed = pending.scheduled_at.elapsed().as_secs();
            if elapsed >= self.config.overlap_secs {
                let pending = self.pending_unsubscribe.take().unwrap();
                info!("Overlap complete, unsubscribing old: {}", pending.slug);
                return SwitchAction::UnsubscribeOld {
                    tokens: pending.tokens,
                    slug: pending.slug,
                };
            }
        }

        match self.phase {
            SwitchPhase::Stable => self.poll_stable().await,
            SwitchPhase::Prepare => self.poll_prepare().await,
            SwitchPhase::Ready => self.poll_ready().await,
            SwitchPhase::Committing => self.poll_committing().await,
        }
    }

    /// Poll in Stable phase - check if we should start preparing next
    async fn poll_stable(&mut self) -> SwitchAction {
        if self.should_prepare_next() {
            info!("Entering Prepare phase (lead_time reached)");
            self.phase = SwitchPhase::Prepare;
            self.next_candidate = None;
            return self.poll_prepare().await;
        }

        // Optionally re-validate current market
        SwitchAction::None
    }

    /// Poll in Prepare phase - resolve next and check consistency
    async fn poll_prepare(&mut self) -> SwitchAction {
        let next_asof = self.next_bucket_asof();
        debug!("Prepare: resolving next bucket with asof={}", next_asof);

        match self.resolver.resolve(&self.series, next_asof).await {
            ResolveResult::Ok(market) => {
                self.last_resolve_ok_at = Some(Instant::now());

                // CRITICAL: Check monotonicity first
                if !self.is_monotonic_advance(&market) {
                    self.stats.freeze_count += 1;
                    warn!(
                        "Prepare: FREEZE_HARD - monotonicity violation for {}",
                        market.slug
                    );
                    // Reset candidate and stay in Prepare
                    self.next_candidate = None;
                    return SwitchAction::Freeze {
                        reason: "MonotonicityViolation".to_string(),
                        message: format!(
                            "next.bucket_start={} is not current+900",
                            market.bucket_start_ts
                        ),
                    };
                }

                if self.is_consistent(&market) {
                    // Increment consecutive count
                    let candidate = self.next_candidate.as_mut().unwrap();
                    candidate.consecutive_matches += 1;
                    let matches = candidate.consecutive_matches;

                    debug!(
                        "Prepare: consistent match {}/{} for {}",
                        matches, self.config.min_consecutive, market.slug
                    );

                    if matches >= self.config.min_consecutive {
                        info!(
                            "Prepare: next is READY after {} consecutive matches: {}",
                            matches, market.slug
                        );
                        self.phase = SwitchPhase::Ready;

                        // Calculate lead time for stats
                        if let Some(current) = &self.current {
                            if let Ok(end) = DateTime::parse_from_rfc3339(&current.end_date) {
                                let secs_to_end = end.timestamp() - Utc::now().timestamp();
                                self.stats.last_ready_lead_secs = Some(secs_to_end);
                            }
                        }
                    }
                } else {
                    // New candidate or mismatch - reset (but only if monotonic)
                    debug!("Prepare: new candidate or mismatch, resetting to: {}", market.slug);
                    self.next_candidate = Some(NextCandidate {
                        market,
                        first_seen_at: Instant::now(),
                        consecutive_matches: 1,
                    });
                }

                SwitchAction::None
            }
            ResolveResult::Freeze { reason, message, .. } => {
                self.stats.freeze_count += 1;
                warn!("Prepare: freeze during next resolution: {:?} - {}", reason, message);
                // Stay in Prepare, retry on next poll
                SwitchAction::None
            }
        }
    }

    /// Poll in Ready phase - wait for boundary, then commit
    /// Includes commit-time CLOB validation as final safety check
    async fn poll_ready(&mut self) -> SwitchAction {
        if !self.is_boundary_reached() {
            return SwitchAction::None;
        }

        info!("Boundary reached, performing commit-time CLOB validation...");
        self.boundary_reached_at = Some(Instant::now());

        // Commit-time CLOB validation: re-check tokens before switching
        if let Some(candidate) = &self.next_candidate {
            let tokens = &candidate.market.clob_token_ids;
            match self.validate_tokens_for_commit(tokens).await {
                Ok(true) => {
                    info!("Commit-time CLOB validation passed, entering Committing phase");
                    self.phase = SwitchPhase::Committing;
                    self.poll_committing().await
                }
                Ok(false) => {
                    // Tokens exist but no valid price - FREEZE_SOFT, stay in Ready
                    self.stats.freeze_count += 1;
                    warn!("Commit-time CLOB validation failed: no price, staying in Ready");
                    SwitchAction::Freeze {
                        reason: "CommitClobNoPriceField".to_string(),
                        message: "CLOB tokens have no price at commit time".to_string(),
                    }
                }
                Err(e) => {
                    // CLOB error - FREEZE_SOFT, stay in Ready and retry
                    self.stats.freeze_count += 1;
                    warn!("Commit-time CLOB validation error: {}, staying in Ready", e);
                    SwitchAction::Freeze {
                        reason: "CommitClobError".to_string(),
                        message: format!("CLOB error at commit time: {}", e),
                    }
                }
            }
        } else {
            warn!("Boundary reached but no next candidate, falling back to Stable");
            self.phase = SwitchPhase::Stable;
            SwitchAction::None
        }
    }

    /// Validate tokens for commit-time check
    /// Returns Ok(true) if tokens are tradeable, Ok(false) if no price, Err on API error
    async fn validate_tokens_for_commit(&self, tokens: &[String; 2]) -> Result<bool> {
        // Only check the first token (Up) - if one works, the pair is likely good
        let token = &tokens[0];
        debug!("Commit-time validation for token: {}", token);

        // Use the resolver's internal CLOB client for price check
        // Try both "BUY" and "buy" cases
        for side in &["BUY", "buy"] {
            match self.resolver.clob().get_price(token, side).await {
                Ok(price_data) => {
                    if price_data.get("price").is_some() {
                        debug!("Commit-time CLOB check passed for {}", token);
                        return Ok(true);
                    } else {
                        debug!("Commit-time CLOB check: no price field for {}", token);
                        return Ok(false);
                    }
                }
                Err(e) => {
                    let error_str = e.to_string();
                    // If 400 error, try next side variant
                    if error_str.contains("400") || error_str.contains("Bad Request") {
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        // All side variants failed
        anyhow::bail!("All CLOB side variants failed for commit-time check")
    }

    /// Poll in Committing phase - execute switch
    async fn poll_committing(&mut self) -> SwitchAction {
        let next = match self.next_candidate.take() {
            Some(c) => c,
            None => {
                warn!("Committing: no next candidate, falling back to Stable");
                self.phase = SwitchPhase::Stable;
                return SwitchAction::None;
            }
        };

        let old = self.current.take();
        let from_slug = old.as_ref().map(|m| m.slug.clone()).unwrap_or_default();
        let to_slug = next.market.slug.clone();
        let new_tokens = next.market.clob_token_ids.clone();

        // Schedule unsubscribe for old tokens
        if let Some(old_market) = old {
            self.pending_unsubscribe = Some(PendingUnsubscribe {
                tokens: old_market.clob_token_ids.clone(),
                slug: old_market.slug.clone(),
                scheduled_at: Instant::now(),
            });
        }

        // Update current
        self.current = Some(next.market);
        self.phase = SwitchPhase::Stable;
        self.stats.switch_count += 1;

        // Calculate switch latency
        if let Some(boundary_at) = self.boundary_reached_at.take() {
            self.stats.last_switch_latency_ms = Some(boundary_at.elapsed().as_millis() as u64);
        }

        info!("SWITCH: {} -> {}", from_slug, to_slug);

        // Return SubscribeNew - UnsubscribeOld will come after overlap
        SwitchAction::SubscribeNew {
            tokens: new_tokens,
            slug: to_slug,
        }
    }

    /// Check if we should start preparing next market
    fn should_prepare_next(&self) -> bool {
        let current = match &self.current {
            Some(m) => m,
            None => return false,
        };

        // Parse end_date
        let end = match DateTime::parse_from_rfc3339(&current.end_date) {
            Ok(dt) => dt,
            Err(_) => return false,
        };

        let now = Utc::now();
        let secs_to_end = (end.timestamp() - now.timestamp()).max(0);

        secs_to_end <= self.config.lead_time_secs
    }

    /// Calculate asof time for next bucket
    fn next_bucket_asof(&self) -> DateTime<Utc> {
        let next_bucket_ts = self
            .current
            .as_ref()
            .map(|m| m.bucket_start_ts + 905) // 900 + 5s safety margin
            .unwrap_or_else(|| Utc::now().timestamp() + 900);

        Utc.timestamp_opt(next_bucket_ts, 0)
            .single()
            .unwrap_or_else(Utc::now)
    }

    /// Check if resolved market is consistent with current candidate
    fn is_consistent(&self, new: &ResolvedMarket) -> bool {
        match &self.next_candidate {
            Some(candidate) => {
                candidate.market.slug == new.slug
                    && candidate.market.clob_token_ids == new.clob_token_ids
            }
            None => false,
        }
    }

    /// Check if next market is a valid monotonic advance from current
    /// CRITICAL: Prevents "rollback" to older buckets
    fn is_monotonic_advance(&self, next: &ResolvedMarket) -> bool {
        match &self.current {
            Some(current) => {
                let expected_next = current.bucket_start_ts + BUCKET_SIZE_SECS;
                if next.bucket_start_ts != expected_next {
                    error!(
                        "MONOTONICITY VIOLATION: current.bucket_start={}, next.bucket_start={}, expected={}",
                        current.bucket_start_ts, next.bucket_start_ts, expected_next
                    );
                    return false;
                }
                true
            }
            // No current market - any bucket is acceptable for init
            None => true,
        }
    }

    /// Check if current bucket boundary has been reached
    fn is_boundary_reached(&self) -> bool {
        let current = match &self.current {
            Some(m) => m,
            None => return false,
        };

        let end = match DateTime::parse_from_rfc3339(&current.end_date) {
            Ok(dt) => dt,
            Err(_) => return false,
        };

        Utc::now().timestamp() >= end.timestamp()
    }

    /// Format status line for observability
    pub fn status_line(&self) -> String {
        let now = Utc::now().format("%H:%M:%S");
        let phase = format!("{:?}", self.phase);
        let current_slug = self.current.as_ref().map(|m| m.slug.as_str()).unwrap_or("None");

        let next_info = match &self.next_candidate {
            Some(c) => format!(
                "{}({}/{})",
                c.market.slug, c.consecutive_matches, self.config.min_consecutive
            ),
            None => "None".to_string(),
        };

        format!(
            "[{}] phase={} current={} next={} freeze_count={}",
            now, phase, current_slug, next_info, self.stats.freeze_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_switch_config_default() {
        let config = SwitchConfig::default();
        assert_eq!(config.lead_time_secs, 90);  // 90s for more margin
        assert_eq!(config.min_consecutive, 3);
        assert_eq!(config.overlap_secs, 15);
        assert_eq!(config.poll_interval_ms, 2000);
    }

    #[test]
    fn test_switch_phase_serialization() {
        let phase = SwitchPhase::Prepare;
        let json = serde_json::to_string(&phase).unwrap();
        assert_eq!(json, "\"prepare\"");
    }

    #[test]
    fn test_switch_action_none() {
        let action = SwitchAction::None;
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"action\":\"none\""));
    }

    #[test]
    fn test_switch_action_subscribe_new() {
        let action = SwitchAction::SubscribeNew {
            tokens: ["token1".to_string(), "token2".to_string()],
            slug: "test-slug".to_string(),
        };
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"action\":\"subscribe_new\""));
        assert!(json.contains("\"slug\":\"test-slug\""));
    }
}
