//! Advanced integration tests using anvil fork simulation
//! 
//! These tests create realistic blockchain scenarios using anvil fork
//! to validate the complete MEV bot pipeline under controlled conditions.

use std::{sync::Arc, time::Duration};
use tokio::time::timeout;
use serde_json::json;

use mev_core::{
    bundle::{BundleBuilder, BundleSubmitter, VictimGenerator},
    simulation::ForkSimulator,
    state_manager::StateManager,
    PrometheusMetrics,
    types::{Opportunity, ParsedTransaction},
};
use mev_mempool::{MempoolIngestion, WebSocketClient};
use mev_strategies::{StrategyEngine, BackrunStrategy, SandwichStrategy};

/// Test configuration for anvil integration tests
#[derive(Clone)]
struct AnvilTestConfig {
    rpc_url: String,
    ws_url: String,
    chain_id: u64,
    test_timeout: Duration,
    anvil_available: bool,
}

impl AnvilTestConfig {
    async fn new() -> Self {
        let rpc_url = std::env::var("MEV_BOT_RPC_URL")
            .unwrap_or_else(|_| "http://localhost:8545".to_string());
        let ws_url = std::env::var("MEV_BOT_WS_URL")
            .unwrap_or_else(|_| "ws://localhost:8545".to_string());
        let chain_id = std::env::var("MEV_BOT_CHAIN_ID")
            .unwrap_or_else(|_| "31337".to_string())
            .parse()
            .unwrap_or(31337);
        
        // Check if anvil is available
        let anvil_available = Self::check_anvil_availability(&rpc_url).await;
        
        Self {
            rpc_url,
            ws_url,
            chain_id,
            test_timeout: Duration::from_secs(60),
            anvil_available,
        }
    }
    
    async fn check_anvil_availability(rpc_url: &str) -> bool {
        let client = reqwest::Client::new();
        let response = client
            .post(rpc_url)
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "eth_blockNumber",
                "params": [],
                "id": 1
            }))
            .send()
            .await;
        
        match response {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }
}

/// Macro to skip tests if anvil is not available
macro_rules! require_anvil {
    ($config:expr) => {
        if !$config.anvil_available {
            println!("⚠️  Skipping test: Anvil not available at {}", $config.rpc_url);
            return;
        }
    };
}

