//! Authentication structures for Polymarket CLOB API
//!
//! # L1 vs L2 Authentication
//! - L1: Private key signs EIP-712 message (for credential creation)
//! - L2: API credentials (apiKey, secret, passphrase) for CLOB operations
//!
//! # WSS Authentication
//! Only `user` channel requires authentication.
//! Market channel is public and requires no auth.
//!
//! # Source
//! - Authentication: https://docs.polymarket.com/developers/CLOB/authentication
//! - WSS Auth: https://docs.polymarket.com/developers/CLOB/websocket/wss-auth

use serde::{Deserialize, Serialize};

/// L2 API credentials for CLOB operations
/// These are derived from L1 authentication (private key signing)
///
/// Source: https://docs.polymarket.com/developers/CLOB/authentication
#[derive(Clone, Serialize, Deserialize)]
pub struct ApiCredentials {
    /// CLOB API key
    pub api_key: String,
    /// CLOB API secret (used for HMAC-SHA256 signing)
    pub secret: String,
    /// CLOB API passphrase
    pub passphrase: String,
}

impl ApiCredentials {
    /// Create credentials from environment variables
    ///
    /// Expected env vars:
    /// - POLY_API_KEY
    /// - POLY_API_SECRET
    /// - POLY_API_PASSPHRASE
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("POLY_API_KEY").ok()?;
        let secret = std::env::var("POLY_API_SECRET").ok()?;
        let passphrase = std::env::var("POLY_API_PASSPHRASE").ok()?;

        Some(Self { api_key, secret, passphrase })
    }

    /// Check if credentials are present (non-empty)
    pub fn is_valid(&self) -> bool {
        !self.api_key.is_empty() && !self.secret.is_empty() && !self.passphrase.is_empty()
    }
}

impl std::fmt::Debug for ApiCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiCredentials")
            .field("api_key", &format!("{}...", &self.api_key.chars().take(8).collect::<String>()))
            .field("secret", &"[REDACTED]")
            .field("passphrase", &"[REDACTED]")
            .finish()
    }
}

/// Convert ApiCredentials to WsAuth for WebSocket subscription
impl From<&ApiCredentials> for crate::types::WsAuth {
    fn from(creds: &ApiCredentials) -> Self {
        crate::types::WsAuth {
            api_key: creds.api_key.clone(),
            secret: creds.secret.clone(),
            passphrase: creds.passphrase.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_debug_redacts_secrets() {
        let creds = ApiCredentials {
            api_key: "test_api_key_12345".to_string(),
            secret: "super_secret".to_string(),
            passphrase: "my_passphrase".to_string(),
        };

        let debug_str = format!("{:?}", creds);
        assert!(!debug_str.contains("super_secret"));
        assert!(!debug_str.contains("my_passphrase"));
        assert!(debug_str.contains("test_api"));
    }

    #[test]
    fn test_credentials_validity() {
        let valid = ApiCredentials {
            api_key: "key".to_string(),
            secret: "secret".to_string(),
            passphrase: "pass".to_string(),
        };
        assert!(valid.is_valid());

        let invalid = ApiCredentials {
            api_key: "".to_string(),
            secret: "secret".to_string(),
            passphrase: "pass".to_string(),
        };
        assert!(!invalid.is_valid());
    }
}
