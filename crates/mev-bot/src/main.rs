use clap::{Arg, Command};
use mev_config::Config;
use mev_core::{MevCore, ParsedTransaction, PrometheusMetrics, TargetType, Transaction};
use mev_strategies::{ArbitrageStrategy, Strategy, StrategyEngine, StrategyEngineConfig};
use mev_hyperliquid::{
    HyperLiquidConfig, HyperLiquidMetrics, HyperLiquidServiceManager,
    MarketDataEvent, StateUpdateEvent, TradeDataAdapter,
};
use tracing::{info, error, warn, debug};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment variables from .env file
    dotenv::dotenv().ok();
    
    // Initialize structured JSON logging
    tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .json()
        .init();

    let matches = Command::new("mev-bot")
        .version("0.1.0")
        .about("MEV Bot - Maximum Extractable Value Bot")
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .value_name("FILE")
                .help("Sets a custom config file")
        )
        .arg(
            Arg::new("profile")
                .short('p')
                .long("profile")
                .value_name("PROFILE")
                .help("Sets the profile (dev/testnet/mainnet)")
                .default_value("dev")
        )
        .get_matches();

    info!("Starting MEV Bot v{}", MevCore::version());

    // Load configuration
    let config = if let Some(config_file) = matches.get_one::<String>("config") {
        Config::load_from_file(config_file)?
    } else {
        Config::load_from_env()?
    };

    info!(
        max_gas_price = config.bot.max_gas_price,
        min_profit_threshold = config.bot.min_profit_threshold,
        "Loaded configuration"
    );

    // Initialize core components
    let _core = MevCore::new("MEV Bot".to_string());
    
    // Initialize metrics (shared across all services)
    let metrics = Arc::new(PrometheusMetrics::new()?);
    
    // Start monitoring server in background
    let monitoring_port = config.monitoring.prometheus_port;
    let monitoring_metrics = metrics.clone();
    let monitoring_handle = tokio::spawn(async move {
        use mev_core::metrics::MetricsServer;
        let metrics_server = MetricsServer::new(monitoring_metrics, monitoring_port);
        if let Err(e) = metrics_server.start().await {
            error!("Monitoring server error: {}", e);
        }
    });
    
    info!("Monitoring server starting on port {}", monitoring_port);
    
    // Initialize strategy engine
    let strategy_engine = Arc::new(StrategyEngine::new(
        StrategyEngineConfig::default(),
        metrics.clone(),
    ));
    
    // Register strategies
    let arbitrage_strategy = Box::new(ArbitrageStrategy::new());
    info!("Registering strategy: {}", arbitrage_strategy.name());
    strategy_engine.register_strategy(arbitrage_strategy).await?;
    
    // Mempool monitoring disabled - using HyperLiquid native API only

    // Initialize HyperLiquid integration if enabled
    // HyperLiquid uses a dual-channel architecture:
    // - WebSocket for real-time market data (trades, order books)
    // - RPC for blockchain state polling and transaction submission
    // Note: HyperLiquid EVM does not support mempool queries, so we don't use the mempool service for it.
    let hyperliquid_manager = if let Some(hl_config) = &config.hyperliquid {
        if hl_config.enabled {
            info!("HyperLiquid integration enabled, initializing dual-channel architecture (WebSocket + RPC)...");
            
            // Load private key from environment variable if config is empty
            let private_key = if hl_config.private_key.is_empty() {
                std::env::var("PRIVATE_KEY").unwrap_or_default()
            } else {
                hl_config.private_key.clone()
            };
            
            // Convert config to HyperLiquid config type
            // Note: The mev-hyperliquid crate has its own config type with optional fields
            let hl_config_typed = HyperLiquidConfig {
                enabled: hl_config.enabled,
                ws_url: hl_config.ws_url.clone(),
                rpc_url: Some(hl_config.rpc_url.clone()),
                polling_interval_ms: Some(hl_config.polling_interval_ms),
                private_key: Some(private_key), // Use private key from environment or config
                trading_pairs: hl_config.trading_pairs.clone(),
                subscribe_orderbook: hl_config.subscribe_orderbook,
                reconnect_min_backoff_secs: hl_config.reconnect_min_backoff_secs,
                reconnect_max_backoff_secs: hl_config.reconnect_max_backoff_secs,
                max_consecutive_failures: hl_config.max_consecutive_failures,
                token_mapping: hl_config.token_mapping.clone(),
            };
            
            // Initialize HyperLiquid metrics
            let hl_metrics = Arc::new(HyperLiquidMetrics::new()?);
            
            // Create service manager
            let mut service_manager = HyperLiquidServiceManager::new(
                hl_config_typed.clone(),
                hl_metrics.clone(),
            )?;
            
            info!(
                trading_pairs = ?hl_config.trading_pairs,
                ws_url = %hl_config.ws_url,
                rpc_url = ?hl_config_typed.rpc_url,
                polling_interval_ms = ?hl_config_typed.polling_interval_ms,
                "HyperLiquid service manager initialized"
            );
            
            // Get receivers before starting (they can only be taken once)
            let mut market_data_rx = service_manager.get_market_data_receiver()
                .expect("Failed to get market data receiver");
            let mut state_update_rx = service_manager.get_state_update_receiver()
                .expect("Failed to get state update receiver");
            
            // Start the service manager (starts both WebSocket and RPC services)
            service_manager.start().await?;
            
            // Initialize trade data adapter for converting market data to transactions
            let adapter = Arc::new(TradeDataAdapter::new_with_metrics(
                998, // HyperLiquid EVM chain ID
                hl_config.token_mapping.clone(),
                hl_metrics.clone(),
            )?);
            
            // Spawn task to process market data events
            let market_data_strategy_engine = strategy_engine.clone();
            let market_data_metrics = metrics.clone();
            let market_data_handle = tokio::spawn(async move {
                let mut processed_count = 0u64;
                
                info!("Starting HyperLiquid market data processor");
                
                while let Some(market_event) = market_data_rx.recv().await {
                    match market_event {
                        MarketDataEvent::Trade(trade_data) => {
                            processed_count += 1;
                            
                            debug!(
                                coin = %trade_data.coin,
                                side = ?trade_data.side,
                                price = trade_data.price,
                                size = trade_data.size,
                                "Received HyperLiquid trade"
                            );
                            
                            // Adapt trade to ParsedTransaction format
                            match adapter.adapt_trade(&trade_data) {
                                Ok(adapted_event) => {
                                    // Convert AdaptedTradeEvent to ParsedTransaction
                                    let parsed_tx = ParsedTransaction {
                                        transaction: Transaction {
                                            hash: adapted_event.hash.clone(),
                                            from: format!("{:?}", adapted_event.from),
                                            to: Some(format!("{:?}", adapted_event.to)),
                                            value: adapted_event.value.to_string(),
                                            gas_price: adapted_event.gas_price.to_string(),
                                            gas_limit: "0".to_string(),
                                            nonce: 0,
                                            input: "0x".to_string(),
                                            timestamp: chrono::DateTime::from_timestamp(
                                                adapted_event.timestamp as i64,
                                                0
                                            ).unwrap_or_else(chrono::Utc::now),
                                        },
                                        decoded_input: None,
                                        target_type: TargetType::OrderBook,
                                        processing_time_ms: 0,
                                    };
                                    
                                    // Record transaction processing metrics
                                    market_data_metrics.inc_transactions_processed("hyperliquid", "orderbook", false);
                                    
                                    // Evaluate with strategy engine
                                    match market_data_strategy_engine.evaluate_transaction(&parsed_tx).await {
                                        Ok(opportunities) => {
                                            if !opportunities.is_empty() {
                                                info!(
                                                    trade_id = %trade_data.trade_id,
                                                    coin = %trade_data.coin,
                                                    opportunity_count = opportunities.len(),
                                                    "Found MEV opportunities from HyperLiquid trade"
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            warn!(
                                                trade_id = %trade_data.trade_id,
                                                error = %e,
                                                "Failed to evaluate HyperLiquid trade"
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(
                                        trade_id = %trade_data.trade_id,
                                        coin = %trade_data.coin,
                                        error = %e,
                                        "Failed to adapt HyperLiquid trade"
                                    );
                                }
                            }
                            
                            if processed_count % 100 == 0 {
                                info!(
                                    processed_total = processed_count,
                                    "HyperLiquid market data processor stats"
                                );
                                
                                // Update buffer utilization (simulate based on processing rate)
                                let utilization = if processed_count > 1000 { 75 } else { 25 };
                                market_data_metrics.set_buffer_utilization("trade_processor", utilization);
                            }
                        }
                        MarketDataEvent::OrderBook(orderbook_data) => {
                            debug!(
                                coin = %orderbook_data.coin,
                                bids = orderbook_data.bids.len(),
                                asks = orderbook_data.asks.len(),
                                "Received HyperLiquid order book update"
                            );
                            // Order book processing can be added here in the future
                        }
                    }
                }
                
                info!("HyperLiquid market data processor stopped");
            });
            
            // Spawn task to process state update events
            let state_update_handle = tokio::spawn(async move {
                let mut processed_count = 0u64;
                
                info!("Starting HyperLiquid state update processor");
                
                while let Some(state_event) = state_update_rx.recv().await {
                    processed_count += 1;
                    
                    match &state_event {
                        StateUpdateEvent::BlockNumber(block_number) => {
                            debug!(
                                block_number = block_number,
                                "Received blockchain state update: new block"
                            );
                        }
                        StateUpdateEvent::StateSnapshot(state) => {
                            debug!(
                                block_number = state.block_number,
                                timestamp = state.timestamp,
                                token_prices_count = state.token_prices.len(),
                                "Received blockchain state snapshot"
                            );
                        }
                        StateUpdateEvent::TokenPrice { token, price } => {
                            debug!(
                                token = ?token,
                                price = ?price,
                                "Received token price update"
                            );
                        }
                        StateUpdateEvent::TransactionConfirmed { tx_hash, block_number, gas_used, status } => {
                            info!(
                                tx_hash = ?tx_hash,
                                block_number = block_number,
                                gas_used = ?gas_used,
                                status = status,
                                "Transaction confirmed"
                            );
                        }
                        StateUpdateEvent::TransactionFailed { tx_hash, reason } => {
                            warn!(
                                tx_hash = ?tx_hash,
                                reason = %reason,
                                "Transaction failed"
                            );
                        }
                        _ => {
                            debug!("Received other state update event");
                        }
                    }
                    
                    // State updates can be used for opportunity validation
                    // For now, we just log them. Future implementation can combine
                    // market data with state updates for more sophisticated strategies.
                    
                    if processed_count % 50 == 0 {
                        info!(
                            processed_total = processed_count,
                            "HyperLiquid state update processor stats"
                        );
                    }
                }
                
                info!("HyperLiquid state update processor stopped");
            });
            
            Some((service_manager, market_data_handle, state_update_handle))
        } else {
            info!("HyperLiquid integration disabled in configuration");
            None
        }
    } else {
        info!("HyperLiquid configuration not found, skipping integration");
        None
    };

    // Log architecture information
    if config.hyperliquid.as_ref().map_or(false, |c| c.enabled) {
        info!(
            "Architecture: HyperLiquid uses dual-channel (WebSocket for market data + RPC for blockchain operations). \
             Mempool monitoring is not supported on HyperLiquid EVM."
        );
    }

    info!("Starting services...");

    // Start services and wait for shutdown
    tokio::select! {
        result = monitoring_handle => {
            if let Err(e) = result {
                error!("Monitoring server task error: {}", e);
            }
        }
        _ = async {
            // This branch handles HyperLiquid task completion
            // We don't actually await the handles here since they should run until shutdown
            std::future::pending::<()>().await
        } => {
            // This will never complete, just here for structure
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received shutdown signal");
        }
    }

    info!("MEV Bot shutting down");
    
    // Gracefully stop HyperLiquid service manager if it's running
    if let Some((mut service_manager, _, _)) = hyperliquid_manager {
        info!("Stopping HyperLiquid service manager...");
        if let Err(e) = service_manager.stop().await {
            error!("Error stopping HyperLiquid service manager: {}", e);
        } else {
            info!("HyperLiquid service manager stopped successfully");
        }
    }
    
    Ok(())
}
