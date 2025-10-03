//! Demonstration of optimized simulation throughput with safety checks

use anyhow::Result;
use ethers::types::{Address, Bytes, U256};
use mev_core::{
    BenchmarkRunner, PrometheusMetrics, RpcClientConfig, SimulationBenchmark, SimulationBundle,
    SimulationOptimizer, SimulationOptimizerConfig, SimulationTransaction, BenchmarkConfig,
};
use std::{sync::Arc, time::Duration};
use tokio::time::timeout;
use tracing::{info, Level};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("Starting simulation throughput demonstration");

    // Initialize metrics
    let metrics = Arc::new(PrometheusMetrics::new()?);

    // Configure RPC client for local testing
    let rpc_config = RpcClientConfig {
        endpoints: vec!["http://localhost:8545".to_string()],
        timeout_ms: 1000,
        max_concurrent_requests: 100,
        connection_pool_size: 20,
        retry_attempts: 2,
        retry_delay_ms: 50,
        enable_failover: false,
        health_check_interval_ms: 30000,
    };

    // Configure simulation optimizer for high throughput
    let optimizer_config = SimulationOptimizerConfig {
        batch_size: 20,
        batch_timeout_ms: 25,
        max_concurrent_batches: 25,
        warm_pool_size: 100,
        account_reuse_count: 200,
        min_profit_threshold_wei: U256::from(500_000_000_000_000u64), // 0.0005 ETH
        max_stale_state_ms: 500,
        pnl_tracking_enabled: true,
        safety_checks_enabled: true,
        throughput_target_sims_per_sec: 250.0, // Aim higher than 200
    };

    // Create simulation optimizer
    let optimizer = match SimulationOptimizer::new(
        optimizer_config.clone(),
        rpc_config.clone(),
        metrics.clone(),
    ).await {
        Ok(optimizer) => {
            info!("✓ Simulation optimizer created successfully");
            optimizer
        }
        Err(e) => {
            info!("⚠ Could not connect to RPC endpoint: {}", e);
            info!("Running in demo mode with mock simulations");
            return run_demo_mode().await;
        }
    };

    // Demonstrate basic batch simulation
    info!("=== Demonstrating Basic Batch Simulation ===");
    demonstrate_batch_simulation(&optimizer).await?;

    // Demonstrate safety checks
    info!("=== Demonstrating Safety Checks ===");
    demonstrate_safety_checks(&optimizer).await?;

    // Run throughput benchmark
    info!("=== Running Throughput Benchmark ===");
    run_throughput_benchmark(optimizer_config, rpc_config, metrics.clone()).await?;

    // Show PnL tracking
    info!("=== PnL Tracking Results ===");
    let pnl_stats = optimizer.get_pnl_stats().await;
    info!("Total Simulated Profit: {} wei", pnl_stats.total_simulated_profit_wei);
    info!("Total Gas Costs: {} wei", pnl_stats.total_gas_costs_wei);
    info!("Successful Simulations: {}", pnl_stats.successful_simulations);
    info!("Failed Simulations: {}", pnl_stats.failed_simulations);

    // Show throughput stats
    let throughput_stats = optimizer.get_throughput_stats().await;
    throughput_stats.print_summary();

    info!("Simulation throughput demonstration completed");
    Ok(())
}