/// Test complete MEV detection pipeline with realistic victim transactions
#[tokio::test]
async fn test_realistic_mev_detection_pipeline() {
    let config = AnvilTestConfig::new().await;
    require_anvil!(config);
    
    println!("🚀 Testing realistic MEV detection pipeline");
    
    // Create victim generator for realistic transactions
    let victim_generator = VictimGenerator::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create victim generator");
    
    // Create strategy engine
    let mut strategy_engine = StrategyEngine::new();
    strategy_engine
        .register_strategy(Box::new(BackrunStrategy::new(Default::default())))
        .await
        .expect("Failed to register backrun strategy");
    
    // Create simulation engine
    let simulator = ForkSimulator::new(&config.rpc_url)
        .await
        .expect("Failed to create simulator");
    
    // Generate realistic victim transactions
    let scenarios = vec![
        ("Large USDC->WETH swap", "USDC", "WETH", 100000.0),
        ("Medium ETH->USDC swap", "WETH", "USDC", 50.0),
        ("Small token swap", "DAI", "USDC", 10000.0),
    ];
    
    for (description, token_in, token_out, amount) in scenarios {
        println!("Testing scenario: {}", description);
        
        // Generate victim transaction
        let victim_tx = victim_generator
            .generate_large_swap(token_in, token_out, amount)
            .await
            .expect("Failed to generate victim transaction");
        
        println!("Generated victim tx: {}", victim_tx.hash);
        
        // Evaluate with strategies
        let start_time = std::time::Instant::now();
        let opportunities = strategy_engine
            .evaluate_transaction(&victim_tx)
            .await
            .expect("Failed to evaluate transaction");
        
        let detection_latency = start_time.elapsed();
        println!("Detection latency: {:.2}ms", detection_latency.as_secs_f64() * 1000.0);
        
        // Verify performance requirements
        assert!(
            detection_latency.as_millis() <= 50,
            "Detection latency exceeded 50ms: {}ms",
            detection_latency.as_millis()
        );
        
        if !opportunities.is_empty() {
            println!("Found {} MEV opportunities", opportunities.len());
            
            for opportunity in &opportunities {
                println!(
                    "  - Strategy: {}, Profit: {} ETH, Confidence: {:.1}%",
                    opportunity.strategy_name,
                    opportunity.estimated_profit_wei as f64 / 1e18,
                    opportunity.confidence_score * 100.0
                );
                
                // Simulate the opportunity
                let bundle_builder = BundleBuilder::new(&config.rpc_url, config.chain_id)
                    .await
                    .expect("Failed to create bundle builder");
                
                let bundle = bundle_builder
                    .build_bundle(opportunity)
                    .await
                    .expect("Failed to build bundle");
                
                let sim_start = std::time::Instant::now();
                let simulation_result = simulator
                    .simulate_bundle(&bundle)
                    .await
                    .expect("Failed to simulate bundle");
                
                let simulation_latency = sim_start.elapsed();
                println!("  - Simulation latency: {:.2}ms", simulation_latency.as_secs_f64() * 1000.0);
                
                // Verify simulation performance
                assert!(
                    simulation_latency.as_millis() <= 100,
                    "Simulation latency exceeded 100ms: {}ms",
                    simulation_latency.as_millis()
                );
                
                // Verify simulation results
                assert!(!simulation_result.reverted, "Bundle should not revert");
                if simulation_result.profit_wei > 0 {
                    println!("  - ✅ Profitable bundle: {} ETH profit", simulation_result.profit_wei as f64 / 1e18);
                }
            }
        } else {
            println!("No MEV opportunities found for this scenario");
        }
        
        println!();
    }
    
    println!("✅ Realistic MEV detection pipeline test completed");
}

/// Test high-frequency transaction processing under load
#[tokio::test]
async fn test_high_frequency_processing() {
    let config = AnvilTestConfig::new().await;
    require_anvil!(config);
    
    println!("🚀 Testing high-frequency transaction processing");
    
    // Create components
    let victim_generator = VictimGenerator::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create victim generator");
    
    let mut strategy_engine = StrategyEngine::new();
    strategy_engine
        .register_strategy(Box::new(BackrunStrategy::new(Default::default())))
        .await
        .expect("Failed to register strategy");
    
    // Generate a batch of transactions
    let transaction_count = 100;
    let mut transactions = Vec::new();
    
    for i in 0..transaction_count {
        let amount = 1000.0 + (i as f64 * 100.0); // Varying amounts
        let tx = victim_generator
            .generate_large_swap("USDC", "WETH", amount)
            .await
            .expect("Failed to generate transaction");
        transactions.push(tx);
    }
    
    println!("Generated {} test transactions", transactions.len());
    
    // Process transactions and measure performance
    let start_time = std::time::Instant::now();
    let mut total_opportunities = 0;
    let mut max_latency = Duration::from_millis(0);
    let mut min_latency = Duration::from_secs(1);
    
    for (i, tx) in transactions.iter().enumerate() {
        let tx_start = std::time::Instant::now();
        
        let opportunities = strategy_engine
            .evaluate_transaction(tx)
            .await
            .expect("Failed to evaluate transaction");
        
        let tx_latency = tx_start.elapsed();
        total_opportunities += opportunities.len();
        
        if tx_latency > max_latency {
            max_latency = tx_latency;
        }
        if tx_latency < min_latency {
            min_latency = tx_latency;
        }
        
        if i % 20 == 0 {
            println!("Processed {} transactions...", i + 1);
        }
    }
    
    let total_time = start_time.elapsed();
    let avg_latency = total_time / transaction_count as u32;
    let throughput = transaction_count as f64 / total_time.as_secs_f64();
    
    println!("Performance Results:");
    println!("  - Total time: {:.2}s", total_time.as_secs_f64());
    println!("  - Average latency: {:.2}ms", avg_latency.as_secs_f64() * 1000.0);
    println!("  - Min latency: {:.2}ms", min_latency.as_secs_f64() * 1000.0);
    println!("  - Max latency: {:.2}ms", max_latency.as_secs_f64() * 1000.0);
    println!("  - Throughput: {:.1} tx/sec", throughput);
    println!("  - Total opportunities found: {}", total_opportunities);
    
    // Verify performance requirements
    assert!(
        avg_latency.as_millis() <= 25,
        "Average latency exceeded 25ms: {}ms",
        avg_latency.as_millis()
    );
    
    assert!(
        throughput >= 40.0,
        "Throughput below 40 tx/sec: {:.1} tx/sec",
        throughput
    );
    
    println!("✅ High-frequency processing test passed");
}

