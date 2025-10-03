use clap::{Arg, Command};
use mev_config::Config;
use mev_core::MevCore;
use mev_mempool::MempoolService;
use mev_strategies::{ArbitrageStrategy, Strategy};
use tracing::{info, error};
use std::sync::Arc;

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
    
    // Parse mempool mode from config
    let mempool_mode = match config.network.mempool_mode.as_deref() {
        Some("websocket") => mev_mempool::MempoolMode::WebSocket,
        Some("polling") => mev_mempool::MempoolMode::Polling,
        _ => mev_mempool::MempoolMode::Auto, // Default to auto
    };
    
    let mempool_service = Arc::new(
        MempoolService::new(
            config.network.ws_url.clone(),
            config.network.rpc_url.clone(),
        )?
        .with_mode(mempool_mode)
        .with_polling_interval(config.network.polling_interval_ms.unwrap_or(300))
        .with_websocket_timeout(config.network.websocket_timeout_secs.unwrap_or(3))
    );
    
    // Initialize strategies
    let arbitrage_strategy = ArbitrageStrategy::new();
    
    info!("Initialized strategy: {}", arbitrage_strategy.name());

    // Start transaction consumer in background
    let consumer_service = Arc::clone(&mempool_service);
    let consumer_handle = tokio::spawn(async move {
        let consumer = consumer_service.get_consumer();
        let mut processed_count = 0u64;
        
        loop {
            // Consume transactions from ring buffer
            let transactions = consumer.consume_batch(10);
            
            if !transactions.is_empty() {
                processed_count += transactions.len() as u64;
                
                for tx in transactions {
                    info!(
                        tx_hash = %tx.transaction.hash,
                        target_type = ?tx.target_type,
                        processing_time_ms = tx.processing_time_ms,
                        "Consumed transaction from buffer"
                    );
                }
                
                if processed_count % 50 == 0 {
                    let (len, capacity, utilization, dropped) = consumer_service.get_buffer_stats();
                    info!(
                        consumed_total = processed_count,
                        buffer_len = len,
                        buffer_capacity = capacity,
                        buffer_utilization = utilization,
                        dropped_count = dropped,
                        "Consumer stats"
                    );
                }
            } else {
                // No transactions, sleep briefly
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }
    });

    info!("Starting mempool monitoring...");

    // Start services
    tokio::select! {
        result = mempool_service.start() => {
            if let Err(e) = result {
                error!("Mempool service error: {}", e);
            }
        }
        result = consumer_handle => {
            if let Err(e) = result {
                error!("Consumer task error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received shutdown signal");
        }
    }

    info!("MEV Bot shutting down");
    Ok(())
}