/// Demonstrate batch simulation with multiple bundles
async fn demonstrate_batch_simulation(optimizer: &SimulationOptimizer) -> Result<()> {
    let bundles = create_test_bundles(5, 3);
    
    info!("Submitting batch of {} bundles for simulation", bundles.len());
    
    match timeout(Duration::from_secs(10), optimizer.simulate_batch(bundles)).await {
        Ok(Ok(result)) => {
            info!("✓ Batch simulation completed successfully");
            info!("  - Batch ID: {}", result.batch_id);
            info!("  - Results: {}", result.results.len());
            info!("  - Batch Latency: {:.2} ms", result.batch_latency_ms);
            info!("  - Throughput: {:.1} sims/sec", result.throughput_sims_per_sec);
            info!("  - Safety Violations: {}", result.safety_violations.len());
            
            for violation in &result.safety_violations {
                match violation {
                    mev_core::SafetyViolation::ProfitBelowThreshold { bundle_id, actual_profit_wei, threshold_wei } => {
                        info!("  ⚠ Profit below threshold for {}: {} < {} wei", 
                              bundle_id, actual_profit_wei, threshold_wei);
                    }
                    mev_core::SafetyViolation::StaleState { bundle_id, state_age_ms, max_age_ms } => {
                        info!("  ⚠ Stale state for {}: {} > {} ms", 
                              bundle_id, state_age_ms, max_age_ms);
                    }
                    mev_core::SafetyViolation::ExcessiveGasCost { bundle_id, gas_cost_wei, profit_wei } => {
                        info!("  ⚠ Excessive gas cost for {}: {} > {} wei", 
                              bundle_id, gas_cost_wei, profit_wei);
                    }
                    mev_core::SafetyViolation::NegativePnL { bundle_id, net_loss_wei } => {
                        info!("  ⚠ Negative PnL for {}: {} wei loss", 
                              bundle_id, net_loss_wei);
                    }
                }
            }
        }
        Ok(Err(e)) => {
            info!("✗ Batch simulation failed: {}", e);
        }
        Err(_) => {
            info!("✗ Batch simulation timeout");
        }
    }
    
    Ok(())
}

/// Demonstrate safety checks with problematic bundles
async fn demonstrate_safety_checks(optimizer: &SimulationOptimizer) -> Result<()> {
    // Create bundles that should trigger safety violations
    let mut problematic_bundles = Vec::new();
    
    // Bundle with very low profit
    let low_profit_bundle = SimulationBundle {
        id: "low_profit_test".to_string(),
        transactions: vec![SimulationTransaction {
            from: Address::from([1u8; 20]),
            to: Some(Address::from([2u8; 20])),
            value: U256::from(1000), // Very small value
            gas_limit: U256::from(21000),
            gas_price: U256::from(100_000_000_000u64), // High gas price
            data: Bytes::default(),
            nonce: Some(U256::from(1)),
        }],
        block_number: None,
        timestamp: Some(chrono::Utc::now().timestamp() as u64),
        base_fee: None,
        state_overrides: None,
    };
    problematic_bundles.push(low_profit_bundle);
    
    // Bundle with excessive gas costs
    let high_gas_bundle = SimulationBundle {
        id: "high_gas_test".to_string(),
        transactions: vec![SimulationTransaction {
            from: Address::from([3u8; 20]),
            to: Some(Address::from([4u8; 20])),
            value: U256::from(1_000_000_000_000_000u64), // 0.001 ETH
            gas_limit: U256::from(1_000_000), // Very high gas limit
            gas_price: U256::from(200_000_000_000u64), // Very high gas price
            data: Bytes::from(vec![0u8; 1000]), // Large data
            nonce: Some(U256::from(2)),
        }],
        block_number: None,
        timestamp: Some(chrono::Utc::now().timestamp() as u64),
        base_fee: None,
        state_overrides: None,
    };
    problematic_bundles.push(high_gas_bundle);
    
    info!("Testing safety checks with {} problematic bundles", problematic_bundles.len());
    
    match optimizer.simulate_batch(problematic_bundles).await {
        Ok(result) => {
            info!("✓ Safety check simulation completed");
            info!("  - Safety Violations Detected: {}", result.safety_violations.len());
            
            if result.safety_violations.is_empty() {
                info!("  ⚠ Expected safety violations but none were detected");
            } else {
                info!("  ✓ Safety checks working correctly");
            }
        }
        Err(e) => {
            info!("Safety check simulation failed: {}", e);
        }
    }
    
    Ok(())
}