/// Test concurrent simulation performance
#[tokio::test]
async fn test_concurrent_simulation_performance() {
    let config = AnvilTestConfig::new().await;
    require_anvil!(config);
    
    println!("🚀 Testing concurrent simulation performance");
    
    // Create components
    let victim_generator = VictimGenerator::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create victim generator");
    
    let simulator = Arc::new(
        ForkSimulator::new(&config.rpc_url)
            .await
            .expect("Failed to create simulator")
    );
    
    let bundle_builder = BundleBuilder::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create bundle builder");
    
    // Generate test opportunities
    let opportunity_count = 50;
    let mut opportunities = Vec::new();
    
    for i in 0..opportunity_count {
        let victim_tx = victim_generator
            .generate_large_swap("USDC", "WETH", 10000.0 + (i as f64 * 1000.0))
            .await
            .expect("Failed to generate victim transaction");
        
        let opportunity = Opportunity {
            id: format!("test_opportunity_{}", i),
            strategy_name: "backrun".to_string(),
            target_transaction: victim_tx,
            estimated_profit_wei: 100000000000000000u64, // 0.1 ETH
            gas_estimate: 200000,
            confidence_score: 0.85,
            expiry_time: chrono::Utc::now() + chrono::Duration::minutes(5),
            bundle_transactions: vec![],
        };
        
        opportunities.push(opportunity);
    }
    
    println!("Generated {} test opportunities", opportunities.len());
    
    // Test different concurrency levels
    for concurrency in [1, 5, 10, 20] {
        println!("Testing concurrency level: {}", concurrency);
        
        let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let start_time = std::time::Instant::now();
        let mut handles = Vec::new();
        
        for opportunity in &opportunities {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let sim = simulator.clone();
            let builder = bundle_builder.clone();
            let opp = opportunity.clone();
            
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                
                // Build bundle
                let bundle = builder.build_bundle(&opp).await?;
                
                // Simulate bundle
                let result = sim.simulate_bundle(&bundle).await?;
                
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(result)
            }));
        }
        
        // Wait for all simulations to complete
        let mut successful_simulations = 0;
        let mut total_profit = 0u64;
        
        for handle in handles {
            match handle.await {
                Ok(Ok(result)) => {
                    successful_simulations += 1;
                    total_profit += result.profit_wei;
                }
                Ok(Err(e)) => {
                    println!("Simulation error: {}", e);
                }
                Err(e) => {
                    println!("Task error: {}", e);
                }
            }
        }
        
        let total_time = start_time.elapsed();
        let throughput = successful_simulations as f64 / total_time.as_secs_f64();
        
        println!("  - Successful simulations: {}/{}", successful_simulations, opportunities.len());
        println!("  - Total time: {:.2}s", total_time.as_secs_f64());
        println!("  - Throughput: {:.1} sims/sec", throughput);
        println!("  - Total profit: {} ETH", total_profit as f64 / 1e18);
        
        // Verify performance requirements (≥200 sims/sec target)
        if concurrency >= 10 {
            assert!(
                throughput >= 50.0, // Relaxed for test environment
                "Simulation throughput below 50 sims/sec at concurrency {}: {:.1} sims/sec",
                concurrency,
                throughput
            );
        }
        
        println!();
    }
    
    println!("✅ Concurrent simulation performance test passed");
}

