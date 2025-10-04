//! Comprehensive demo of the pluggable strategy system

use anyhow::Result;
use mev_core::{
    ParsedTransaction, PrometheusMetrics, Transaction, TargetType, DecodedInput, DecodedParameter,
};
use mev_strategies::{
    BackrunStrategy, SandwichStrategy, ArbitrageStrategy,
    StrategyEngine, StrategyEngineConfig, ConfigManager, StrategyConfigFile,
    Strategy,
};
use std::{sync::Arc, time::Instant};
use tracing::{info, warn};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("Starting MEV Strategy Integration Demo");

    // Initialize metrics
    let metrics = Arc::new(PrometheusMetrics::new()?);

    // Create and configure strategy engine
    let engine_config = StrategyEngineConfig {
        max_concurrent_evaluations: 5,
        evaluation_timeout_ms: 100,
        opportunity_cache_size: 500,
        bundle_cache_ttl_seconds: 60,
        enable_parallel_evaluation: true,
    };

    let engine = StrategyEngine::new(engine_config, metrics.clone());

    // Load configuration from YAML
    let config_path = "config/strategies.yml".to_string();
    let mut config_manager = ConfigManager::new(config_path);
    
    // Load or create default configuration
    match config_manager.load_from_file() {
        Ok(()) => info!("Configuration loaded successfully"),
        Err(e) => {
            warn!("Failed to load config, using defaults: {}", e);
            // Create default config and save it
            config_manager = ConfigManager::new("config/strategies.yml".to_string());
        }
    }

    // Validate configuration
    let warnings = config_manager.validate_config()?;
    if !warnings.is_empty() {
        warn!("Configuration warnings:");
        for warning in warnings {
            warn!("  - {}", warning);
        }
    }

    // Register strategies with their configurations
    register_strategies(&engine, &config_manager).await?;

    // Start cleanup task
    engine.start_cleanup_task().await;

    // Run demonstration scenarios
    run_demo_scenarios(&engine).await?;

    // Display final statistics
    display_final_stats(&engine).await?;

    Ok(())
}

async fn register_strategies(engine: &StrategyEngine, config_manager: &ConfigManager) -> Result<()> {
    info!("Registering strategies with engine...");

    // Register backrun strategy
    if let Some(backrun_config) = config_manager.get_strategy_config("backrun") {
        let mut backrun_strategy = BackrunStrategy::new();
        backrun_strategy.update_config(backrun_config.clone())?;
        engine.register_strategy(Box::new(backrun_strategy)).await?;
        info!("Backrun strategy registered (enabled: {})", backrun_config.enabled);
    }

    // Register sandwich strategy
    if let Some(sandwich_config) = config_manager.get_strategy_config("sandwich") {
        let mut sandwich_strategy = SandwichStrategy::new();
        sandwich_strategy.update_config(sandwich_config.clone())?;
        engine.register_strategy(Box::new(sandwich_strategy)).await?;
        info!("Sandwich strategy registered (enabled: {})", sandwich_config.enabled);
    }

    // Register arbitrage strategy
    if let Some(arbitrage_config) = config_manager.get_strategy_config("arbitrage") {
        let mut arbitrage_strategy = ArbitrageStrategy::new();
        arbitrage_strategy.update_config(arbitrage_config.clone())?;
        engine.register_strategy(Box::new(arbitrage_strategy)).await?;
        info!("Arbitrage strategy registered (enabled: {})", arbitrage_config.enabled);
    }

    Ok(())
}

