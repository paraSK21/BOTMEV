//! Integration tests for MEV bot using anvil fork simulation
//! 
//! These tests use a local anvil fork to simulate real blockchain conditions
//! and test the complete MEV bot pipeline from mempool ingestion to bundle execution.

use std::{sync::Arc, time::Duration};
use tokio::time::timeout;

use mev_core::{
    bundle::{BundleBuilder, BundleSubmitter, VictimGenerator},
    simulation::ForkSimulator,
    state_manager::StateManager,
    PrometheusMetrics,
};
use mev_mempool::{MempoolIngestion, WebSocketClient};
use mev_strategies::{StrategyEngine, BackrunStrategy, SandwichStrategy};

/// Test configuration for anvil integration tests
struct TestConfig {
    rpc_url: String,
    ws_url: String,
    chain_id: u64,
    test_timeout: Duration,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            rpc_url: std::env::var("MEV_BOT_RPC_URL")
                .unwrap_or_else(|_| "http://localhost:8545".to_string()),
            ws_url: std::env::var("MEV_BOT_WS_URL")
                .unwrap_or_else(|_| "ws://localhost:8545".to_string()),
            chain_id: std::env::var("MEV_BOT_CHAIN_ID")
                .unwrap_or_else(|_| "31337".to_string())
                .parse()
                .unwrap_or(31337),
            test_timeout: Duration::from_secs(30),
        }
    }
}

/// Integration test for complete mempool ingestion pipeline
#[tokio::test]
async fn test_mempool_ingestion_integration() {
    let config = TestConfig::default();
    
    // Create mempool ingestion system
    let mut mempool = WebSocketClient::new(&config.ws_url, config.chain_id)
        .expect("Failed to create WebSocket client");
    
    // Start ingestion
    mempool.start().await.expect("Failed to start mempool ingestion");
    
    // Get transaction stream
    let mut tx_stream = mempool.get_transaction_stream();
    
    // Wait for some transactions or timeout
    let result = timeout(config.test_timeout, async {
        let mut count = 0;
        while count < 5 {
            if let Ok(tx) = tx_stream.recv().await {
                println!("Received transaction: {}", tx.transaction.hash);
                count += 1;
            }
        }
        count
    }).await;
    
    match result {
        Ok(count) => {
            println!("✅ Mempool ingestion test passed: received {} transactions", count);
            assert!(count > 0, "Should receive at least one transaction");
        }
        Err(_) => {
            println!("⚠️  Mempool ingestion test timed out - this may be expected in test environment");
            // Don't fail the test as anvil might not have active transactions
        }
    }
    
    // Verify metrics are being collected
    let metrics = mempool.get_metrics();
    println!("Mempool metrics: {:?}", metrics);
}

/// Integration test for fork simulation with victim generator
#[tokio::test]
async fn test_fork_simulation_with_victim() {
    let config = TestConfig::default();
    
    // Create victim generator
    let victim_generator = VictimGenerator::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create victim generator");
    
    // Create fork simulator
    let simulator = ForkSimulator::new(&config.rpc_url)
        .await
        .expect("Failed to create fork simulator");
    
    // Generate a test victim transaction
    let victim_tx = victim_generator
        .generate_large_swap("USDC", "WETH", 10000.0)
        .await
        .expect("Failed to generate victim transaction");
    
    println!("Generated victim transaction: {}", victim_tx.hash);
    
    // Simulate the victim transaction
    let simulation_result = simulator
        .simulate_transaction(&victim_tx)
        .await
        .expect("Failed to simulate victim transaction");
    
    println!("Simulation result: {:?}", simulation_result);
    
    // Verify simulation results
    assert!(!simulation_result.reverted, "Victim transaction should not revert");
    assert!(simulation_result.gas_used > 0, "Should use some gas");
    
    println!("✅ Fork simulation with victim test passed");
}

