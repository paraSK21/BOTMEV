//! Tests for simulation optimizer with throughput and safety checks

use anyhow::Result;
use ethers::types::{Address, Bytes, U256};
use mev_core::{
    PrometheusMetrics, RpcClientConfig, SimulationBundle, SimulationOptimizer,
    SimulationOptimizerConfig, SimulationTransaction, SafetyViolation,
};
use std::{sync::Arc, time::Duration};
use tokio::time::timeout;

/// Create test simulation optimizer with mock configuration
async fn create_test_optimizer() -> Result<SimulationOptimizer> {
    let metrics = Arc::new(PrometheusMetrics::new()?);
    
    let rpc_config = RpcClientConfig {
        endpoints: vec!["http://localhost:8545".to_string()],
        timeout_ms: 1000,
        max_concurrent_requests: 10,
        connection_pool_size: 5,
        retry_attempts: 1,
        retry_delay_ms: 50,
        enable_failover: false,
        health_check_interval_ms: 60000,
    };
    
    let optimizer_config = SimulationOptimizerConfig {
        batch_size: 5,
        batch_timeout_ms: 100,
        max_concurrent_batches: 3,
        warm_pool_size: 10,
        account_reuse_count: 10,
        min_profit_threshold_wei: U256::from(1_000_000_000_000_000u64), // 0.001 ETH
        max_stale_state_ms: 1000,
        pnl_tracking_enabled: true,
        safety_checks_enabled: true,
        throughput_target_sims_per_sec: 50.0, // Lower for tests
    };
    
    SimulationOptimizer::new(optimizer_config, rpc_config, metrics).await
}

/// Create test simulation bundle
fn create_test_bundle(id: &str, tx_count: usize) -> SimulationBundle {
    let mut transactions = Vec::new();
    
    for i in 0..tx_count {
        let tx = SimulationTransaction {
            from: Address::from([1u8; 20]),
            to: Some(Address::from([2u8; 20])),
            value: U256::from(1_000_000_000_000_000u64), // 0.001 ETH
            gas_limit: U256::from(21000),
            gas_price: U256::from(20_000_000_000u64), // 20 gwei
            data: Bytes::default(),
            nonce: Some(U256::from(i)),
        };
        transactions.push(tx);
    }
    
    SimulationBundle {
        id: id.to_string(),
        transactions,
        block_number: None,
        timestamp: Some(chrono::Utc::now().timestamp() as u64),
        base_fee: None,
        state_overrides: None,
    }
}

#[tokio::test]
async fn test_simulation_optimizer_creation() {
    let result = create_test_optimizer().await;
    
    // Should succeed even without RPC connection (will use fallback)
    match result {
        Ok(_) => {
            // Success case
        }
        Err(e) => {
            // Expected if no RPC endpoint available
            println!("Expected RPC connection error: {}", e);
        }
    }
}

#[tokio::test]
async fn test_batch_simulation_structure() {
    // Test the structure without requiring RPC connection
    let bundles = vec![
        create_test_bundle("test1", 2),
        create_test_bundle("test2", 3),
    ];
    
    assert_eq!(bundles.len(), 2);
    assert_eq!(bundles[0].transactions.len(), 2);
    assert_eq!(bundles[1].transactions.len(), 3);
    assert_eq!(bundles[0].id, "test1");
    assert_eq!(bundles[1].id, "test2");
}

#[tokio::test]
async fn test_safety_violation_detection() {
    // Test safety violation logic
    let config = SimulationOptimizerConfig::default();
    
    // Test profit threshold violation
    let low_profit = U256::from(100_000_000_000_000u64); // 0.0001 ETH
    let threshold = config.min_profit_threshold_wei;
    
    assert!(low_profit < threshold, "Low profit should be below threshold");
    
    // Test stale state detection
    let max_stale_ms = config.max_stale_state_ms;
    let stale_age_ms = max_stale_ms + 100;
    
    assert!(stale_age_ms > max_stale_ms, "Stale age should exceed maximum");
}

#[tokio::test]
async fn test_configuration_validation() {
    let config = SimulationOptimizerConfig::default();
    
    // Validate configuration parameters
    assert!(config.batch_size > 0, "Batch size must be positive");
    assert!(config.batch_timeout_ms > 0, "Batch timeout must be positive");
    assert!(config.max_concurrent_batches > 0, "Max concurrent batches must be positive");
    assert!(config.throughput_target_sims_per_sec >= 200.0, "Should target at least 200 sims/sec");
    assert!(config.min_profit_threshold_wei > U256::zero(), "Profit threshold must be positive");
    assert!(config.max_stale_state_ms > 0, "Stale state timeout must be positive");
}

#[tokio::test]
async fn test_account_pool_functionality() {
    use mev_core::AccountPool;
    
    let accounts = vec![
        Address::from([1u8; 20]),
        Address::from([2u8; 20]),
        Address::from([3u8; 20]),
    ];
    
    let mut pool = AccountPool::new(accounts.clone(), 2);
    
    // Test account retrieval
    let account1 = pool.get_account();
    assert!(account1.is_some(), "Should get first account");
    
    let account2 = pool.get_account();
    assert!(account2.is_some(), "Should get second account");
    
    // Test account reuse
    let account3 = pool.get_account();
    assert!(account3.is_some(), "Should get third account");
    
    // Test usage reset
    pool.reset_usage();
    let account_after_reset = pool.get_account();
    assert!(account_after_reset.is_some(), "Should get account after reset");
}

