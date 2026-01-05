//! Polymarket CLOB Smoke Test CLI
//!
//! Commands:
//! - `market`: Subscribe to market channel and collect messages
//! - `user`: Subscribe to user channel (requires credentials)
//! - `rest`: Test REST API connectivity
//! - `resolve`: Resolve current 15-minute market for trading
//!
//! # Usage
//! ```bash
//! # Market channel smoke test
//! pm_smoke market --asset-id <ASSET_ID> --out data/ws_raw.jsonl --limit 500
//!
//! # User channel (requires env vars)
//! POLY_API_KEY=... POLY_API_SECRET=... POLY_API_PASSPHRASE=...
//! pm_smoke user --market-id <MARKET_ID> --out data/user_raw.jsonl --limit 200
//!
//! # REST connectivity test
//! pm_smoke rest --asset-id <ASSET_ID>
//!
//! # Resolve current BTC 15-minute market
//! pm_smoke resolve --series btc15m
//! pm_smoke resolve --series btc15m --out resolved.json
//! ```

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{error, info, warn};

use polymarket_adapter::gamma::{MarketResolver, MarketSeries};
use polymarket_adapter::httpws::{ApiCredentials, MarketWsClient, RestClient, UserWsClient};
use polymarket_adapter::types::ResolveResult;
use polymarket_adapter::{CLOB_REST_BASE, CLOB_WSS_ENDPOINT, GAMMA_API_BASE};

#[derive(Parser)]
#[command(name = "pm_smoke")]
#[command(about = "Polymarket CLOB smoke test CLI")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info", global = true)]
    log_level: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Subscribe to market channel and collect messages
    Market {
        /// Asset ID(s) (token_id) to subscribe to. Can specify multiple times.
        #[arg(long, required = true)]
        asset_id: Vec<String>,

        /// Output file path for raw JSONL
        #[arg(long, default_value = "data/ws_market_raw.jsonl")]
        out: PathBuf,

        /// Maximum messages to collect (0 = unlimited until Ctrl+C)
        #[arg(long, default_value = "500")]
        limit: u64,

        /// Enable feature-flagged messages (best_bid_ask, new_market, etc.)
        #[arg(long, default_value = "true")]
        enable_features: bool,
    },

    /// Subscribe to user channel (requires POLY_API_KEY, POLY_API_SECRET, POLY_API_PASSPHRASE)
    User {
        /// Market ID (condition_id) to subscribe to
        #[arg(long)]
        market_id: String,

        /// Output file path for raw JSONL
        #[arg(long, default_value = "data/ws_user_raw.jsonl")]
        out: PathBuf,

        /// Maximum messages to collect (0 = unlimited until Ctrl+C)
        #[arg(long, default_value = "200")]
        limit: u64,
    },

    /// Test REST API connectivity
    Rest {
        /// Asset ID (token_id) for book/price queries
        #[arg(long)]
        asset_id: Option<String>,
    },

    /// Resolve current 15-minute market for a series
    Resolve {
        /// Market series to resolve (btc15m, eth15m)
        #[arg(long)]
        series: String,

        /// Reference time for resolution (ISO 8601, default: now)
        #[arg(long)]
        asof: Option<String>,

        /// Output file for ResolvedMarket JSON (optional, defaults to stdout)
        #[arg(long)]
        out: Option<PathBuf>,

        /// Skip CLOB price validation
        #[arg(long, default_value = "false")]
        skip_clob_check: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&cli.log_level));

    tracing_subscriber::fmt().with_env_filter(env_filter).with_target(false).init();

    // Setup Ctrl+C handler
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Received Ctrl+C, shutting down...");
        shutdown_clone.store(true, Ordering::Relaxed);
    });

    match cli.command {
        Commands::Market { asset_id: asset_ids, out, limit, enable_features } => {
            run_market_smoke(asset_ids, out, limit, enable_features, shutdown).await
        }
        Commands::User { market_id, out, limit } => {
            run_user_smoke(market_id, out, limit, shutdown).await
        }
        Commands::Rest { asset_id } => run_rest_smoke(asset_id).await,
        Commands::Resolve { series, asof, out, skip_clob_check } => {
            run_resolve(series, asof, out, skip_clob_check).await
        }
    }
}

