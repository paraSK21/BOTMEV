//! Decision Path Demo - Demonstrates ultra-low latency MEV detection pipeline
//! 
//! This example shows the complete pipeline from mempool ingestion through
//! strategy evaluation to bundle creation, optimized for ≤25ms decision latency.

use mev_core::{
    decision_path::{DecisionPath, DecisionPathConfig},
    decision_path_manager::{DecisionPathManager, DecisionPathManagerConfig},
    metrics::PrometheusMetrics,
    types::{ParsedTransaction, Transaction, TargetType},
};
use mev_core::{
    MockArbitrageStrategy, MockBackrunStrategy, MockStrategyEngine,
};
use std::{sync::Arc, time::Duration};
use tokio::time::sleep;
use tracing::{info, Level};
use tracing_subscriber;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .init();

    info!("🚀 Starting Decision Path Demo");
    info!("This demo showcases ultra-low latency MEV detection (target: ≤25ms)");

    // Run different demo scenarios
    run_basic_decision_path_demo().await?;
    run_performance_benchmark().await?;
    run_full_pipeline_demo().await?;

    info!("✅ Decision Path Demo completed successfully");
    Ok(())
}

/// Demonstrate basic decision path functionality
async fn run_basic_decision_path_demo() -> anyhow::Result<()> {
    info!("\n=== Basic Decision Path Demo ===");

    // Create optimized configuration for low latency
    let config = DecisionPathConfig {
        enable_cpu_pinning: true,
        latency_targets: mev_core::decision_path::LatencyTargets {
            total_decision_loop_ms: 25,
            filter_stage_ms: 5,
            simulation_stage_ms: 15,
            bundle_stage_ms: 5,
            timeout_multiplier: 1.5,
        },
        thread_pools: mev_core::decision_path::ThreadPoolSizes {
            filter_workers: 2,
            simulation_workers: 4,
            bundle_workers: 1,
        },
        memory_pools: mev_core::decision_path::MemoryPoolConfig {
            transaction_pool_size: 1000,
            opportunity_pool_size: 500,
            bundle_pool_size: 100,
            enable_object_reuse: true,
        },
        ..Default::default()
    };

    // Create metrics and strategy engine
    let metrics = Arc::new(PrometheusMetrics::new()?);
    let mut strategy_engine = MockStrategyEngine::new();

    // Register strategies
    strategy_engine.register_strategy(Box::new(MockArbitrageStrategy::new())).await?;
    strategy_engine.register_strategy(Box::new(MockBackrunStrategy::new())).await?;
    
    let strategy_engine = Arc::new(strategy_engine);

    // Create decision path
    let decision_path = Arc::new(DecisionPath::new(
        config,
        strategy_engine,
        metrics,
    )?);

    // Start the pipeline
    decision_path.start().await?;
    info!("✅ Decision path pipeline started");

    // Create test transactions
    let test_transactions = create_test_transactions();
    info!("📝 Created {} test transactions", test_transactions.len());

    // Process transactions and measure latency
    let results_rx = decision_path.get_results();
    
    // Send test transactions
    for (i, tx) in test_transactions.iter().enumerate() {
        decision_path.process_transaction(tx.clone()).await?;
        info!("📤 Sent transaction {} to decision path", i + 1);
        
        // Small delay to avoid overwhelming the pipeline
        sleep(Duration::from_millis(10)).await;
    }

    // Collect results
    let mut results_collected = 0;
    let target_results = test_transactions.len();
    
    info!("📥 Collecting decision results...");
    
    while results_collected < target_results {
        match results_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(result) => {
                results_collected += 1;
                
                let latency_status = if result.processing_time_ms <= 25.0 {
                    "✅ WITHIN TARGET"
                } else {
                    "❌ EXCEEDED TARGET"
                };
                
                info!(
                    "📊 Result {}/{}: {:.2}ms total ({:.1}ms filter, {:.1}ms sim, {:.1}ms bundle) - {} opportunities - {}",
                    results_collected,
                    target_results,
                    result.processing_time_ms,
                    result.stage_timings.filter_ms,
                    result.stage_timings.simulation_ms,
                    result.stage_timings.bundle_ms,
                    result.opportunities.len(),
                    latency_status
                );

                // Log opportunities found
                for opp in &result.opportunities {
                    info!(
                        "  💰 {} opportunity: {:.4} ETH profit (confidence: {:.1}%)",
                        opp.strategy_name,
                        opp.estimated_profit_wei as f64 / 1e18,
                        opp.confidence_score * 100.0
                    );
                }
            }
            Err(_) => {
                info!("⏰ Timeout waiting for results, stopping collection");
                break;
            }
        }
    }

    // Get final statistics
    let stats = decision_path.get_stats().await;
    info!("\n📈 Decision Path Statistics:");
    info!("  Transactions Processed: {}", stats.transactions_processed);
    info!("  Opportunities Found: {}", stats.opportunities_found);
    info!("  Average Latency: {:.2}ms", stats.average_latency_ms);
    info!("  P95 Latency: {:.2}ms", stats.p95_latency_ms);
    info!("  P99 Latency: {:.2}ms", stats.p99_latency_ms);
    info!("  Throughput: {:.1} TPS", stats.throughput_tps);

    decision_path.shutdown().await;
    info!("✅ Basic decision path demo completed\n");

    Ok(())
}