async fn run_demo_scenarios(engine: &StrategyEngine) -> Result<()> {
    info!("Running demonstration scenarios...");

    // Scenario 1: Large Uniswap V2 swap (good for backrun)
    let large_uniswap_tx = create_test_transaction(
        "0x1111111111111111111111111111111111111111",
        "5000000000000000000", // 5 ETH
        TargetType::UniswapV2,
        "swapExactETHForTokens",
        "0x7ff36ab5",
    );

    info!("=== Scenario 1: Large Uniswap V2 Swap ===");
    evaluate_and_process_transaction(engine, &large_uniswap_tx, "Large swap suitable for backrunning").await?;

    // Scenario 2: High slippage Uniswap transaction (good for sandwich)
    let high_slippage_tx = create_test_transaction(
        "0x2222222222222222222222222222222222222222",
        "2000000000000000000", // 2 ETH
        TargetType::UniswapV2,
        "swapExactETHForTokens",
        "0x7ff36ab5",
    );

    info!("=== Scenario 2: High Slippage Transaction ===");
    evaluate_and_process_transaction(engine, &high_slippage_tx, "High slippage transaction suitable for sandwiching").await?;

    // Scenario 3: Small transaction (should not trigger strategies)
    let small_tx = create_test_transaction(
        "0x3333333333333333333333333333333333333333",
        "10000000000000000", // 0.01 ETH
        TargetType::UniswapV2,
        "swapExactETHForTokens",
        "0x7ff36ab5",
    );

    info!("=== Scenario 3: Small Transaction ===");
    evaluate_and_process_transaction(engine, &small_tx, "Small transaction below profit thresholds").await?;

    // Scenario 4: Non-DEX transaction (should not trigger MEV strategies)
    let non_dex_tx = create_test_transaction(
        "0x4444444444444444444444444444444444444444",
        "1000000000000000000", // 1 ETH
        TargetType::Unknown,
        "transfer",
        "0xa9059cbb",
    );

    info!("=== Scenario 4: Non-DEX Transaction ===");
    evaluate_and_process_transaction(engine, &non_dex_tx, "Non-DEX transaction").await?;

    // Scenario 5: Batch evaluation
    info!("=== Scenario 5: Batch Transaction Evaluation ===");
    let batch_transactions = vec![
        large_uniswap_tx.clone(),
        high_slippage_tx.clone(),
        small_tx.clone(),
    ];

    let batch_start = Instant::now();
    let mut total_opportunities = 0;

    for (i, tx) in batch_transactions.iter().enumerate() {
        let opportunities = engine.evaluate_transaction(tx).await?;
        total_opportunities += opportunities.len();
        info!(
            batch_index = i,
            tx_hash = %tx.transaction.hash,
            opportunities_found = opportunities.len(),
            "Batch evaluation result"
        );
    }

    let batch_duration = batch_start.elapsed();
    info!(
        total_transactions = batch_transactions.len(),
        total_opportunities = total_opportunities,
        batch_duration_ms = batch_duration.as_millis(),
        avg_evaluation_time_ms = batch_duration.as_millis() / batch_transactions.len() as u128,
        "Batch evaluation completed"
    );

    Ok(())
}

async fn evaluate_and_process_transaction(
    engine: &StrategyEngine,
    tx: &ParsedTransaction,
    description: &str,
) -> Result<()> {
    info!(
        tx_hash = %tx.transaction.hash,
        description = description,
        value_eth = tx.transaction.value.parse::<u128>().unwrap_or(0) as f64 / 1e18,
        target_type = ?tx.target_type,
        "Evaluating transaction"
    );

    let start_time = Instant::now();
    let opportunities = engine.evaluate_transaction(tx).await?;
    let evaluation_time = start_time.elapsed();

    info!(
        opportunities_found = opportunities.len(),
        evaluation_time_ms = evaluation_time.as_millis(),
        "Transaction evaluation completed"
    );

    // Process each opportunity
    for opportunity in &opportunities {
        info!(
            opportunity_id = %opportunity.id,
            strategy = %opportunity.strategy_name,
            estimated_profit_eth = opportunity.estimated_profit_wei as f64 / 1e18,
            confidence = opportunity.confidence_score,
            priority = opportunity.priority,
            "Opportunity details"
        );

        // Create bundle plan
        match engine.create_bundle_plan(&opportunity.id).await? {
            Some(bundle_plan) => {
                info!(
                    bundle_id = %bundle_plan.id,
                    transaction_count = bundle_plan.transactions.len(),
                    estimated_gas = bundle_plan.estimated_gas_total,
                    risk_score = bundle_plan.risk_score,
                    "Bundle plan created"
                );

                // Display bundle details
                for (i, tx) in bundle_plan.transactions.iter().enumerate() {
                    info!(
                        bundle_id = %bundle_plan.id,
                        tx_index = i,
                        tx_type = ?tx.transaction_type,
                        to = %tx.to,
                        value_eth = tx.value as f64 / 1e18,
                        gas_limit = tx.gas_limit,
                        description = %tx.description,
                        "Bundle transaction"
                    );
                }
            }
            None => {
                warn!(
                    opportunity_id = %opportunity.id,
                    "Failed to create bundle plan for opportunity"
                );
            }
        }
    }

    if opportunities.is_empty() {
        info!("No MEV opportunities found for this transaction");
    }

    println!(); // Add spacing between scenarios
    Ok(())
}

