//! WebSocket client for Polymarket user channel
//!
//! Endpoint: wss://ws-subscriptions-clob.polymarket.com/ws/
//!
//! # Authentication Required
//! User channel requires L2 credentials (apiKey, secret, passphrase)
//! passed in the subscription message's `auth` field.
//!
//! # Source
//! - WSS Overview: https://docs.polymarket.com/developers/CLOB/websocket/wss-overview
//! - User Channel: https://docs.polymarket.com/developers/CLOB/websocket/user-channel
//! - WSS Auth: https://docs.polymarket.com/developers/CLOB/websocket/wss-auth

use anyhow::{Context, Result};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use crate::httpws::auth::ApiCredentials;
use crate::types::{MessageStats, SubscribeRequest, WsAuth, WsInboundMessage};
use crate::CLOB_WSS_ENDPOINT;

/// Maximum reconnection backoff interval
const MAX_BACKOFF_SECS: u64 = 30;

/// Initial backoff interval
const INITIAL_BACKOFF_SECS: u64 = 1;

/// User channel WebSocket client
pub struct UserWsClient {
    endpoint: String,
    credentials: ApiCredentials,
    market_ids: Vec<String>,
}

impl UserWsClient {
    /// Create a new user channel client
    ///
    /// # Arguments
    /// * `credentials` - L2 API credentials (apiKey, secret, passphrase)
    /// * `market_ids` - Condition IDs to subscribe to
    pub fn new(credentials: ApiCredentials, market_ids: Vec<String>) -> Self {
        Self { endpoint: CLOB_WSS_ENDPOINT.to_string(), credentials, market_ids }
    }

    /// Create with custom endpoint (for testing)
    pub fn with_endpoint(
        endpoint: &str,
        credentials: ApiCredentials,
        market_ids: Vec<String>,
    ) -> Self {
        Self { endpoint: endpoint.to_string(), credentials, market_ids }
    }

    /// Run the client, collecting messages until limit or shutdown
    ///
    /// # Arguments
    /// * `output_path` - Path to write raw JSONL
    /// * `limit` - Maximum messages to collect (0 = unlimited)
    /// * `shutdown` - Atomic flag to signal shutdown
    pub async fn run(
        &self,
        output_path: &Path,
        limit: u64,
        shutdown: Arc<AtomicBool>,
    ) -> Result<MessageStats> {
        let mut stats = MessageStats::new();
        let mut backoff_secs = INITIAL_BACKOFF_SECS;
        let mut total_collected: u64 = 0;

        // Create output file
        let mut file = File::create(output_path).await.context("Failed to create output file")?;

        info!("Starting user channel client, output: {}", output_path.display());

        while !shutdown.load(Ordering::Relaxed) {
            match self.connect_and_subscribe().await {
                Ok((mut write, mut read)) => {
                    info!("Connected and subscribed to user channel");
                    backoff_secs = INITIAL_BACKOFF_SECS; // Reset backoff on success

                    // Read messages
                    while !shutdown.load(Ordering::Relaxed) {
                        // Check limit
                        if limit > 0 && total_collected >= limit {
                            info!("Reached message limit: {}", limit);
                            return Ok(stats);
                        }

                        // Read with timeout for responsiveness
                        let msg = tokio::time::timeout(Duration::from_secs(30), read.next()).await;

                        match msg {
                            Ok(Some(Ok(Message::Text(text)))) => {
                                // Write raw to file (JSONL format)
                                file.write_all(text.as_bytes()).await?;
                                file.write_all(b"\n").await?;

                                // Parse and record stats
                                let parsed = WsInboundMessage::parse(&text);
                                stats.record(&parsed);
                                total_collected += 1;

                                if total_collected % 10 == 0 {
                                    debug!(
                                        "Collected {} user messages, {} unknown",
                                        total_collected, stats.unknown_type_count
                                    );
                                }
                            }
                            Ok(Some(Ok(Message::Ping(data)))) => {
                                // Respond to ping
                                if let Err(e) = write.send(Message::Pong(data)).await {
                                    warn!("Failed to send pong: {}", e);
                                }
                            }
                            Ok(Some(Ok(Message::Close(_)))) => {
                                info!("Server closed connection");
                                break;
                            }
                            Ok(Some(Ok(_))) => {
                                // Binary or other message types - ignore
                            }
                            Ok(Some(Err(e))) => {
                                warn!("WebSocket error: {}", e);
                                break;
                            }
                            Ok(None) => {
                                info!("WebSocket stream ended");
                                break;
                            }
                            Err(_) => {
                                // Timeout - send ping to keep alive
                                debug!("Read timeout, sending ping");
                                if let Err(e) = write.send(Message::Ping(vec![].into())).await {
                                    warn!("Failed to send ping: {}", e);
                                    break;
                                }
                            }
                        }
                    }

                    // Flush file before reconnect
                    file.flush().await?;
                }
                Err(e) => {
                    error!("Connection failed: {}", e);
                }
            }

            // Check shutdown before reconnect
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            // Exponential backoff
            warn!("Reconnecting in {} seconds...", backoff_secs);
            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
        }

        // Final flush
        file.flush().await?;

        info!(
            "User client stopped. Total: {}, Parsed: {}, Unknown: {}",
            stats.total_messages, stats.parsed_ok, stats.unknown_type_count
        );

        Ok(stats)
    }