/// Run performance benchmark to validate latency targets
async fn run_performance_benchmark() -> anyhow::Result<()> {
    info!("=== Performance Benchmark ===");
    info!("🎯 Target: Median ≤25ms decision loop latency");

    // Create high-performance configuration
    let config = DecisionPathConfig {
        enable_cpu_pinning: true,
        latency_targets: mev_core::decision_path::LatencyTargets {
            total_decision_loop_ms: 25,
            filter_stage_ms: 3,
            simulation_stage_ms: 18,
            bundle_stage_ms: 4,
            timeout_multiplier: 1.2,
        },
        thread_pools: mev_core::decision_path::ThreadPoolSizes {
            filter_workers: 4,
            simulation_workers: 8,
            bundle_workers: 2,
        },
        channel_buffer_sizes: mev_core::decision_path::ChannelBufferSizes {
            filter_input: 2000,
            simulation_input: 1000,
            bundle_input: 500,
            output: 200,
        },
        memory_pools: mev_core::decision_path::MemoryPoolConfig {
            transaction_pool_size: 2000,
            opportunity_pool_size: 1000,
            bundle_pool_size: 500,
            enable_object_reuse: true,
        },
        ..Default::default()
    };

    let metrics = Arc::new(PrometheusMetrics::new()?);
    let mut strategy_engine = MockStrategyEngine::new();

    // Register strategies
    strategy_engine.register_strategy(Box::new(MockArbitrageStrategy::new())).await?;
    strategy_engine.register_strategy(Box::new(MockBackrunStrategy::new())).await?;
    
    let strategy_engine = Arc::new(strategy_engine);

    let decision_path = Arc::new(DecisionPath::new(
        config,
        strategy_engine,
        metrics,
    )?);

    decision_path.start().await?;
    info!("✅ High-performance pipeline started");

    // Generate load test transactions
    let load_test_transactions = create_load_test_transactions(100);
    info!("📝 Generated {} load test transactions", load_test_transactions.len());

    let results_rx = decision_path.get_results();
    let start_time = std::time::Instant::now();

    // Send all transactions rapidly
    for tx in &load_test_transactions {
        decision_path.process_transaction(tx.clone()).await?;
    }
    
    let send_duration = start_time.elapsed();
    info!("📤 Sent {} transactions in {:.2}ms", load_test_transactions.len(), send_duration.as_secs_f64() * 1000.0);

    // Collect results and measure performance
    let mut latencies = Vec::new();
    let mut opportunities_total = 0;
    let mut results_collected = 0;
    let target_results = load_test_transactions.len();

    while results_collected < target_results {
        match results_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(result) => {
                results_collected += 1;
                latencies.push(result.processing_time_ms);
                opportunities_total += result.opportunities.len();

                if results_collected % 20 == 0 {
                    info!("📊 Processed {}/{} results", results_collected, target_results);
                }
            }
            Err(_) => {
                info!("⏰ Timeout collecting results");
                break;
            }
        }
    }

    let total_duration = start_time.elapsed();

    // Calculate performance metrics
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_latency = if latencies.len() % 2 == 0 {
        (latencies[latencies.len() / 2 - 1] + latencies[latencies.len() / 2]) / 2.0
    } else {
        latencies[latencies.len() / 2]
    };

    let p95_latency = latencies[(latencies.len() as f64 * 0.95) as usize];
    let p99_latency = latencies[(latencies.len() as f64 * 0.99) as usize];
    let mean_latency = latencies.iter().sum::<f64>() / latencies.len() as f64;
    let throughput = results_collected as f64 / total_duration.as_secs_f64();

    info!("\n🏆 Performance Benchmark Results:");
    info!("  Total Duration: {:.2}s", total_duration.as_secs_f64());
    info!("  Results Collected: {}/{}", results_collected, target_results);
    info!("  Throughput: {:.1} TPS", throughput);
    info!("  Opportunities Found: {}", opportunities_total);
    info!("\n📊 Latency Distribution:");
    info!("  Mean: {:.2}ms", mean_latency);
    info!("  Median: {:.2}ms", median_latency);
    info!("  P95: {:.2}ms", p95_latency);
    info!("  P99: {:.2}ms", p99_latency);

    // Performance assessment
    let target_met = median_latency <= 25.0;
    let performance_grade = if median_latency <= 20.0 {
        "🏆 EXCELLENT"
    } else if median_latency <= 25.0 {
        "✅ GOOD"
    } else if median_latency <= 35.0 {
        "⚠️ ACCEPTABLE"
    } else {
        "❌ NEEDS IMPROVEMENT"
    };

    info!("\n🎯 Performance Assessment:");
    info!("  Target: ≤25ms median latency");
    info!("  Achieved: {:.2}ms median latency", median_latency);
    info!("  Status: {} ({})", if target_met { "✅ TARGET MET" } else { "❌ TARGET MISSED" }, performance_grade);

    if throughput >= 100.0 {
        info!("  Throughput: 🏆 EXCELLENT ({:.1} TPS)", throughput);
    } else if throughput >= 50.0 {
        info!("  Throughput: ✅ GOOD ({:.1} TPS)", throughput);
    } else {
        info!("  Throughput: ⚠️ NEEDS IMPROVEMENT ({:.1} TPS)", throughput);
    }

    decision_path.shutdown().await;
    info!("✅ Performance benchmark completed\n");

    Ok(())
}

