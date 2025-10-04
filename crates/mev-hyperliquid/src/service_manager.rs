//! Service manager for coordinating HyperLiquid WebSocket and RPC services
//!
//! This module provides the `HyperLiquidServiceManager` which orchestrates both
//! the WebSocket service (for real-time market data) and the RPC service (for
//! blockchain operations). It manages the lifecycle of both services and provides
//! unified channels for consuming market data and state updates.

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::config::HyperLiquidConfig;
use crate::metrics::HyperLiquidMetrics;
use crate::rpc_service::{HyperLiquidRpcService, RpcConfig};
use crate::types::{HyperLiquidMessage, MarketDataEvent, MessageData, StateUpdateEvent};
use crate::websocket::HyperLiquidWsService;

/// Service manager for HyperLiquid integration
///
/// Coordinates both WebSocket and RPC services, providing a unified interface
/// for consuming market data and blockchain state updates.
pub struct HyperLiquidServiceManager {
    /// WebSocket service for market data streaming
    ws_service: Arc<HyperLiquidWsService>,
    
    /// RPC service for blockchain operations
    rpc_service: Arc<HyperLiquidRpcService>,
    
    /// Configuration
    config: HyperLiquidConfig,
    
    /// Metrics collector
    metrics: Arc<HyperLiquidMetrics>,
    
    /// Shutdown signal
    shutdown: Arc<AtomicBool>,
    
    /// WebSocket service task handle
    ws_task: Option<JoinHandle<Result<()>>>,
    
    /// RPC polling task handle
    rpc_task: Option<JoinHandle<Result<()>>>,
    
    /// Channel receiver for market data events
    market_data_rx: Option<mpsc::UnboundedReceiver<MarketDataEvent>>,
    
    /// Channel receiver for state update events
    state_update_rx: Option<mpsc::UnboundedReceiver<StateUpdateEvent>>,
}

impl HyperLiquidServiceManager {
    /// Create a new HyperLiquidServiceManager
    ///
    /// # Arguments
    /// * `config` - HyperLiquid configuration including WebSocket and RPC settings
    /// * `metrics` - Metrics collector for monitoring
    ///
    /// # Returns
    /// A new service manager instance (services not yet started)
    ///
    /// # Errors
    /// Returns error if:
    /// - Configuration validation fails
    /// - WebSocket service creation fails
    /// - RPC service creation fails
    pub fn new(config: HyperLiquidConfig, metrics: Arc<HyperLiquidMetrics>) -> Result<Self> {
        info!("Creating HyperLiquid service manager");
        
        // Validate configuration
        config.validate()
            .context("Invalid HyperLiquid configuration")?;
        
        // Create channels for market data (from WebSocket)
        let (ws_message_tx, ws_message_rx) = mpsc::unbounded_channel::<HyperLiquidMessage>();
        let (market_data_tx, market_data_rx) = mpsc::unbounded_channel::<MarketDataEvent>();
        
        // Create channels for state updates (from RPC)
        let (state_update_tx, state_update_rx) = mpsc::unbounded_channel::<StateUpdateEvent>();
        
        // Create WebSocket service
        let ws_service = HyperLiquidWsService::new(
            config.clone(),
            ws_message_tx,
            Arc::clone(&metrics),
        )
        .context("Failed to create WebSocket service")?;
        
        // Create RPC service configuration
        let rpc_config = RpcConfig::new(
            config.rpc_url.clone().unwrap_or_default(),
            config.polling_interval_ms.unwrap_or(1000),
        )
        .with_timeout(10)
        .with_private_key(config.private_key.clone().unwrap_or_default());
        
        // Create RPC service
        let rpc_service = HyperLiquidRpcService::new(
            rpc_config,
            Some(state_update_tx),
        );
        
        // Spawn task to convert WebSocket messages to MarketDataEvent
        let market_data_tx_clone = market_data_tx.clone();
        tokio::spawn(async move {
            Self::process_ws_messages(ws_message_rx, market_data_tx_clone).await;
        });
        
        let shutdown = Arc::new(AtomicBool::new(false));
        
        Ok(Self {
            ws_service: Arc::new(ws_service),
            rpc_service: Arc::new(rpc_service),
            config,
            metrics,
            shutdown,
            ws_task: None,
            rpc_task: None,
            market_data_rx: Some(market_data_rx),
            state_update_rx: Some(state_update_rx),
        })
    }
    
