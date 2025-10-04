//! MEV Config - Configuration management

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub network: NetworkConfig,
    pub bot: BotConfig,
    pub performance: Option<PerformanceConfig>,
    pub monitoring: MonitoringConfig,
    pub hyperliquid: Option<HyperLiquidConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperLiquidConfig {
    pub enabled: bool,
    // WebSocket configuration for market data
    pub ws_url: String,
    pub trading_pairs: Vec<String>,
    pub subscribe_orderbook: bool,
    // RPC configuration for blockchain operations
    pub rpc_url: String,
    pub polling_interval_ms: u64,
    // Reconnection settings (for WebSocket)
    pub reconnect_min_backoff_secs: u64,
    pub reconnect_max_backoff_secs: u64,
    pub max_consecutive_failures: u32,
    // Token mapping for cross-exchange arbitrage
    pub token_mapping: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub rpc_url: String,
    pub ws_url: String,
    pub chain_id: u64,
    pub private_key: String,
    pub mempool_mode: Option<String>,
    pub polling_interval_ms: Option<u64>,
    pub websocket_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    pub max_gas_price: u64,
    pub min_profit_threshold: f64,
    pub strategies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    pub worker_threads: usize,
    pub ring_buffer_size: usize,
    pub simulation_timeout_ms: u64,
    pub max_concurrent_simulations: usize,
    pub cpu_core_pinning: bool,
    pub core_ids: Vec<usize>,
    pub backpressure: BackpressureConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackpressureConfig {
    pub enabled: bool,
    pub drop_policy: String, // "oldest", "newest", "random"
    pub high_watermark: f64,
    pub low_watermark: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    pub prometheus_port: u16,
    pub log_level: String,
}

impl HyperLiquidConfig {
    /// Validates the HyperLiquid configuration
    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // Validate WebSocket URL
        if self.ws_url.is_empty() {
            anyhow::bail!("HyperLiquid ws_url cannot be empty when enabled");
        }
        if !self.ws_url.starts_with("ws://") && !self.ws_url.starts_with("wss://") {
            anyhow::bail!("HyperLiquid ws_url must start with ws:// or wss://, got: {}", self.ws_url);
        }

        // Validate RPC URL
        if self.rpc_url.is_empty() {
            anyhow::bail!("HyperLiquid rpc_url cannot be empty when enabled");
        }
        if !self.rpc_url.starts_with("http://") && !self.rpc_url.starts_with("https://") {
            anyhow::bail!("HyperLiquid rpc_url must start with http:// or https://, got: {}", self.rpc_url);
        }

        // Validate polling interval
        if self.polling_interval_ms == 0 {
            anyhow::bail!("HyperLiquid polling_interval_ms must be greater than 0");
        }
        if self.polling_interval_ms < 100 {
            eprintln!("Warning: HyperLiquid polling_interval_ms is very low ({}ms), this may cause rate limiting", self.polling_interval_ms);
        }

        // Validate trading pairs
        if self.trading_pairs.is_empty() {
            anyhow::bail!("HyperLiquid trading_pairs cannot be empty when enabled");
        }

        Ok(())
    }
}

impl Config {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = serde_yaml::from_str(&content)?;
        
        // Validate HyperLiquid configuration if present
        if let Some(ref hyperliquid) = config.hyperliquid {
            hyperliquid.validate()?;
        }
        
        Ok(config)
    }

    pub fn load_from_env() -> anyhow::Result<Self> {
        dotenv::dotenv().ok();
        let config = Config {
            network: NetworkConfig {
                rpc_url: std::env::var("RPC_URL")
                    .unwrap_or_else(|_| "http://localhost:8545".to_string()),
                ws_url: std::env::var("WS_URL")
                    .unwrap_or_else(|_| "ws://localhost:8546".to_string()),
                chain_id: std::env::var("CHAIN_ID")
                    .unwrap_or_else(|_| "1".to_string())
                    .parse()
                    .unwrap_or(1),
                private_key: std::env::var("PRIVATE_KEY").unwrap_or_default(),
                mempool_mode: std::env::var("MEMPOOL_MODE").ok(),
                polling_interval_ms: std::env::var("POLLING_INTERVAL_MS")
                    .ok()
                    .and_then(|s| s.parse().ok()),
                websocket_timeout_secs: std::env::var("WEBSOCKET_TIMEOUT_SECS")
                    .ok()
                    .and_then(|s| s.parse().ok()),
            },
            bot: BotConfig {
                max_gas_price: std::env::var("MAX_GAS_PRICE")
                    .unwrap_or_else(|_| "100000000000".to_string())
                    .parse()
                    .unwrap_or(100_000_000_000),
                min_profit_threshold: std::env::var("MIN_PROFIT_THRESHOLD")
                    .unwrap_or_else(|_| "0.01".to_string())
                    .parse()
                    .unwrap_or(0.01),
                strategies: vec!["arbitrage".to_string(), "sandwich".to_string()],
            },
            performance: Some(PerformanceConfig {
                worker_threads: std::env::var("WORKER_THREADS")
                    .unwrap_or_else(|_| "4".to_string())
                    .parse()
                    .unwrap_or(4),
                ring_buffer_size: std::env::var("RING_BUFFER_SIZE")
                    .unwrap_or_else(|_| "10000".to_string())
                    .parse()
                    .unwrap_or(10000),
                simulation_timeout_ms: std::env::var("SIMULATION_TIMEOUT_MS")
                    .unwrap_or_else(|_| "100".to_string())
                    .parse()
                    .unwrap_or(100),
                max_concurrent_simulations: std::env::var("MAX_CONCURRENT_SIMULATIONS")
                    .unwrap_or_else(|_| "50".to_string())
                    .parse()
                    .unwrap_or(50),
                cpu_core_pinning: std::env::var("CPU_CORE_PINNING")
                    .unwrap_or_else(|_| "false".to_string())
                    .parse()
                    .unwrap_or(false),
                core_ids: vec![0, 1, 2, 3],
                backpressure: BackpressureConfig {
                    enabled: std::env::var("BACKPRESSURE_ENABLED")
                        .unwrap_or_else(|_| "true".to_string())
                        .parse()
                        .unwrap_or(true),
                    drop_policy: std::env::var("DROP_POLICY")
                        .unwrap_or_else(|_| "oldest".to_string()),
                    high_watermark: std::env::var("HIGH_WATERMARK")
                        .unwrap_or_else(|_| "0.8".to_string())
                        .parse()
                        .unwrap_or(0.8),
                    low_watermark: std::env::var("LOW_WATERMARK")
                        .unwrap_or_else(|_| "0.6".to_string())
                        .parse()
                        .unwrap_or(0.6),
                },
            }),
            monitoring: MonitoringConfig {
                prometheus_port: std::env::var("PROMETHEUS_PORT")
                    .unwrap_or_else(|_| "9090".to_string())
                    .parse()
                    .unwrap_or(9090),
                log_level: std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
            },
            hyperliquid: None,
        };

        Ok(config)
    }
}