fn create_test_transaction(
    hash: &str,
    value_wei: &str,
    target_type: TargetType,
    function_name: &str,
    function_signature: &str,
) -> ParsedTransaction {
    ParsedTransaction {
        transaction: Transaction {
            hash: hash.to_string(),
            from: "0xfrom1234567890123456789012345678901234567890".to_string(),
            to: Some("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D".to_string()), // Uniswap V2 Router
            value: value_wei.to_string(),
            gas_price: "50000000000".to_string(), // 50 gwei
            gas_limit: "200000".to_string(),
            nonce: 1,
            input: format!("{}000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000000000000000", function_signature),
            timestamp: chrono::Utc::now(),
        },
        decoded_input: Some(DecodedInput {
            function_name: function_name.to_string(),
            function_signature: function_signature.to_string(),
            parameters: vec![
                DecodedParameter {
                    name: "amountOutMin".to_string(),
                    param_type: "uint256".to_string(),
                    value: "0".to_string(), // Poor slippage protection
                },
                DecodedParameter {
                    name: "path".to_string(),
                    param_type: "address[]".to_string(),
                    value: "[\"0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2\", \"0xA0b86a33E6417c0c6b8c0c6b8c0c6b8c0c6b8c0c\"]".to_string(),
                },
                DecodedParameter {
                    name: "to".to_string(),
                    param_type: "address".to_string(),
                    value: "0xfrom1234567890123456789012345678901234567890".to_string(),
                },
                DecodedParameter {
                    name: "deadline".to_string(),
                    param_type: "uint256".to_string(),
                    value: (chrono::Utc::now().timestamp() + 300).to_string(), // 5 minutes from now
                },
            ],
        }),
        target_type,
        processing_time_ms: 1,
    }
}

async fn display_final_stats(engine: &StrategyEngine) -> Result<()> {
    info!("=== Final Statistics ===");

    // Engine statistics
    let engine_stats = engine.get_engine_stats().await;
    info!(
        transactions_evaluated = engine_stats.transactions_evaluated,
        opportunities_found = engine_stats.opportunities_found,
        bundles_created = engine_stats.bundles_created,
        avg_evaluation_time_ms = format!("{:.2}", engine_stats.average_evaluation_time_ms),
        "Strategy Engine Statistics"
    );

    // Individual strategy statistics
    let strategy_names = vec!["backrun", "sandwich", "arbitrage"];
    for strategy_name in strategy_names {
        if let Some(stats) = engine.get_strategy_stats(strategy_name).await {
            info!(
                strategy = strategy_name,
                opportunities_detected = stats.opportunities_detected,
                bundles_created = stats.bundles_created,
                success_rate = format!("{:.2}%", stats.success_rate * 100.0),
                avg_confidence = format!("{:.2}", stats.average_confidence_score),
                "Strategy Statistics"
            );
        }
    }

    // Active opportunities
    let active_opportunities = engine.get_active_opportunities().await;
    info!(
        active_opportunities = active_opportunities.len(),
        "Active opportunities in cache"
    );

    for opportunity in active_opportunities.iter().take(5) { // Show first 5
        info!(
            opportunity_id = %opportunity.id,
            strategy = %opportunity.strategy_name,
            profit_eth = opportunity.estimated_profit_wei as f64 / 1e18,
            confidence = opportunity.confidence_score,
            expires_in_seconds = opportunity.expiry_timestamp.saturating_sub(chrono::Utc::now().timestamp() as u64),
            "Active opportunity"
        );
    }

    Ok(())
}

/// Configuration demonstration
async fn demonstrate_yaml_config() -> Result<()> {
    info!("=== YAML Configuration Demonstration ===");

    // Create a sample configuration
    let sample_config = StrategyConfigFile::default();
    
    // Export to YAML
    let yaml_content = serde_yaml::to_string(&sample_config)?;
    info!("Sample YAML configuration:");
    println!("{}", yaml_content);

    // Show how to modify configuration
    let mut config_manager = ConfigManager::new("demo_config.yml".to_string());
    
    // Enable sandwich strategy
    config_manager.set_strategy_enabled("sandwich", true)?;
    info!("Enabled sandwich strategy");

    // Update backrun strategy parameters
    if let Some(mut backrun_config) = config_manager.get_strategy_config("backrun").cloned() {
        backrun_config.min_profit_wei = 1_000_000_000_000_000; // 0.001 ETH
        backrun_config.max_gas_price_gwei = 200;
        config_manager.update_strategy_config(backrun_config)?;
        info!("Updated backrun strategy configuration");
    }

    // Show enabled strategies
    let enabled_strategies = config_manager.get_enabled_strategies();
    info!("Enabled strategies: {:?}", enabled_strategies.iter().map(|s| &s.name).collect::<Vec<_>>());

    Ok(())
}