    /// Process WebSocket messages and convert to MarketDataEvent
    async fn process_ws_messages(
        mut ws_message_rx: mpsc::UnboundedReceiver<HyperLiquidMessage>,
        market_data_tx: mpsc::UnboundedSender<MarketDataEvent>,
    ) {
        while let Some(message) = ws_message_rx.recv().await {
            // Convert HyperLiquidMessage to MarketDataEvent
            let market_event = match message.data {
                MessageData::Trade(trade_data) => {
                    Some(MarketDataEvent::Trade(trade_data))
                }
                MessageData::OrderBook(orderbook_data) => {
                    Some(MarketDataEvent::OrderBook(orderbook_data))
                }
                MessageData::SubscriptionResponse(_) | MessageData::Error(_) => {
                    // Skip subscription responses and errors
                    None
                }
            };
            
            if let Some(event) = market_event {
                if let Err(e) = market_data_tx.send(event) {
                    error!("Failed to send market data event: {}", e);
                    break;
                }
            }
        }
    }
    
    /// Start both WebSocket and RPC services
    ///
    /// This method starts both services in parallel. The WebSocket service will
    /// stream real-time market data, while the RPC service will poll blockchain
    /// state at the configured interval.
    ///
    /// Both services will run until `stop()` is called or an unrecoverable error occurs.
    ///
    /// # Returns
    /// * `Ok(())` if both services start successfully
    /// * `Err` if either service fails to start
    ///
    /// # Errors
    /// Returns error if:
    /// - RPC connection fails
    /// - WebSocket service fails to start
    /// - Services are already running
    pub async fn start(&mut self) -> Result<()> {
        if self.ws_task.is_some() || self.rpc_task.is_some() {
            return Err(anyhow::anyhow!("Services are already running"));
        }
        
        info!("Starting HyperLiquid service manager");
        
        // Start WebSocket service in background task
        let ws_service = Arc::clone(&self.ws_service);
        let ws_task = tokio::spawn(async move {
            ws_service.start().await
        });
        
        info!("WebSocket service started");
        
        // Start RPC polling service in background task
        let rpc_service_clone = Arc::clone(&self.rpc_service);
        let shutdown_clone = Arc::clone(&self.shutdown);
        let metrics_clone = Arc::clone(&self.metrics);
        let polling_interval_ms = self.config.polling_interval_ms.unwrap_or(1000);
        
        let rpc_task = tokio::spawn(async move {
            Self::run_rpc_polling(
                rpc_service_clone,
                shutdown_clone,
                metrics_clone,
                polling_interval_ms,
            )
            .await
        });
        
        info!("RPC polling service started");
        
        self.ws_task = Some(ws_task);
        self.rpc_task = Some(rpc_task);
        
        info!("HyperLiquid service manager started successfully");
        
        Ok(())
    }
    