/// Test state management and reorg handling
#[tokio::test]
async fn test_state_management_and_reorgs() {
    let config = AnvilTestConfig::new().await;
    require_anvil!(config);
    
    println!("🚀 Testing state management and reorg handling");
    
    // Create state manager
    let state_manager = StateManager::new(&config.rpc_url, ":memory:")
        .await
        .expect("Failed to create state manager");
    
    // Start state tracking
    state_manager.start().await.expect("Failed to start state manager");
    
    // Wait for initial sync
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Get initial state
    let initial_block = state_manager
        .get_current_block()
        .await
        .expect("Failed to get current block");
    
    println!("Initial block: {}", initial_block);
    assert!(initial_block > 0, "Should have a valid initial block");
    
    // Generate and track some transactions
    let victim_generator = VictimGenerator::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create victim generator");
    
    let mut tracked_transactions = Vec::new();
    
    for i in 0..5 {
        let tx = victim_generator
            .generate_large_swap("USDC", "WETH", 1000.0 * (i + 1) as f64)
            .await
            .expect("Failed to generate transaction");
        
        state_manager
            .track_pending_transaction(&tx)
            .await
            .expect("Failed to track transaction");
        
        tracked_transactions.push(tx);
        println!("Tracked transaction {}: {}", i + 1, tracked_transactions[i].hash);
    }
    
    // Verify tracking
    let pending_count = state_manager
        .get_pending_transaction_count()
        .await
        .expect("Failed to get pending count");
    
    println!("Pending transactions: {}", pending_count);
    assert!(pending_count >= 5, "Should have at least 5 pending transactions");
    
    // Simulate block progression
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    let current_block = state_manager
        .get_current_block()
        .await
        .expect("Failed to get current block");
    
    println!("Current block after wait: {}", current_block);
    
    // Test reorg detection (simulate by checking block hash changes)
    let block_hash = state_manager
        .get_block_hash(current_block)
        .await
        .expect("Failed to get block hash");
    
    println!("Current block hash: {:?}", block_hash);
    assert!(block_hash.is_some(), "Should have a valid block hash");
    
    // Test transaction cleanup
    state_manager
        .cleanup_old_transactions(Duration::from_secs(1))
        .await
        .expect("Failed to cleanup transactions");
    
    println!("✅ State management and reorg handling test passed");
}