/// Run comprehensive throughput benchmark
async fn run_throughput_benchmark(
    optimizer_config: SimulationOptimizerConfig,
    rpc_config: RpcClientConfig,
    metrics: Arc<PrometheusMetrics>,
) -> Result<()> {
    let benchmark_config = BenchmarkConfig {
        duration_seconds: 30, // Shorter for demo
        concurrent_batches: 15,
        bundles_per_batch: 8,
        transactions_per_bundle: 2,
        target_throughput_sims_per_sec: 200.0,
        warmup_duration_seconds: 5,
    };
    
    match SimulationBenchmark::new(
        benchmark_config,
        optimizer_config,
        rpc_config,
        metrics,
    ).await {
        Ok(benchmark) => {
            match benchmark.run_benchmark().await {
                Ok(results) => {
                    if results.throughput_achieved {
                        info!("🎉 Throughput target ACHIEVED!");
                    } else {
                        info!("📊 Throughput results recorded (target not met)");
                    }
                }
                Err(e) => {
                    info!("Benchmark execution failed: {}", e);
                }
            }
        }
        Err(e) => {
            info!("Could not create benchmark: {}", e);
        }
    }
    
    Ok(())
}

/// Run in demo mode when RPC is not available
async fn run_demo_mode() -> Result<()> {
    info!("=== Running in Demo Mode ===");
    info!("This demonstrates the simulation optimizer structure without live RPC");
    
    // Show configuration
    let config = SimulationOptimizerConfig::default();
    info!("Default Configuration:");
    info!("  - Batch Size: {}", config.batch_size);
    info!("  - Batch Timeout: {} ms", config.batch_timeout_ms);
    info!("  - Max Concurrent Batches: {}", config.max_concurrent_batches);
    info!("  - Throughput Target: {:.0} sims/sec", config.throughput_target_sims_per_sec);
    info!("  - Min Profit Threshold: {} wei", config.min_profit_threshold_wei);
    info!("  - Max Stale State: {} ms", config.max_stale_state_ms);
    
    // Show safety features
    info!("Safety Features:");
    info!("  ✓ Minimum profit threshold checking");
    info!("  ✓ Stale state detection");
    info!("  ✓ PnL tracking and accounting");
    info!("  ✓ Excessive gas cost detection");
    info!("  ✓ Account reuse optimization");
    
    // Show performance optimizations
    info!("Performance Optimizations:");
    info!("  ✓ Task batching for simulations");
    info!("  ✓ Warm HTTP client pools");
    info!("  ✓ Connection pooling and reuse");
    info!("  ✓ Concurrent batch processing");
    info!("  ✓ Priority-based queue management");
    
    info!("Demo mode completed - connect to RPC endpoint for full functionality");
    Ok(())
}

/// Create test simulation bundles
fn create_test_bundles(bundle_count: usize, tx_per_bundle: usize) -> Vec<SimulationBundle> {
    let mut bundles = Vec::new();
    
    for i in 0..bundle_count {
        let mut transactions = Vec::new();
        
        for j in 0..tx_per_bundle {
            let tx = SimulationTransaction {
                from: Address::from([(i + 1) as u8; 20]),
                to: Some(Address::from([(i + 2) as u8; 20])),
                value: U256::from(1_000_000_000_000_000u64), // 0.001 ETH
                gas_limit: U256::from(21000 + j * 1000), // Varying gas limits
                gas_price: U256::from(20_000_000_000u64 + j as u64 * 1_000_000_000), // Varying gas prices
                data: if j % 2 == 0 { 
                    Bytes::default() 
                } else { 
                    Bytes::from(vec![0x60, 0x60, 0x60, 0x40]) 
                },
                nonce: Some(U256::from(j)),
            };
            transactions.push(tx);
        }
        
        let bundle = SimulationBundle {
            id: format!("test_bundle_{}", i),
            transactions,
            block_number: None,
            timestamp: Some(chrono::Utc::now().timestamp() as u64),
            base_fee: None,
            state_overrides: None,
        };
        
        bundles.push(bundle);
    }
    
    bundles
}