    /// Run RPC polling loop
    async fn run_rpc_polling(
        rpc_service: Arc<HyperLiquidRpcService>,
        shutdown: Arc<AtomicBool>,
        metrics: Arc<HyperLiquidMetrics>,
        polling_interval_ms: u64,
    ) -> Result<()> {
        use tokio::time::{interval, Duration};
        
        // Set polling interval metric
        let interval_secs = polling_interval_ms / 1000;
        metrics.set_state_poll_interval("rpc", interval_secs as i64);
        
        info!(
            "Starting RPC polling loop with interval: {}ms",
            polling_interval_ms
        );
        
        // Note: RPC service connection is established lazily on first request
        // due to the architecture of the Provider. We don't need explicit connect() call.
        
        let mut poll_interval = interval(Duration::from_millis(polling_interval_ms));
        poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        
        let mut last_successful_poll = std::time::Instant::now();
        
        loop {
            // Check for shutdown signal
            if shutdown.load(Ordering::Relaxed) {
                info!("RPC polling loop received shutdown signal");
                break;
            }
            
            poll_interval.tick().await;
            
            // Poll blockchain state
            let poll_start = std::time::Instant::now();
            match rpc_service.poll_blockchain_state().await {
                Ok(_state) => {
                    let duration = poll_start.elapsed();
                    
                    // Update metrics
                    metrics.inc_rpc_calls("poll_blockchain_state", "success");
                    metrics.record_rpc_call_duration("poll_blockchain_state", duration);
                    
                    // Update freshness metric
                    last_successful_poll = std::time::Instant::now();
                    metrics.set_state_freshness("rpc", 0);
                    
                    if duration.as_millis() > 100 {
                        warn!(
                            "Blockchain state polling took {}ms (target: <100ms)",
                            duration.as_millis()
                        );
                    }
                }
                Err(e) => {
                    let duration = poll_start.elapsed();
                    
                    // Update metrics
                    metrics.inc_rpc_calls("poll_blockchain_state", "error");
                    metrics.inc_rpc_errors("poll_blockchain_state", "poll_failed");
                    metrics.record_rpc_call_duration("poll_blockchain_state", duration);
                    
                    // Update freshness metric (time since last successful poll)
                    let freshness_secs = last_successful_poll.elapsed().as_secs();
                    metrics.set_state_freshness("rpc", freshness_secs as i64);
                    
                    error!("Failed to poll blockchain state: {:#}", e);
                    
                    // Continue polling despite errors (graceful degradation)
                }
            }
        }
        
        info!("RPC polling loop stopped");
        Ok(())
    }
    