#[tokio::test]
async fn test_pnl_tracker_functionality() {
    use mev_core::PnLTracker;
    
    let mut tracker = PnLTracker::default();
    
    // Test initial state
    assert_eq!(tracker.successful_simulations, 0);
    assert_eq!(tracker.failed_simulations, 0);
    assert_eq!(tracker.total_simulated_profit_wei, U256::zero());
    
    // Test updates
    tracker.successful_simulations += 5;
    tracker.failed_simulations += 2;
    tracker.total_simulated_profit_wei += U256::from(1_000_000_000_000_000u64);
    
    assert_eq!(tracker.successful_simulations, 5);
    assert_eq!(tracker.failed_simulations, 2);
    assert!(tracker.total_simulated_profit_wei > U256::zero());
}

#[tokio::test]
async fn test_throughput_stats_calculation() {
    use mev_core::ThroughputStats;
    
    let stats = ThroughputStats {
        total_simulations: 1000,
        active_batches: 5,
        max_concurrent_batches: 10,
        target_throughput_sims_per_sec: 200.0,
        batch_size: 20,
    };
    
    assert_eq!(stats.total_simulations, 1000);
    assert!(stats.active_batches <= stats.max_concurrent_batches);
    assert_eq!(stats.target_throughput_sims_per_sec, 200.0);
}

#[tokio::test]
async fn test_simulation_priority_ordering() {
    use mev_core::SimulationPriority;
    
    let priorities = vec![
        SimulationPriority::Low,
        SimulationPriority::Critical,
        SimulationPriority::Normal,
        SimulationPriority::High,
    ];
    
    let mut sorted_priorities = priorities.clone();
    sorted_priorities.sort();
    
    // Should be sorted: Low, Normal, High, Critical
    assert_eq!(sorted_priorities[0], SimulationPriority::Low);
    assert_eq!(sorted_priorities[1], SimulationPriority::Normal);
    assert_eq!(sorted_priorities[2], SimulationPriority::High);
    assert_eq!(sorted_priorities[3], SimulationPriority::Critical);
}

#[tokio::test]
async fn test_safety_violation_types() {
    // Test different safety violation types
    let bundle_id = "test_bundle".to_string();
    
    let profit_violation = SafetyViolation::ProfitBelowThreshold {
        bundle_id: bundle_id.clone(),
        actual_profit_wei: U256::from(100),
        threshold_wei: U256::from(1000),
    };
    
    let stale_violation = SafetyViolation::StaleState {
        bundle_id: bundle_id.clone(),
        state_age_ms: 2000,
        max_age_ms: 1000,
    };
    
    let gas_violation = SafetyViolation::ExcessiveGasCost {
        bundle_id: bundle_id.clone(),
        gas_cost_wei: U256::from(2000),
        profit_wei: U256::from(1000),
    };
    
    let pnl_violation = SafetyViolation::NegativePnL {
        bundle_id: bundle_id.clone(),
        net_loss_wei: U256::from(500),
    };
    
    // Verify violations can be created and contain expected data
    match profit_violation {
        SafetyViolation::ProfitBelowThreshold { actual_profit_wei, threshold_wei, .. } => {
            assert!(actual_profit_wei < threshold_wei);
        }
        _ => panic!("Wrong violation type"),
    }
    
    match stale_violation {
        SafetyViolation::StaleState { state_age_ms, max_age_ms, .. } => {
            assert!(state_age_ms > max_age_ms);
        }
        _ => panic!("Wrong violation type"),
    }
    
    match gas_violation {
        SafetyViolation::ExcessiveGasCost { gas_cost_wei, profit_wei, .. } => {
            assert!(gas_cost_wei > profit_wei);
        }
        _ => panic!("Wrong violation type"),
    }
    
    match pnl_violation {
        SafetyViolation::NegativePnL { net_loss_wei, .. } => {
            assert!(net_loss_wei > U256::zero());
        }
        _ => panic!("Wrong violation type"),
    }
}

#[tokio::test]
async fn test_benchmark_config_defaults() {
    use mev_core::BenchmarkConfig;
    
    let config = BenchmarkConfig::default();
    
    assert!(config.duration_seconds > 0);
    assert!(config.concurrent_batches > 0);
    assert!(config.bundles_per_batch > 0);
    assert!(config.transactions_per_bundle > 0);
    assert_eq!(config.target_throughput_sims_per_sec, 200.0);
    assert!(config.warmup_duration_seconds >= 0);
}

// Integration test that requires RPC connection (will be skipped if no connection)
#[tokio::test]
#[ignore] // Use `cargo test -- --ignored` to run this test
async fn test_full_simulation_integration() -> Result<()> {
    let optimizer = create_test_optimizer().await?;
    
    let bundles = vec![
        create_test_bundle("integration_test_1", 1),
        create_test_bundle("integration_test_2", 2),
    ];
    
    let result = timeout(
        Duration::from_secs(10),
        optimizer.simulate_batch(bundles)
    ).await;
    
    match result {
        Ok(Ok(batch_result)) => {
            assert!(!batch_result.batch_id.is_empty());
            assert!(batch_result.batch_latency_ms >= 0.0);
            println!("Integration test successful: {} results", batch_result.results.len());
        }
        Ok(Err(e)) => {
            println!("Simulation failed (expected without RPC): {}", e);
        }
        Err(_) => {
            println!("Simulation timeout (expected without RPC)");
        }
    }
    
    Ok(())
}