    /// Connect and subscribe to the user channel
    async fn connect_and_subscribe(
        &self,
    ) -> Result<(
        futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            Message,
        >,
        futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
    )> {
        info!("Connecting to {}", self.endpoint);

        let (ws_stream, response) =
            connect_async(&self.endpoint).await.context("WebSocket connection failed")?;

        debug!("WebSocket connected, status: {}", response.status());

        let (mut write, read) = ws_stream.split();

        // Send subscription request with auth
        let auth = WsAuth::from(&self.credentials);
        let subscribe_req = SubscribeRequest::user(auth, self.market_ids.clone());
        let subscribe_json = serde_json::to_string(&subscribe_req)?;

        info!("Subscribing to {} markets with authentication", self.market_ids.len());
        // Don't log the full request as it contains credentials
        debug!("Subscribe request: [REDACTED - contains auth]");

        write
            .send(Message::Text(subscribe_json.into()))
            .await
            .context("Failed to send subscribe request")?;

        Ok((write, read))
    }
}

/// Generate timestamped output filename
pub fn generate_user_output_filename() -> String {
    let now = Utc::now();
    format!("ws_user_{}.jsonl", now.format("%Y%m%d_%H%M%S"))
}

/// Smoke test helper - runs basic connectivity verification
/// Requires environment variables: POLY_API_KEY, POLY_API_SECRET, POLY_API_PASSPHRASE
pub async fn smoke_test_user_ws(market_ids: Vec<String>) -> Result<MessageStats> {
    let credentials = ApiCredentials::from_env()
        .context("Missing credentials. Set POLY_API_KEY, POLY_API_SECRET, POLY_API_PASSPHRASE")?;

    if !credentials.is_valid() {
        anyhow::bail!("Invalid credentials - one or more fields are empty");
    }

    let client = UserWsClient::new(credentials, market_ids.clone());
    let shutdown = Arc::new(AtomicBool::new(false));

    let output_file = generate_user_output_filename();
    let output_path = Path::new(&output_file);

    info!("=== User WebSocket Smoke Test ===");
    info!("Endpoint: {}", CLOB_WSS_ENDPOINT);
    info!("Markets: {:?}", market_ids);
    info!("Output: {}", output_path.display());

    // Run for a limited time or messages
    let shutdown_clone = shutdown.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        shutdown_clone.store(true, Ordering::Relaxed);
    });

    let stats = client.run(output_path, 50, shutdown).await?;

    info!("Smoke test complete");
    info!("  Total messages: {}", stats.total_messages);
    info!("  Parsed OK: {}", stats.parsed_ok);
    info!("  Unknown types: {}", stats.unknown_type_count);
    info!("  Last type: {:?}", stats.last_message_type);

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_filename_generation() {
        let filename = generate_user_output_filename();
        assert!(filename.starts_with("ws_user_"));
        assert!(filename.ends_with(".jsonl"));
    }
}