/// Demonstrate full pipeline with mempool integration
async fn run_full_pipeline_demo() -> anyhow::Result<()> {
    info!("=== Full Pipeline Demo ===");
    info!("🔄 Demonstrating complete mempool → decision path → results pipeline");

    // Note: This would normally connect to a real mempool endpoint
    // For demo purposes, we'll simulate the pipeline behavior
    
    let config = DecisionPathManagerConfig {
        mempool_endpoint: "ws://localhost:8545".to_string(), // Mock endpoint
        enable_backpressure_monitoring: true,
        stats_reporting_interval_seconds: 5,
        ..Default::default()
    };

    info!("📋 Configuration:");
    info!("  CPU Pinning: {}", config.decision_path.enable_cpu_pinning);
    info!("  Target Latency: {}ms", config.decision_path.latency_targets.total_decision_loop_ms);
    info!("  Filter Workers: {}", config.decision_path.thread_pools.filter_workers);
    info!("  Simulation Workers: {}", config.decision_path.thread_pools.simulation_workers);
    info!("  Memory Pool Size: {}", config.decision_path.memory_pools.transaction_pool_size);

    // Note: In a real implementation, this would start the full pipeline
    // For demo purposes, we'll show what the integration would look like
    
    info!("🔧 Pipeline Components:");
    info!("  ✅ Mempool Service (WebSocket connection)");
    info!("  ✅ Transaction Filter (Fast pre-screening)");
    info!("  ✅ Strategy Engine (Parallel evaluation)");
    info!("  ✅ Bundle Builder (Opportunity packaging)");
    info!("  ✅ Metrics Collection (Prometheus export)");
    info!("  ✅ Backpressure Monitoring (Queue management)");

    info!("\n🎯 Performance Targets:");
    info!("  Detection Latency: ≤20ms (mempool → local)");
    info!("  Decision Latency: ≤25ms (detection → bundle)");
    info!("  Simulation Throughput: ≥200 sims/sec");
    info!("  Memory Usage: Bounded pools, zero-allocation hot path");
    info!("  CPU Utilization: Pinned cores, optimized thread pools");

    info!("\n📊 Expected Performance:");
    info!("  Throughput: 500-1000 TPS (depending on hardware)");
    info!("  Latency P95: <30ms end-to-end");
    info!("  Memory: <100MB steady state");
    info!("  CPU: 4-8 cores for optimal performance");

    info!("✅ Full pipeline demo completed");
    info!("💡 To run with real mempool data, configure a valid WebSocket endpoint");

    Ok(())
}