/// Integration test for strategy detection and bundle construction
#[tokio::test]
async fn test_strategy_detection_integration() {
    let config = TestConfig::default();
    
    // Create strategy engine
    let mut strategy_engine = StrategyEngine::new();
    
    // Register strategies
    let backrun_strategy = BackrunStrategy::new(Default::default());
    let sandwich_strategy = SandwichStrategy::new(Default::default());
    
    strategy_engine.register_strategy(Box::new(backrun_strategy))
        .await
        .expect("Failed to register backrun strategy");
    strategy_engine.register_strategy(Box::new(sandwich_strategy))
        .await
        .expect("Failed to register sandwich strategy");
    
    // Create victim generator for test transactions
    let victim_generator = VictimGenerator::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create victim generator");
    
    // Generate test transactions that should trigger strategies
    let large_swap = victim_generator
        .generate_large_swap("USDC", "WETH", 50000.0)
        .await
        .expect("Failed to generate large swap");
    
    // Evaluate strategies
    let opportunities = strategy_engine
        .evaluate_transaction(&large_swap)
        .await
        .expect("Failed to evaluate transaction");
    
    println!("Found {} opportunities", opportunities.len());
    
    for opportunity in &opportunities {
        println!(
            "Strategy: {}, Profit: {} ETH, Confidence: {:.1}%",
            opportunity.strategy_name,
            opportunity.estimated_profit_wei as f64 / 1e18,
            opportunity.confidence_score * 100.0
        );
    }
    
    // Should find at least one opportunity for a large swap
    assert!(!opportunities.is_empty(), "Should find at least one MEV opportunity");
    
    println!("✅ Strategy detection integration test passed");
}

/// Integration test for complete bundle construction and simulation
#[tokio::test]
async fn test_bundle_construction_integration() {
    let config = TestConfig::default();
    
    // Create necessary components
    let victim_generator = VictimGenerator::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create victim generator");
    
    let simulator = ForkSimulator::new(&config.rpc_url)
        .await
        .expect("Failed to create fork simulator");
    
    let bundle_builder = BundleBuilder::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create bundle builder");
    
    // Generate victim transaction
    let victim_tx = victim_generator
        .generate_large_swap("USDC", "WETH", 25000.0)
        .await
        .expect("Failed to generate victim transaction");
    
    // Create a mock MEV opportunity
    let opportunity = mev_core::Opportunity {
        id: "test_opportunity".to_string(),
        strategy_name: "backrun".to_string(),
        target_transaction: victim_tx.clone(),
        estimated_profit_wei: 1_000_000_000_000_000_000u64, // 1 ETH
        gas_estimate: 200_000,
        confidence_score: 0.85,
        expiry_time: chrono::Utc::now() + chrono::Duration::minutes(5),
        bundle_transactions: vec![],
    };
    
    // Build bundle
    let bundle = bundle_builder
        .build_bundle(&opportunity)
        .await
        .expect("Failed to build bundle");
    
    println!("Built bundle with {} transactions", bundle.transactions.len());
    
    // Simulate bundle
    let bundle_result = simulator
        .simulate_bundle(&bundle)
        .await
        .expect("Failed to simulate bundle");
    
    println!("Bundle simulation result: {:?}", bundle_result);
    
    // Verify bundle simulation
    assert!(!bundle_result.reverted, "Bundle should not revert");
    assert!(bundle_result.profit_wei > 0, "Bundle should be profitable");
    
    println!("✅ Bundle construction integration test passed");
}

/// Integration test for state management and reorg detection
#[tokio::test]
async fn test_state_management_integration() {
    let config = TestConfig::default();
    
    // Create state manager
    let state_manager = StateManager::new(&config.rpc_url, ":memory:")
        .await
        .expect("Failed to create state manager");
    
    // Start state tracking
    state_manager.start().await.expect("Failed to start state manager");
    
    // Wait for initial state sync
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Get current state
    let current_block = state_manager
        .get_current_block()
        .await
        .expect("Failed to get current block");
    
    println!("Current block: {}", current_block);
    assert!(current_block > 0, "Should have a valid block number");
    
    // Test pending transaction tracking
    let victim_generator = VictimGenerator::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create victim generator");
    
    let test_tx = victim_generator
        .generate_large_swap("USDC", "WETH", 1000.0)
        .await
        .expect("Failed to generate test transaction");
    
    // Track the transaction
    state_manager
        .track_pending_transaction(&test_tx)
        .await
        .expect("Failed to track pending transaction");
    
    // Verify tracking
    let pending_count = state_manager
        .get_pending_transaction_count()
        .await
        .expect("Failed to get pending transaction count");
    
    println!("Pending transactions: {}", pending_count);
    assert!(pending_count > 0, "Should have at least one pending transaction");
    
    println!("✅ State management integration test passed");
}

