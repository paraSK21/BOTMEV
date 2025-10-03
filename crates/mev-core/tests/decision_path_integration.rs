//! Integration tests for the ultra-low latency decision path

use mev_core::{
    DecisionPath, DecisionPathConfig, MockArbitrageStrategy, MockBackrunStrategy, MockStrategyEngine,
    PrometheusMetrics, ParsedTransaction, Transaction, TargetType,
};
use std::{sync::Arc, time::Duration};

/// Create a test transaction
fn create_test_transaction(hash_suffix: u32, target_type: TargetType, value_eth: u64) -> ParsedTransaction {
    ParsedTransaction {
        transaction: Transaction {
            hash: format!("0x{:064x}", hash_suffix),
            from: format!("0xuser{:x}", hash_suffix),
            to: Some(format!("0xcontract{:x}", hash_suffix)),
            value: format!("{}", value_eth * 1_000_000_000_000_000_000u64), // Convert to wei
            gas_price: "50000000000".to_string(), // 50 gwei
            gas_limit: "200000".to_string(),
            nonce: hash_suffix as u64,
            input: "0x38ed1739000000000000000000000000".to_string(),
            timestamp: chrono::Utc::now(),
        },
        decoded_input: None,
        target_type,
        processing_time_ms: 1,
    }
}

#[tokio::test]
async fn test_decision_path_latency_target() {
    // Create optimized configuration for low latency
    let config = DecisionPathConfig {
        enable_cpu_pinning: false, // Disable for test environment
        latency_targets: mev_core::decision_path::LatencyTargets {
            total_decision_loop_ms: 25,
            filter_stage_ms: 5,
            simulation_stage_ms: 15,
            bundle_stage_ms: 5,
            timeout_multiplier: 1.5,
        },
        thread_pools: mev_core::decision_path::ThreadPoolSizes {
            filter_workers: 1,
            simulation_workers: 2,
            bundle_workers: 1,
        },
        ..Default::default()
    };

    // Create strategy engine with mock strategies
    let mut strategy_engine = MockStrategyEngine::new();
    strategy_engine.register_strategy(Box::new(MockArbitrageStrategy::new())).await.unwrap();
    strategy_engine.register_strategy(Box::new(MockBackrunStrategy::new())).await.unwrap();
    
    // Create decision path
    let metrics = Arc::new(PrometheusMetrics::new().unwrap());
    let decision_path = Arc::new(DecisionPath::new(
        config,
        Arc::new(strategy_engine),
        metrics,
    ).unwrap());

    // Start the pipeline
    decision_path.start().await.unwrap();

    // Create test transactions
    let test_transactions = vec![
        create_test_transaction(1, TargetType::UniswapV2, 2), // Should trigger arbitrage
        create_test_transaction(2, TargetType::SushiSwap, 6), // Should trigger backrun
        create_test_transaction(3, TargetType::Curve, 1),     // Might trigger strategies
        create_test_transaction(4, TargetType::Unknown, 0),   // Should not trigger strategies
    ];

    let results_rx = decision_path.get_results();
    
    // Send test transactions
    for tx in &test_transactions {
        decision_path.process_transaction(tx.clone()).await.unwrap();
    }

    // Collect results with timeout
    let mut results = Vec::new();
    let mut collected = 0;
    let target_results = test_transactions.len();

    while collected < target_results {
        match results_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(result) => {
                results.push(result);
                collected += 1;
            }
            Err(_) => break, // Timeout or channel closed
        }
    }

    // Verify we got results
    assert!(!results.is_empty(), "Should have received at least one result");

    // Verify latency targets
    let mut within_target_count = 0;
    let mut total_latency = 0.0;
    let target_latency = 25.0; // 25ms target

    for result in &results {
        total_latency += result.processing_time_ms;
        if result.processing_time_ms <= target_latency {
            within_target_count += 1;
        }
        
        println!(
            "Transaction {}: {:.2}ms (target: {}ms) - {} opportunities",
            result.transaction_id,
            result.processing_time_ms,
            target_latency,
            result.opportunities.len()
        );
    }

    let average_latency = total_latency / results.len() as f64;
    let success_rate = within_target_count as f64 / results.len() as f64;

    println!("Average latency: {:.2}ms", average_latency);
    println!("Success rate: {:.1}% ({}/{} within target)", success_rate * 100.0, within_target_count, results.len());

    // Assert performance targets
    assert!(average_latency <= target_latency, 
        "Average latency {:.2}ms exceeds target {}ms", average_latency, target_latency);
    assert!(success_rate >= 0.8, 
        "Success rate {:.1}% is below 80%", success_rate * 100.0);

    // Verify we found some opportunities
    let total_opportunities: usize = results.iter().map(|r| r.opportunities.len()).sum();
    assert!(total_opportunities > 0, "Should have found at least one MEV opportunity");

    // Shutdown
    decision_path.shutdown().await;
    
    println!("✅ Decision path latency test passed!");
}