/// Create test transactions for demonstration
fn create_test_transactions() -> Vec<ParsedTransaction> {
    let mut transactions = Vec::new();
    
    // UniswapV2 swap transaction
    transactions.push(ParsedTransaction {
        transaction: Transaction {
            hash: "0x1111111111111111111111111111111111111111111111111111111111111111".to_string(),
            from: "0xuser1".to_string(),
            to: Some("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D".to_string()), // Uniswap V2 Router
            value: "2000000000000000000".to_string(), // 2 ETH
            gas_price: "50000000000".to_string(), // 50 gwei
            gas_limit: "200000".to_string(),
            nonce: 1,
            input: "0x38ed1739000000000000000000000000000000000000000000000001a055690d9db80000".to_string(),
            timestamp: chrono::Utc::now(),
        },
        decoded_input: None,
        target_type: TargetType::UniswapV2,
        processing_time_ms: 1,
    });

    // Large Curve swap
    transactions.push(ParsedTransaction {
        transaction: Transaction {
            hash: "0x2222222222222222222222222222222222222222222222222222222222222222".to_string(),
            from: "0xuser2".to_string(),
            to: Some("0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7".to_string()), // Curve 3pool
            value: "0".to_string(),
            gas_price: "75000000000".to_string(), // 75 gwei
            gas_limit: "300000".to_string(),
            nonce: 42,
            input: "0x5b36389c000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000056bc75e2d630eb364".to_string(),
            timestamp: chrono::Utc::now(),
        },
        decoded_input: None,
        target_type: TargetType::Curve,
        processing_time_ms: 1,
    });

    // SushiSwap transaction
    transactions.push(ParsedTransaction {
        transaction: Transaction {
            hash: "0x3333333333333333333333333333333333333333333333333333333333333333".to_string(),
            from: "0xuser3".to_string(),
            to: Some("0xd9e1cE17f2641f24aE83637ab66a2cca9C378B9F".to_string()), // SushiSwap Router
            value: "1000000000000000000".to_string(), // 1 ETH
            gas_price: "60000000000".to_string(), // 60 gwei
            gas_limit: "250000".to_string(),
            nonce: 15,
            input: "0x38ed1739000000000000000000000000000000000000000000000000de0b6b3a7640000".to_string(),
            timestamp: chrono::Utc::now(),
        },
        decoded_input: None,
        target_type: TargetType::SushiSwap,
        processing_time_ms: 1,
    });

    transactions
}

/// Create load test transactions for performance benchmarking
fn create_load_test_transactions(count: usize) -> Vec<ParsedTransaction> {
    let mut transactions = Vec::new();
    let target_types = [
        TargetType::UniswapV2,
        TargetType::UniswapV3,
        TargetType::SushiSwap,
        TargetType::Curve,
        TargetType::Balancer,
    ];

    for i in 0..count {
        let target_type = target_types[i % target_types.len()].clone();
        let hash = format!("0x{:064x}", i);
        let value = format!("{}", 1000000000000000000u64 + (i as u64 * 100000000000000000)); // 1+ ETH
        
        transactions.push(ParsedTransaction {
            transaction: Transaction {
                hash,
                from: format!("0xuser{:x}", i),
                to: Some(format!("0xcontract{:x}", i)),
                value,
                gas_price: format!("{}", 50000000000u64 + (i as u64 % 50) * 1000000000), // 50-100 gwei
                gas_limit: "200000".to_string(),
                nonce: i as u64,
                input: format!("0x38ed1739{:056x}", i),
                timestamp: chrono::Utc::now(),
            },
            decoded_input: None,
            target_type,
            processing_time_ms: 1,
        });
    }

    transactions
}