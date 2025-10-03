//! Filter testing utility for MEV bot mempool filtering system

use anyhow::Result;
use chrono::Utc;
use mev_core::{DecodedInput, ParsedTransaction, TargetType, Transaction};
use mev_mempool::{FilterConfigManager, FilterEngine, FilterRuleTemplates};
use std::time::Instant;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("info,mev_mempool=debug")
        .with_target(false)
        .init();

    info!("Starting MEV Bot Filter Test");

    // Test 1: Basic filter functionality
    test_basic_filters().await?;

    // Test 2: YAML configuration loading
    test_yaml_config().await?;

    // Test 3: Performance test
    test_performance().await?;

    info!("All filter tests completed successfully");
    Ok(())
}

async fn test_basic_filters() -> Result<()> {
    info!("Testing basic filter functionality");

    let mut filter_engine = FilterEngine::new();
    let rules = FilterRuleTemplates::get_default_rules();
    filter_engine.load_rules(rules)?;

    // Create test transactions
    let high_value_tx = create_test_transaction(10.0, 50, TargetType::UniswapV2);
    let spam_tx = create_test_transaction(0.0, 3, TargetType::Unknown);
    let whale_tx = create_test_transaction(150.0, 100, TargetType::UniswapV3);

    // Test filtering
    let result1 = filter_engine.filter_transaction(high_value_tx)?;
    let result2 = filter_engine.filter_transaction(spam_tx)?;
    let result3 = filter_engine.filter_transaction(whale_tx)?;

    assert!(result1.is_some(), "High value transaction should be accepted");
    assert!(result2.is_none(), "Spam transaction should be rejected");
    assert!(result3.is_some(), "Whale transaction should be accepted");

    let stats = filter_engine.get_stats();
    info!("Basic filter test completed - processed: {}, accepted: {}, rejected: {}", 
          stats.total_processed, stats.accepted, stats.rejected);

    Ok(())
}

async fn test_yaml_config() -> Result<()> {
    info!("Testing YAML configuration loading");

    let config_path = "config/filter_rules.yml";
    if !std::path::Path::new(config_path).exists() {
        warn!("Filter rules config file not found: {}", config_path);
        return Ok(());
    }

    let mut config_manager = FilterConfigManager::new(config_path);
    let config = config_manager.load_config()?;
    let rules = config_manager.to_filter_rules(&config)?;

    let mut filter_engine = FilterEngine::new();
    filter_engine.load_rules(rules)?;

    info!("YAML configuration loaded successfully with {} rules", 
          filter_engine.get_active_rules().len());

    Ok(())
}

async fn test_performance() -> Result<()> {
    info!("Testing filter performance");

    let mut filter_engine = FilterEngine::new();
    let rules = FilterRuleTemplates::get_default_rules();
    filter_engine.load_rules(rules)?;

    let start_time = Instant::now();
    let test_count = 1000;

    for i in 0..test_count {
        let tx = create_test_transaction(
            if i % 2 == 0 { 2.0 } else { 0.1 },
            20,
            TargetType::UniswapV2,
        );
        let _ = filter_engine.filter_transaction(tx)?;
    }

    let duration = start_time.elapsed();
    let tps = test_count as f64 / duration.as_secs_f64();

    info!("Performance test completed - {} TPS, {:.3}ms avg filter time", 
          tps, duration.as_millis() as f64 / test_count as f64);

    Ok(())
}

fn create_test_transaction(value_eth: f64, gas_price_gwei: u64, target_type: TargetType) -> ParsedTransaction {
    let value_wei = (value_eth * 1e18) as u128;
    let gas_price_wei = gas_price_gwei * 1_000_000_000;

    ParsedTransaction {
        transaction: Transaction {
            hash: "0x123".to_string(),
            from: "0xfrom".to_string(),
            to: Some("0xto".to_string()),
            value: value_wei.to_string(),
            gas_price: gas_price_wei.to_string(),
            gas_limit: "21000".to_string(),
            nonce: 1,
            input: "0x38ed1739".to_string(),
            timestamp: Utc::now(),
        },
        decoded_input: Some(DecodedInput {
            function_name: "swapExactTokensForTokens".to_string(),
            function_signature: "0x38ed1739".to_string(),
            parameters: vec![],
        }),
        target_type,
        processing_time_ms: 1,
    }
}