#[tokio::test]
async fn test_decision_path_throughput() {
    // Create configuration optimized for throughput
    let config = DecisionPathConfig {
        enable_cpu_pinning: false,
        thread_pools: mev_core::decision_path::ThreadPoolSizes {
            filter_workers: 2,
            simulation_workers: 4,
            bundle_workers: 2,
        },
        channel_buffer_sizes: mev_core::decision_path::ChannelBufferSizes {
            filter_input: 1000,
            simulation_input: 500,
            bundle_input: 200,
            output: 100,
        },
        ..Default::default()
    };

    let mut strategy_engine = MockStrategyEngine::new();
    strategy_engine.register_strategy(Box::new(MockArbitrageStrategy::new())).await.unwrap();
    
    let metrics = Arc::new(PrometheusMetrics::new().unwrap());
    let decision_path = Arc::new(DecisionPath::new(
        config,
        Arc::new(strategy_engine),
        metrics,
    ).unwrap());

    decision_path.start().await.unwrap();

    // Generate load test transactions
    let transaction_count = 50;
    let mut test_transactions = Vec::new();
    
    for i in 0..transaction_count {
        let target_type = match i % 3 {
            0 => TargetType::UniswapV2,
            1 => TargetType::SushiSwap,
            _ => TargetType::Curve,
        };
        test_transactions.push(create_test_transaction(i, target_type, 2));
    }

    let results_rx = decision_path.get_results();
    let start_time = std::time::Instant::now();

    // Send all transactions rapidly
    for tx in &test_transactions {
        decision_path.process_transaction(tx.clone()).await.unwrap();
    }
    
    let send_duration = start_time.elapsed();

    // Collect results
    let mut results = Vec::new();
    let mut collected = 0;

    while collected < transaction_count && collected < 100 { // Safety limit
        match results_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(result) => {
                results.push(result);
                collected += 1;
            }
            Err(_) => break,
        }
    }

    let total_duration = start_time.elapsed();
    let throughput = results.len() as f64 / total_duration.as_secs_f64();

    println!("Sent {} transactions in {:.2}ms", transaction_count, send_duration.as_secs_f64() * 1000.0);
    println!("Processed {} results in {:.2}s", results.len(), total_duration.as_secs_f64());
    println!("Throughput: {:.1} TPS", throughput);

    // Verify throughput target (should handle at least 10 TPS in test environment)
    assert!(throughput >= 10.0, "Throughput {:.1} TPS is below minimum 10 TPS", throughput);
    assert!(results.len() >= (transaction_count / 2) as usize, "Should process at least half the transactions");

    decision_path.shutdown().await;
    
    println!("✅ Decision path throughput test passed!");
}

#[tokio::test]
async fn test_decision_path_opportunity_detection() {
    let config = DecisionPathConfig::default();
    
    let mut strategy_engine = MockStrategyEngine::new();
    strategy_engine.register_strategy(Box::new(MockArbitrageStrategy::new())).await.unwrap();
    strategy_engine.register_strategy(Box::new(MockBackrunStrategy::new())).await.unwrap();
    
    let metrics = Arc::new(PrometheusMetrics::new().unwrap());
    let decision_path = Arc::new(DecisionPath::new(
        config,
        Arc::new(strategy_engine),
        metrics,
    ).unwrap());

    decision_path.start().await.unwrap();

    // Create transactions that should trigger opportunities
    let high_value_uniswap = create_test_transaction(1, TargetType::UniswapV2, 10); // High value for backrun
    let sushiswap_tx = create_test_transaction(2, TargetType::SushiSwap, 8);        // High value for backrun
    let small_tx = create_test_transaction(3, TargetType::Unknown, 0);              // Should not trigger

    let results_rx = decision_path.get_results();
    
    // Send transactions
    decision_path.process_transaction(high_value_uniswap).await.unwrap();
    decision_path.process_transaction(sushiswap_tx).await.unwrap();
    decision_path.process_transaction(small_tx).await.unwrap();

    // Collect results
    let mut results = Vec::new();
    let mut collected = 0;

    while collected < 3 {
        match results_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(result) => {
                results.push(result);
                collected += 1;
            }
            Err(_) => break,
        }
    }

    // Verify opportunity detection
    let mut arbitrage_found = false;
    let mut backrun_found = false;
    let mut total_opportunities = 0;

    for result in &results {
        total_opportunities += result.opportunities.len();
        
        for opportunity in &result.opportunities {
            match opportunity.strategy_name.as_str() {
                "mock_arbitrage" => arbitrage_found = true,
                "mock_backrun" => backrun_found = true,
                _ => {}
            }
            
            // Verify opportunity structure
            assert!(!opportunity.id.is_empty());
            assert!(opportunity.estimated_profit_wei > 0);
            assert!(opportunity.confidence_score > 0.0 && opportunity.confidence_score <= 1.0);
            
            println!(
                "Found {} opportunity: {} ETH profit (confidence: {:.1}%)",
                opportunity.strategy_name,
                opportunity.estimated_profit_wei as f64 / 1e18,
                opportunity.confidence_score * 100.0
            );
        }
    }

    // Should find at least some opportunities from high-value transactions
    assert!(total_opportunities > 0, "Should have found at least one opportunity");
    
    // At least one strategy should have triggered (due to randomness, not guaranteed both will)
    assert!(arbitrage_found || backrun_found, "At least one strategy should have found an opportunity");

    decision_path.shutdown().await;
    
    println!("✅ Decision path opportunity detection test passed!");
}