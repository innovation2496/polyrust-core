//! Polymarket CLOB Smoke Test CLI
//!
//! Commands:
//! - `market`: Subscribe to market channel and collect messages
//! - `user`: Subscribe to user channel (requires credentials)
//! - `rest`: Test REST API connectivity
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
//! ```

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{error, info};

use polymarket_adapter::httpws::{ApiCredentials, MarketWsClient, RestClient, UserWsClient};
use polymarket_adapter::{CLOB_REST_BASE, CLOB_WSS_ENDPOINT};

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
        /// Asset ID (token_id) to subscribe to
        #[arg(long)]
        asset_id: String,

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
        Commands::Market { asset_id, out, limit, enable_features } => {
            run_market_smoke(asset_id, out, limit, enable_features, shutdown).await
        }
        Commands::User { market_id, out, limit } => {
            run_user_smoke(market_id, out, limit, shutdown).await
        }
        Commands::Rest { asset_id } => run_rest_smoke(asset_id).await,
    }
}

async fn run_market_smoke(
    asset_id: String,
    out: PathBuf,
    limit: u64,
    enable_features: bool,
    shutdown: Arc<AtomicBool>,
) -> Result<()> {
    info!("=== Market Channel Smoke Test ===");
    info!("Endpoint: {}", CLOB_WSS_ENDPOINT);
    info!("Asset ID: {}", asset_id);
    info!("Output: {}", out.display());
    info!("Limit: {} (0 = unlimited)", limit);
    info!("Features enabled: {}", enable_features);
    info!("Press Ctrl+C to stop");
    info!("");

    // Ensure output directory exists
    if let Some(parent) = out.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut client = MarketWsClient::new(vec![asset_id]);
    client.set_enable_features(enable_features);

    let stats = client.run(&out, limit, shutdown).await?;

    // Print summary
    info!("");
    info!("=== Summary ===");
    info!("Total messages: {}", stats.total_messages);
    info!("Parsed OK: {}", stats.parsed_ok);
    info!("Unknown type count: {}", stats.unknown_type_count);
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