async fn run_market_smoke(
    asset_ids: Vec<String>,
    out: PathBuf,
    limit: u64,
    enable_features: bool,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    info!("=== Market Channel Smoke Test ===");
    info!("Endpoint: {}", CLOB_WSS_ENDPOINT);
    info!("Asset IDs: {} token(s)", asset_ids.len());
    for (i, id) in asset_ids.iter().enumerate() {
        info!("  [{}]: {}", i, id);
    }
    info!("Output: {}", out.display());
    info!("Limit: {} (0 = unlimited)", limit);
    info!("Features enabled: {}", enable_features);
    info!("Press Ctrl+C to stop");
    info!("");

    // Ensure output directory exists
    if let Some(parent) = out.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut client = MarketWsClient::new(asset_ids);
    client.set_enable_features(enable_features);

    let stats = client.run(&out, limit, shutdown).await?;

    // Print summary
    info!("");
    info!("=== Summary ===");
    info!("Total messages: {}", stats.total_messages);
    info!("Parsed OK: {}", stats.parsed_ok);
    info!("Unknown type count: {}", stats.unknown_type_count);
    info!("Snapshot arrays: {}", stats.snapshot_array_count);
    info!("Parse errors: {}", stats.parse_error_count);
    info!("Last message type: {:?}", stats.last_message_type);
    info!("");
    info!("Message type distribution:");
    let mut types: Vec<_> = stats.type_counts.iter().collect();
    types.sort_by(|a, b| b.1.cmp(a.1));
    for (msg_type, count) in types {
        info!("  {}: {}", msg_type, count);
    }
    info!("");
    info!("Output written to: {}", out.display());

    Ok(())
}

async fn run_user_smoke(
    market_id: String,
    out: PathBuf,
    limit: u64,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    info!("=== User Channel Smoke Test ===");
    info!("Endpoint: {}", CLOB_WSS_ENDPOINT);
    info!("Market ID: {}", market_id);
    info!("Output: {}", out.display());
    info!("Limit: {} (0 = unlimited)", limit);
    info!("");

    // Load credentials from environment
    let credentials = match ApiCredentials::from_env() {
        Some(c) => c,
        None => {
            error!("Missing credentials. Set environment variables:");
            error!("  POLY_API_KEY");
            error!("  POLY_API_SECRET");
            error!("  POLY_API_PASSPHRASE");
            anyhow::bail!("Missing credentials");
        }
    };

    if !credentials.is_valid() {
        error!("Invalid credentials - one or more fields are empty");
        anyhow::bail!("Invalid credentials");
    }

    info!("Credentials loaded: {:?}", credentials);
    info!("Press Ctrl+C to stop");
    info!("");

    // Ensure output directory exists
    if let Some(parent) = out.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let client = UserWsClient::new(credentials, vec![market_id]);

    let stats = client.run(&out, limit, shutdown).await?;

    // Print summary
    info!("");
    info!("=== Summary ===");
    info!("Total messages: {}", stats.total_messages);
    info!("Parsed OK: {}", stats.parsed_ok);
    info!("Unknown type count: {}", stats.unknown_type_count);
    info!("Snapshot arrays: {}", stats.snapshot_array_count);
    info!("Parse errors: {}", stats.parse_error_count);
    info!("Last message type: {:?}", stats.last_message_type);
    info!("");
    info!("Message type distribution:");
    let mut types: Vec<_> = stats.type_counts.iter().collect();
    types.sort_by(|a, b| b.1.cmp(a.1));
    for (msg_type, count) in types {
        info!("  {}: {}", msg_type, count);
    }
    info!("");
    info!("Output written to: {}", out.display());

    Ok(())
}