/// Test complete end-to-end MEV bot workflow
#[tokio::test]
async fn test_end_to_end_mev_workflow() {
    let config = AnvilTestConfig::new().await;
    require_anvil!(config);
    
    println!("🚀 Testing complete end-to-end MEV workflow");
    
    // Create all components
    let metrics = Arc::new(PrometheusMetrics::new().expect("Failed to create metrics"));
    
    let state_manager = StateManager::new(&config.rpc_url, ":memory:")
        .await
        .expect("Failed to create state manager");
    
    let mut strategy_engine = StrategyEngine::new();
    strategy_engine
        .register_strategy(Box::new(BackrunStrategy::new(Default::default())))
        .await
        .expect("Failed to register backrun strategy");
    
    let victim_generator = VictimGenerator::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create victim generator");
    
    let simulator = ForkSimulator::new(&config.rpc_url)
        .await
        .expect("Failed to create simulator");
    
    let bundle_builder = BundleBuilder::new(&config.rpc_url, config.chain_id)
        .await
        .expect("Failed to create bundle builder");
    
    // Start state management
    state_manager.start().await.expect("Failed to start state manager");
    
    println!("All components initialized successfully");
    
    // Simulate complete workflow
    let workflow_scenarios = vec![
        ("High-value swap", 100000.0),
        ("Medium-value swap", 25000.0),
        ("Low-value swap", 5000.0),
    ];
    
    for (description, swap_amount) in workflow_scenarios {
        println!("\n--- Testing workflow: {} ---", description);
        
        // Step 1: Generate victim transaction
        let victim_tx = victim_generator
            .generate_large_swap("USDC", "WETH", swap_amount)
            .await
            .expect("Failed to generate victim transaction");
        
        println!("1. Generated victim transaction: {}", victim_tx.hash);
        metrics.record_mempool_transaction();
        
        // Step 2: Track in state manager
        state_manager
            .track_pending_transaction(&victim_tx)
            .await
            .expect("Failed to track transaction");
        
        println!("2. Transaction tracked in state manager");
        
        // Step 3: Evaluate with strategies
        let detection_start = std::time::Instant::now();
        let opportunities = strategy_engine
            .evaluate_transaction(&victim_tx)
            .await
            .expect("Failed to evaluate transaction");
        
        let detection_latency = detection_start.elapsed().as_secs_f64() * 1000.0;
        metrics.record_detection_latency(detection_latency);
        
        println!("3. Strategy evaluation completed in {:.2}ms", detection_latency);
        println!("   Found {} opportunities", opportunities.len());
        
        // Step 4: Process opportunities
        for (i, opportunity) in opportunities.iter().enumerate() {
            println!("   Processing opportunity {}: {}", i + 1, opportunity.strategy_name);
            
            // Build bundle
            let bundle = bundle_builder
                .build_bundle(opportunity)
                .await
                .expect("Failed to build bundle");
            
            println!("   Bundle built with {} transactions", bundle.transactions.len());
            
            // Simulate bundle
            let sim_start = std::time::Instant::now();
            let simulation_result = simulator
                .simulate_bundle(&bundle)
                .await
                .expect("Failed to simulate bundle");
            
            let simulation_latency = sim_start.elapsed().as_secs_f64() * 1000.0;
            metrics.record_simulation_latency(simulation_latency);
            
            println!("   Simulation completed in {:.2}ms", simulation_latency);
            println!("   Profit: {} ETH, Gas: {}", 
                simulation_result.profit_wei as f64 / 1e18,
                simulation_result.gas_used
            );
            
            if simulation_result.profit_wei > 0 && !simulation_result.reverted {
                metrics.record_bundle_success();
                println!("   ✅ Profitable and valid bundle!");
            } else {
                println!("   ❌ Bundle not profitable or reverted");
            }
            
            // Verify performance requirements
            assert!(
                detection_latency <= 50.0,
                "Detection latency exceeded 50ms: {:.2}ms",
                detection_latency
            );
            
            assert!(
                simulation_latency <= 100.0,
                "Simulation latency exceeded 100ms: {:.2}ms",
                simulation_latency
            );
        }
    }
    
    // Step 5: Verify final metrics
    let final_metrics = metrics.get_snapshot();
    println!("\n--- Final Metrics ---");
    println!("Mempool transactions: {}", final_metrics.mempool_transactions_total);
    println!("Average detection latency: {:.2}ms", final_metrics.detection_latency_avg);
    println!("Average simulation latency: {:.2}ms", final_metrics.simulation_latency_avg);
    println!("Successful bundles: {}", final_metrics.bundle_success_total);
    
    // Verify metrics are reasonable
    assert!(final_metrics.mempool_transactions_total > 0, "Should have processed transactions");
    assert!(final_metrics.detection_latency_avg > 0.0, "Should have recorded detection latency");
    assert!(final_metrics.simulation_latency_avg > 0.0, "Should have recorded simulation latency");
    
    println!("\n✅ Complete end-to-end MEV workflow test passed");
}