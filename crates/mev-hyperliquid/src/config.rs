use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for HyperLiquid WebSocket integration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HyperLiquidConfig {
    /// Enable or disable HyperLiquid integration
    pub enabled: bool,
    
    /// WebSocket URL for HyperLiquid API
    pub ws_url: String,
    
    /// RPC URL for blockchain operations (optional)
    pub rpc_url: Option<String>,
    
    /// Polling interval in milliseconds for RPC state polling (optional, default: 1000ms)
    pub polling_interval_ms: Option<u64>,
    
    /// Private key for transaction signing (optional)
    pub private_key: Option<String>,
    
    /// List of trading pairs to monitor (e.g., ["BTC", "ETH", "SOL"])
    pub trading_pairs: Vec<String>,
    
    /// Subscribe to order book updates in addition to trades
    pub subscribe_orderbook: bool,
    
    /// Minimum backoff time in seconds for reconnection attempts
    pub reconnect_min_backoff_secs: u64,
    
    /// Maximum backoff time in seconds for reconnection attempts
    pub reconnect_max_backoff_secs: u64,
    
    /// Maximum number of consecutive connection failures before entering degraded state
    pub max_consecutive_failures: u32,
    
    /// Mapping of HyperLiquid coin symbols to EVM token addresses
    /// Example: {"BTC": "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"}
    pub token_mapping: HashMap<String, String>,
}

impl Default for HyperLiquidConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ws_url: "wss://api.hyperliquid.xyz/ws".to_string(),
            rpc_url: None,
            polling_interval_ms: Some(1000),
            private_key: None,
            trading_pairs: vec!["BTC".to_string(), "ETH".to_string()],
            subscribe_orderbook: false,
            reconnect_min_backoff_secs: 1,
            reconnect_max_backoff_secs: 60,
            max_consecutive_failures: 10,
            token_mapping: HashMap::new(),
        }
    }
}

impl HyperLiquidConfig {
    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate WebSocket URL
        if self.ws_url.is_empty() {
            return Err(anyhow!("WebSocket URL cannot be empty"));
        }
        
        if !self.ws_url.starts_with("ws://") && !self.ws_url.starts_with("wss://") {
            return Err(anyhow!(
                "WebSocket URL must start with ws:// or wss://, got: {}",
                self.ws_url
            ));
        }
        
        // Validate RPC URL if provided
        if let Some(ref rpc_url) = self.rpc_url {
            if rpc_url.is_empty() {
                return Err(anyhow!("RPC URL cannot be empty if provided"));
            }
            
            if !rpc_url.starts_with("http://") && !rpc_url.starts_with("https://") {
                return Err(anyhow!(
                    "RPC URL must start with http:// or https://, got: {}",
                    rpc_url
                ));
            }
        }
        
        // Validate polling interval if provided
        if let Some(interval) = self.polling_interval_ms {
            if interval == 0 {
                return Err(anyhow!("Polling interval must be greater than 0"));
            }
            
            if interval < 100 {
                return Err(anyhow!(
                    "Polling interval too low ({}ms). Minimum recommended: 100ms",
                    interval
                ));
            }
        }
        
        // Validate trading pairs
        if self.enabled && self.trading_pairs.is_empty() {
            return Err(anyhow!(
                "At least one trading pair must be configured when HyperLiquid is enabled"
            ));
        }
        
        for pair in &self.trading_pairs {
            if pair.is_empty() {
                return Err(anyhow!("Trading pair symbol cannot be empty"));
            }
            
            if pair.len() > 10 {
                return Err(anyhow!(
                    "Trading pair symbol '{}' is too long (max 10 characters)",
                    pair
                ));
            }
        }
        
        // Validate reconnection settings
        if self.reconnect_min_backoff_secs == 0 {
            return Err(anyhow!("Minimum backoff must be at least 1 second"));
        }
        
        if self.reconnect_max_backoff_secs < self.reconnect_min_backoff_secs {
            return Err(anyhow!(
                "Maximum backoff ({}) must be greater than or equal to minimum backoff ({})",
                self.reconnect_max_backoff_secs,
                self.reconnect_min_backoff_secs
            ));
        }
        
        if self.max_consecutive_failures == 0 {
            return Err(anyhow!(
                "Maximum consecutive failures must be at least 1"
            ));
        }
        
        // Validate token mapping addresses
        for (coin, address) in &self.token_mapping {
            if coin.is_empty() {
                return Err(anyhow!("Token mapping coin symbol cannot be empty"));
            }
            
            if address.is_empty() {
                return Err(anyhow!(
                    "Token mapping address for '{}' cannot be empty",
                    coin
                ));
            }
            
            // Basic Ethereum address validation (0x + 40 hex chars)
            if !address.starts_with("0x") || address.len() != 42 {
                return Err(anyhow!(
                    "Invalid Ethereum address for '{}': {}. Expected format: 0x followed by 40 hex characters",
                    coin,
                    address
                ));
            }
            
            // Validate hex characters
            if !address[2..].chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(anyhow!(
                    "Invalid Ethereum address for '{}': {}. Contains non-hex characters",
                    coin,
                    address
                ));
            }
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = HyperLiquidConfig::default();
        // Default config has enabled=false, so validation should pass even with empty token_mapping
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_empty_ws_url() {
        let mut config = HyperLiquidConfig::default();
        config.ws_url = String::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_ws_url_scheme() {
        let mut config = HyperLiquidConfig::default();
        config.ws_url = "http://api.hyperliquid.xyz/ws".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_trading_pairs_when_enabled() {
        let mut config = HyperLiquidConfig::default();
        config.enabled = true;
        config.trading_pairs = vec![];
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_trading_pairs_when_disabled() {
        let mut config = HyperLiquidConfig::default();
        config.enabled = false;
        config.trading_pairs = vec![];
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_backoff_settings() {
        let mut config = HyperLiquidConfig::default();
        config.reconnect_min_backoff_secs = 0;
        assert!(config.validate().is_err());

        let mut config = HyperLiquidConfig::default();
        config.reconnect_max_backoff_secs = 5;
        config.reconnect_min_backoff_secs = 10;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_max_failures() {
        let mut config = HyperLiquidConfig::default();
        config.max_consecutive_failures = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_valid_token_mapping() {
        let mut config = HyperLiquidConfig::default();
        config.token_mapping.insert(
            "BTC".to_string(),
            "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599".to_string(),
        );
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_token_address_format() {
        let mut config = HyperLiquidConfig::default();
        config.token_mapping.insert(
            "BTC".to_string(),
            "invalid_address".to_string(),
        );
        assert!(config.validate().is_err());

        let mut config = HyperLiquidConfig::default();
        config.token_mapping.insert(
            "BTC".to_string(),
            "0x123".to_string(), // Too short
        );
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_trading_pair_too_long() {
        let mut config = HyperLiquidConfig::default();
        config.enabled = true;
        config.trading_pairs = vec!["VERYLONGSYMBOL".to_string()];
        assert!(config.validate().is_err());
    }
}