async fn run_rest_smoke(asset_id: Option<String>) -> Result<()> {
    info!("=== REST API Smoke Test ===");
    info!("Base URL: {}", CLOB_REST_BASE);
    info!("");

    let client = RestClient::new()?;

    // Test connectivity
    info!("Testing connectivity...");
    match client.test_connectivity().await {
        Ok(_) => info!("Connectivity: OK"),
        Err(e) => {
            error!("Connectivity failed: {}", e);
            // Don't fail completely - continue with other tests
        }
    }

    // If asset_id provided, try to get book
    if let Some(asset_id) = asset_id {
        info!("");
        info!("Fetching orderbook for asset: {}", asset_id);

        match client.get_book(&asset_id).await {
            Ok(book) => {
                info!("Book response:");
                // Pretty print first few levels
                if let Some(bids) = book.get("bids") {
                    if let Some(arr) = bids.as_array() {
                        info!("  Bids: {} levels", arr.len());
                        for (i, bid) in arr.iter().take(3).enumerate() {
                            info!("    {}: {:?}", i, bid);
                        }
                    }
                }
                if let Some(asks) = book.get("asks") {
                    if let Some(arr) = asks.as_array() {
                        info!("  Asks: {} levels", arr.len());
                        for (i, ask) in arr.iter().take(3).enumerate() {
                            info!("    {}: {:?}", i, ask);
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to get book: {}", e);
            }
        }

        info!("");
        info!("Fetching midpoint...");
        match client.get_midpoint(&asset_id).await {
            Ok(mid) => info!("Midpoint: {:?}", mid),
            Err(e) => error!("Failed to get midpoint: {}", e),
        }

        info!("");
        info!("Fetching spread...");
        match client.get_spread(&asset_id).await {
            Ok(spread) => info!("Spread: {:?}", spread),
            Err(e) => error!("Failed to get spread: {}", e),
        }

        info!("");
        info!("Fetching tick size...");
        match client.get_tick_size(&asset_id).await {
            Ok(tick) => info!("Tick size: {:?}", tick),
            Err(e) => error!("Failed to get tick size: {}", e),
        }
    } else {
        info!("");
        info!("No asset-id provided, skipping book/price queries");
        info!("Use --asset-id <TOKEN_ID> to test specific market data");
    }

    info!("");
    info!("REST smoke test complete");

    Ok(())
}

async fn run_resolve(
    series: String,
    asof: Option<String>,
    out: Option<PathBuf>,
    skip_clob_check: bool,
) -> Result<()> {
    info!("=== Market Resolver ===");
    info!("Gamma API: {}", GAMMA_API_BASE);
    info!("CLOB API: {}", CLOB_REST_BASE);
    info!("Series: {}", series);
    info!("");

    // Parse series
    let market_series = match MarketSeries::from_str(&series) {
        Some(s) => s,
        None => {
            error!("Unknown series: {}. Supported: btc15m, eth15m", series);
            anyhow::bail!("Unknown series: {}", series);
        }
    };

    // Parse asof time
    let asof_time: DateTime<Utc> = match asof {
        Some(ref s) => {
            DateTime::parse_from_rfc3339(s)
                .map_err(|e| anyhow::anyhow!("Invalid asof time '{}': {}", s, e))?
                .with_timezone(&Utc)
        }
        None => Utc::now(),
    };

    info!("Reference time (asof): {}", asof_time);
    info!("CLOB validation: {}", if skip_clob_check { "disabled" } else { "enabled" });
    info!("");

    // Create resolver
    let mut config = polymarket_adapter::gamma::resolver::ResolverConfig::default();
    config.clob_validation = !skip_clob_check;

    let resolver = MarketResolver::with_config(config)?;

    // Resolve
    info!("Resolving market...");
    let result = resolver.resolve(&market_series, asof_time).await;

    // Output result
    let json_output = serde_json::to_string_pretty(&result)?;

    match &result {
        ResolveResult::Ok(market) => {
            info!("");
            info!("=== Resolution SUCCESS ===");
            info!("Slug: {}", market.slug);
            info!("Condition ID: {}", market.condition_id);
            info!("CLOB Token IDs:");
            info!("  [0] {}: {}", market.outcomes[0], market.clob_token_ids[0]);
            info!("  [1] {}: {}", market.outcomes[1], market.clob_token_ids[1]);
            info!("Start: {}", market.start_date);
            info!("End: {}", market.end_date);
            info!("Selection reason: {:?}", market.selection_reason);
        }
        ResolveResult::Freeze { reason, message, candidates } => {
            warn!("");
            warn!("=== Resolution FREEZE ===");
            warn!("Reason: {:?}", reason);
            warn!("Message: {}", message);
            warn!("Candidates considered: {:?}", candidates);
            warn!("");
            warn!("DO NOT TRADE - Resolution failed");
        }
    }

    // Write to file or stdout
    if let Some(out_path) = out {
        if let Some(parent) = out_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&out_path, &json_output).await?;
        info!("");
        info!("Output written to: {}", out_path.display());
    } else {
        // Print JSON to stdout
        println!();
        println!("{}", json_output);
    }

    // Return error if FREEZE
    if !result.is_ok() {
        anyhow::bail!("Market resolution failed - FREEZE");
    }

    Ok(())
}
