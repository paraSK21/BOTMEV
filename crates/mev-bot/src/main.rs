use clap::{Arg, Command};
use mev_config::Config;
use mev_core::{MevCore, ParsedTransaction, PrometheusMetrics, TargetType, Transaction};
use mev_mempool::MempoolService;
use mev_strategies::{ArbitrageStrategy, Strategy, StrategyEngine, StrategyEngineConfig};
use mev_hyperliquid::{
    HyperLiquidConfig, HyperLiquidMetrics, HyperLiquidWsService, TradeDataAdapter,
    HyperLiquidMessage, MessageData,
};
use tracing::{info, error, warn, debug};
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
        chain_id = config.network.chain_id,
        rpc_url = %config.network.rpc_url,
        ws_url = %config.network.ws_url,
        max_gas_price = config.bot.max_gas_price,
        min_profit_threshold = config.bot.min_profit_threshold,
        "Loaded configuration"
    );

    // Initialize core components
    let _core = MevCore::new("MEV Bot".to_string());
    
    // Initialize metrics (shared across all services)
    let metrics = Arc::new(PrometheusMetrics::new()?);
    
    // Initialize strategy engine
    let strategy_engine = Arc::new(StrategyEngine::new(
        StrategyEngineConfig::default(),
        metrics.clone(),
    ));
    
    // Register strategies
    let arbitrage_strategy = Box::new(ArbitrageStrategy::new());
    info!("Registering strategy: {}", arbitrage_strategy.name());
    strategy_engine.register_strategy(arbitrage_strategy).await?;
    
    // Parse mempool mode from config
    let mempool_mode = match config.network.mempool_mode.as_deref() {
        Some("websocket") => mev_mempool::MempoolMode::WebSocket,
        Some("polling") => mev_mempool::MempoolMode::Polling,
        _ => mev_mempool::MempoolMode::Auto, // Default to auto
    };
    
    let mempool_service = Arc::new(
        MempoolService::new_with_metrics(
            config.network.ws_url.clone(),
            config.network.rpc_url.clone(),
            metrics.clone(),
        )?
        .with_mode(mempool_mode)
        .with_polling_interval(config.network.polling_interval_ms.unwrap_or(300))
        .with_websocket_timeout(config.network.websocket_timeout_secs.unwrap_or(3))
    );

    // Start transaction consumer for mempool in background
    let consumer_service = Arc::clone(&mempool_service);
    let mempool_strategy_engine = strategy_engine.clone();
    let consumer_handle = tokio::spawn(async move {
        let consumer = consumer_service.get_consumer();
        let mut processed_count = 0u64;
        
        loop {
            // Consume transactions from ring buffer
            let transactions = consumer.consume_batch(10);
            
            if !transactions.is_empty() {
                processed_count += transactions.len() as u64;
                
                for tx in transactions {
                    debug!(
                        tx_hash = %tx.transaction.hash,
                        target_type = ?tx.target_type,
                        processing_time_ms = tx.processing_time_ms,
                        "Consumed transaction from mempool buffer"
                    );
                    
                    // Evaluate transaction with strategy engine
                    match mempool_strategy_engine.evaluate_transaction(&tx).await {
                        Ok(opportunities) => {
                            if !opportunities.is_empty() {
                                info!(
                                    tx_hash = %tx.transaction.hash,
                                    opportunity_count = opportunities.len(),
                                    "Found MEV opportunities from mempool transaction"
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                tx_hash = %tx.transaction.hash,
                                error = %e,
                                "Failed to evaluate mempool transaction"
                            );
                        }
                    }
                }
                
                if processed_count % 50 == 0 {
                    let (len, capacity, utilization, dropped) = consumer_service.get_buffer_stats();
                    info!(
                        consumed_total = processed_count,
                        buffer_len = len,
                        buffer_capacity = capacity,
                        buffer_utilization = utilization,
                        dropped_count = dropped,
                        "Mempool consumer stats"
                    );
                }
            } else {
                // No transactions, sleep briefly
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }
    });

    // Initialize HyperLiquid integration if enabled
    let hyperliquid_handle = if let Some(hl_config) = &config.hyperliquid {
        if hl_config.enabled {
            info!("HyperLiquid integration enabled, initializing...");
            
            // Convert config to HyperLiquid config type
            let hl_config_typed = HyperLiquidConfig {
                enabled: hl_config.enabled,
                ws_url: hl_config.ws_url.clone(),
                trading_pairs: hl_config.trading_pairs.clone(),
                subscribe_orderbook: hl_config.subscribe_orderbook,
                reconnect_min_backoff_secs: hl_config.reconnect_min_backoff_secs,
                reconnect_max_backoff_secs: hl_config.reconnect_max_backoff_secs,
                max_consecutive_failures: hl_config.max_consecutive_failures,
                token_mapping: hl_config.token_mapping.clone(),
            };
            
            // Create message channel for HyperLiquid messages
            let (hl_tx, mut hl_rx) = mpsc::unbounded_channel::<HyperLiquidMessage>();
            
            // Initialize HyperLiquid metrics
            let hl_metrics = Arc::new(HyperLiquidMetrics::new()?);
            
            // Initialize HyperLiquid WebSocket service
            let hl_service = Arc::new(HyperLiquidWsService::new(
                hl_config_typed.clone(),
                hl_tx,
                hl_metrics.clone(),
            )?);
            
            // Initialize trade data adapter
            let adapter = Arc::new(TradeDataAdapter::new_with_metrics(
                config.network.chain_id,
                hl_config.token_mapping.clone(),
                hl_metrics.clone(),
            )?);
            
            info!(
                trading_pairs = ?hl_config.trading_pairs,
                ws_url = %hl_config.ws_url,
                "HyperLiquid service initialized"
            );
            
            // Start HyperLiquid WebSocket service
            let hl_service_clone = hl_service.clone();
            let hl_service_handle = tokio::spawn(async move {
                if let Err(e) = hl_service_clone.start().await {
                    error!("HyperLiquid service error: {}", e);
                }
            });
            
            // Start HyperLiquid message processor
            let hl_strategy_engine = strategy_engine.clone();
            let hl_processor_handle = tokio::spawn(async move {
                let mut processed_count = 0u64;
                
                while let Some(message) = hl_rx.recv().await {
                    match message.data {
                        MessageData::Trade(trade_data) => {
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
                                    
                                    // Evaluate with strategy engine
                                    match hl_strategy_engine.evaluate_transaction(&parsed_tx).await {
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
                                let active_subs = hl_service.active_subscriptions().await;
                                info!(
                                    processed_total = processed_count,
                                    active_subscriptions = active_subs,
                                    "HyperLiquid processor stats"
                                );
                            }
                        }
                        MessageData::OrderBook(orderbook_data) => {
                            debug!(
                                coin = %orderbook_data.coin,
                                bids = orderbook_data.bids.len(),
                                asks = orderbook_data.asks.len(),
                                "Received HyperLiquid order book update"
                            );
                            // Order book processing can be added here in the future
                        }
                        MessageData::SubscriptionResponse(_) | MessageData::Error(_) => {
                            // These are handled internally by the WebSocket service
                        }
                    }
                }
                
                info!("HyperLiquid message processor stopped");
            });
            
            Some((hl_service_handle, hl_processor_handle))
        } else {
            info!("HyperLiquid integration disabled in configuration");
            None
        }
    } else {
        info!("HyperLiquid configuration not found, skipping integration");
        None
    };

    info!("Starting services...");

    // Start services and wait for shutdown
    tokio::select! {
        result = mempool_service.start() => {
            if let Err(e) = result {
                error!("Mempool service error: {}", e);
            }
        }
        result = consumer_handle => {
            if let Err(e) = result {
                error!("Mempool consumer task error: {}", e);
            }
        }
        result = async {
            if let Some((service_handle, processor_handle)) = hyperliquid_handle {
                tokio::select! {
                    result = service_handle => {
                        if let Err(e) = result {
                            error!("HyperLiquid service task error: {}", e);
                        }
                    }
                    result = processor_handle => {
                        if let Err(e) = result {
                            error!("HyperLiquid processor task error: {}", e);
                        }
                    }
                }
            } else {
                // If HyperLiquid is not enabled, just wait indefinitely
                std::future::pending::<()>().await;
            }
            Ok::<(), anyhow::Error>(())
        } => {
            if let Err(e) = result {
                error!("HyperLiquid task error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received shutdown signal");
        }
    }

    info!("MEV Bot shutting down");
    Ok(())
}