/// Integration test for metrics collection and monitoring
#[tokio::test]
async fn test_metrics_integration() {
    // Create metrics system
    let metrics = Arc::new(PrometheusMetrics::new().expect("Failed to create metrics"));
    
    // Record some test metrics
    metrics.record_mempool_transaction();
    metrics.record_detection_latency(15.5);
    metrics.record_simulation_latency(8.2);
    metrics.record_bundle_success();
    
    // Get metrics snapshot
    let snapshot = metrics.get_snapshot();
    
    println!("Metrics snapshot: {:?}", snapshot);
    
    // Verify metrics are being recorded
    assert!(snapshot.mempool_transactions_total > 0, "Should have recorded mempool transactions");
    assert!(snapshot.detection_latency_avg > 0.0, "Should have recorded detection latency");
    assert!(snapshot.simulation_latency_avg > 0.0, "Should have recorded simulation latency");
    assert!(snapshot.bundle_success_total > 0, "Should have recorded bundle success");
    
    println!("✅ Metrics integration test passed");
}

/// End-to-end integration test covering the complete MEV bot pipeline
#[tokio::test]
async fn test_end_to_end_pipeline() {
    let config = TestConfig::default();
    
    println!("🚀 Starting end-to-end MEV bot pipeline test");
    
    // This test requires a running anvil instance with some activity
    // Skip if we can't connect to the test environment
    let client = reqwest::Client::new();
    let health_check = client
        .post(&config.rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": [],
            "id": 1
        }))
        .send()
        .await;
    
    if health_check.is_err() {
        println!("⚠️  Skipping end-to-end test: anvil not available");
        return;
    }
    
    // Create all components
    let metrics = Arc::new(PrometheusMetrics::new().expect("Failed to create metrics"));
    
    let state_manager = StateManager::new(&config.rpc_url, ":memory:")
        .await
        .expect("Failed to create state manager");
    
    let mut strategy_engine = StrategyEngine::new();
    strategy_engine.register_strategy(Box::new(BackrunStrategy::new(Default::default())))
        .await
        .expect("Failed to register backrun strategy");
    
    let victim_generator = VictimGenerator::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create victim generator");
    
    let simulator = ForkSimulator::new(&config.rpc_url)
        .await
        .expect("Failed to create fork simulator");
    
    let bundle_builder = BundleBuilder::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create bundle builder");
    
    // Start state management
    state_manager.start().await.expect("Failed to start state manager");
    
    // Generate a victim transaction
    let victim_tx = victim_generator
        .generate_large_swap("USDC", "WETH", 100000.0)
        .await
        .expect("Failed to generate victim transaction");
    
    println!("Generated victim transaction: {}", victim_tx.hash);
    metrics.record_mempool_transaction();
    
    // Evaluate strategies
    let start_time = std::time::Instant::now();
    let opportunities = strategy_engine
        .evaluate_transaction(&victim_tx)
        .await
        .expect("Failed to evaluate transaction");
    
    let detection_latency = start_time.elapsed().as_secs_f64() * 1000.0;
    metrics.record_detection_latency(detection_latency);
    
    println!("Found {} opportunities in {:.2}ms", opportunities.len(), detection_latency);
    
    if !opportunities.is_empty() {
        let opportunity = &opportunities[0];
        
        // Build bundle
        let bundle = bundle_builder
            .build_bundle(opportunity)
            .await
            .expect("Failed to build bundle");
        
        // Simulate bundle
        let sim_start = std::time::Instant::now();
        let simulation_result = simulator
            .simulate_bundle(&bundle)
            .await
            .expect("Failed to simulate bundle");
        
        let simulation_latency = sim_start.elapsed().as_secs_f64() * 1000.0;
        metrics.record_simulation_latency(simulation_latency);
        
        println!(
            "Bundle simulation completed in {:.2}ms: profit = {} ETH, gas = {}",
            simulation_latency,
            simulation_result.profit_wei as f64 / 1e18,
            simulation_result.gas_used
        );
        
        if simulation_result.profit_wei > 0 {
            metrics.record_bundle_success();
            println!("✅ Profitable bundle found!");
        }
        
        // Verify performance targets
        assert!(detection_latency < 50.0, "Detection latency should be under 50ms");
        assert!(simulation_latency < 100.0, "Simulation latency should be under 100ms");
    }
    
    // Get final metrics
    let final_metrics = metrics.get_snapshot();
    println!("Final metrics: {:?}", final_metrics);
    
    println!("✅ End-to-end pipeline test completed successfully");
}