    /// Stop both services gracefully
    ///
    /// This method signals both services to shut down and waits for them to complete.
    /// It ensures all resources are properly cleaned up.
    ///
    /// # Returns
    /// * `Ok(())` if both services stop successfully
    /// * `Err` if there are errors during shutdown (services will still be stopped)
    pub async fn stop(&mut self) -> Result<()> {
        info!("Stopping HyperLiquid service manager");
        
        // Set shutdown signal
        self.shutdown.store(true, Ordering::Relaxed);
        
        let mut errors = Vec::new();
        
        // Wait for RPC task to complete
        if let Some(rpc_task) = self.rpc_task.take() {
            match rpc_task.await {
                Ok(Ok(())) => {
                    info!("RPC service stopped successfully");
                }
                Ok(Err(e)) => {
                    error!("RPC service stopped with error: {:#}", e);
                    errors.push(format!("RPC service error: {}", e));
                }
                Err(e) => {
                    error!("RPC task panicked: {:#}", e);
                    errors.push(format!("RPC task panic: {}", e));
                }
            }
        }
        
        // Wait for WebSocket task to complete
        if let Some(ws_task) = self.ws_task.take() {
            match ws_task.await {
                Ok(Ok(())) => {
                    info!("WebSocket service stopped successfully");
                }
                Ok(Err(e)) => {
                    error!("WebSocket service stopped with error: {:#}", e);
                    errors.push(format!("WebSocket service error: {}", e));
                }
                Err(e) => {
                    error!("WebSocket task panicked: {:#}", e);
                    errors.push(format!("WebSocket task panic: {}", e));
                }
            }
        }
        
        if errors.is_empty() {
            info!("HyperLiquid service manager stopped successfully");
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "Service manager stopped with errors: {}",
                errors.join(", ")
            ))
        }
    }
    
    /// Get the market data receiver channel
    ///
    /// This channel receives `MarketDataEvent` from the WebSocket service.
    /// Can only be called once - subsequent calls will return None.
    ///
    /// # Returns
    /// * `Some(receiver)` if available
    /// * `None` if already taken
    pub fn get_market_data_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<MarketDataEvent>> {
        self.market_data_rx.take()
    }
    
    /// Get the state update receiver channel
    ///
    /// This channel receives `StateUpdateEvent` from the RPC service.
    /// Can only be called once - subsequent calls will return None.
    ///
    /// # Returns
    /// * `Some(receiver)` if available
    /// * `None` if already taken
    pub fn get_state_update_receiver(&mut self) -> Option<mpsc::UnboundedReceiver<StateUpdateEvent>> {
        self.state_update_rx.take()
    }
    
    /// Check if services are running
    pub fn is_running(&self) -> bool {
        self.ws_task.is_some() && self.rpc_task.is_some()
    }
    
    /// Get reference to the RPC service
    ///
    /// This allows direct access to RPC operations like transaction submission
    pub fn rpc_service(&self) -> Arc<HyperLiquidRpcService> {
        Arc::clone(&self.rpc_service)
    }
    
    /// Get reference to the WebSocket service
    pub fn ws_service(&self) -> Arc<HyperLiquidWsService> {
        Arc::clone(&self.ws_service)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn create_test_config() -> HyperLiquidConfig {
        HyperLiquidConfig {
            enabled: true,
            ws_url: "wss://api.hyperliquid.xyz/ws".to_string(),
            trading_pairs: vec!["BTC".to_string(), "ETH".to_string()],
            subscribe_orderbook: false,
            reconnect_min_backoff_secs: 1,
            reconnect_max_backoff_secs: 60,
            max_consecutive_failures: 10,
            token_mapping: std::collections::HashMap::new(),
            rpc_url: Some("https://test-rpc.example.com".to_string()),
            polling_interval_ms: Some(1000),
            private_key: None,
        }
    }
    
    #[tokio::test]
    async fn test_service_manager_creation() {
        let config = create_test_config();
        let metrics = Arc::new(HyperLiquidMetrics::new_or_default());
        
        let manager = HyperLiquidServiceManager::new(config, metrics);
        assert!(manager.is_ok());
        
        let manager = manager.unwrap();
        assert!(!manager.is_running());
    }
    
    #[test]
    fn test_service_manager_invalid_config() {
        let mut config = create_test_config();
        config.ws_url = "invalid-url".to_string();
        
        let metrics = Arc::new(HyperLiquidMetrics::new_or_default());
        let manager = HyperLiquidServiceManager::new(config, metrics);
        
        assert!(manager.is_err());
    }
    
    #[tokio::test]
    async fn test_get_receivers() {
        let config = create_test_config();
        let metrics = Arc::new(HyperLiquidMetrics::new_or_default());
        
        let mut manager = HyperLiquidServiceManager::new(config, metrics).unwrap();
        
        // First call should return Some
        let market_data_rx = manager.get_market_data_receiver();
        assert!(market_data_rx.is_some());
        
        // Second call should return None
        let market_data_rx = manager.get_market_data_receiver();
        assert!(market_data_rx.is_none());
        
        // Same for state updates
        let state_update_rx = manager.get_state_update_receiver();
        assert!(state_update_rx.is_some());
        
        let state_update_rx = manager.get_state_update_receiver();
        assert!(state_update_rx.is_none());
    }
    
    #[tokio::test]
    async fn test_service_manager_lifecycle() {
        let config = create_test_config();
        let metrics = Arc::new(HyperLiquidMetrics::new_or_default());
        
        let manager = HyperLiquidServiceManager::new(config, metrics).unwrap();
        
        // Initially not running
        assert!(!manager.is_running());
        
        // Note: We can't actually start the services in tests without a real RPC endpoint
        // This test just verifies the structure is correct
        
        // Verify we can access the services
        let _rpc_service = manager.rpc_service();
        let _ws_service = manager.ws_service();
    }
}
