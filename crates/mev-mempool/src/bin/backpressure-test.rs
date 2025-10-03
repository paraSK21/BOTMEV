//! Backpressure testing utility for MEV bot
//! 
//! This tool tests the backpressure handling system under various load conditions
//! and validates queue behavior under high concurrency.

use anyhow::Result;
use mev_core::{CorePinningConfig, CpuAffinityManager, PrometheusMetrics};
use mev_mempool::{BackpressureBuffer, BackpressureConfig, BackpressureLoadTester, DropPolicy};
use std::{sync::Arc, time::Duration};
use tokio::{signal, time::sleep};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("info,mev_mempool=debug")
        .with_target(false)
        .with_thread_ids(true)
        .init();

    info!("Starting MEV Bot Backpressure Test");

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let test_type = args.get(1).map(|s| s.as_str()).unwrap_or("basic");

    match test_type {
        "basic" => run_basic_test().await?,
        "load" => run_load_test().await?,
        "concurrency" => run_concurrency_test().await?,
        "drop-policies" => run_drop_policy_test().await?,
        "cpu-pinning" => run_cpu_pinning_test().await?,
        _ => {
            println!("Usage: backpressure-test [basic|load|concurrency|drop-policies|cpu-pinning]");
            return Ok(());
        }
    }

    Ok(())
}

async fn run_basic_test() -> Result<()> {
    info!("Running basic backpressure test");

    let metrics = Arc::new(PrometheusMetrics::new()?);
    let config = BackpressureConfig {
        enabled: true,
        buffer_size: 1000,
        high_watermark: 0.8,
        low_watermark: 0.6,
        drop_policy: DropPolicy::Oldest,
    };

    let buffer = Arc::new(BackpressureBuffer::new(config, metrics));
    let load_tester = BackpressureLoadTester::new(buffer.clone(), 100, 30); // 100 TPS for 30 seconds

    // Start consumer task
    let buffer_clone = buffer.clone();
    let consumer_handle = tokio::spawn(async move {
        let mut processed = 0u64;
        loop {
            let batch = buffer_clone.receive_batch(10).await;
            if batch.is_empty() {
                sleep(Duration::from_millis(1)).await;
                continue;
            }
            
            processed += batch.len() as u64;
            
            // Simulate processing time
            sleep(Duration::from_millis(5)).await;
            
            if processed % 100 == 0 {
                info!("Consumer processed {} transactions", processed);
            }
        }
    });

    // Run load test
    let stats = load_tester.run_test().await?;
    stats.print_summary();

    // Stop consumer
    consumer_handle.abort();

    // Validate results
    validate_basic_test_results(&stats)?;

    Ok(())
}

async fn run_load_test() -> Result<()> {
    info!("Running high-load backpressure test (10k TPS)");

    let metrics = Arc::new(PrometheusMetrics::new()?);
    let config = BackpressureConfig {
        enabled: true,
        buffer_size: 5000,
        high_watermark: 0.9,
        low_watermark: 0.7,
        drop_policy: DropPolicy::Oldest,
    };

    let buffer = Arc::new(BackpressureBuffer::new(config, metrics));
    let load_tester = BackpressureLoadTester::new(buffer.clone(), 10000, 60); // 10k TPS for 60 seconds

    // Start multiple consumer tasks for higher throughput
    let mut consumer_handles = Vec::new();
    for i in 0..4 {
        let buffer_clone = buffer.clone();
        let handle = tokio::spawn(async move {
            let mut processed = 0u64;
            loop {
                let batch = buffer_clone.receive_batch(50).await;
                if batch.is_empty() {
                    sleep(Duration::from_millis(1)).await;
                    continue;
                }
                
                processed += batch.len() as u64;
                
                // Simulate minimal processing time for high throughput
                sleep(Duration::from_micros(100)).await;
                
                if processed % 1000 == 0 {
                    info!("Consumer {} processed {} transactions", i, processed);
                }
            }
        });
        consumer_handles.push(handle);
    }

    // Run load test
    let stats = load_tester.run_test().await?;
    stats.print_summary();

    // Stop consumers
    for handle in consumer_handles {
        handle.abort();
    }

    // Validate high-load results
    validate_load_test_results(&stats)?;

    Ok(())
}

async fn run_concurrency_test() -> Result<()> {
    info!("Running concurrency test with multiple producers");

    let metrics = Arc::new(PrometheusMetrics::new()?);
    let config = BackpressureConfig {
        enabled: true,
        buffer_size: 2000,
        high_watermark: 0.8,
        low_watermark: 0.6,
        drop_policy: DropPolicy::Random,
    };

    let buffer = Arc::new(BackpressureBuffer::new(config, metrics));

    // Start consumer
    let buffer_clone = buffer.clone();
    let consumer_handle = tokio::spawn(async move {
        let mut processed = 0u64;
        loop {
            let batch = buffer_clone.receive_batch(20).await;
            if batch.is_empty() {
                sleep(Duration::from_millis(1)).await;
                continue;
            }
            
            processed += batch.len() as u64;
            sleep(Duration::from_millis(2)).await;
            
            if processed % 500 == 0 {
                info!("Consumer processed {} transactions", processed);
            }
        }
    });

    // Start multiple producer tasks
    let mut producer_handles = Vec::new();
    for producer_id in 0..8 {
        let buffer_clone = buffer.clone();
        let handle = tokio::spawn(async move {
            let load_tester = BackpressureLoadTester::new(buffer_clone, 500, 45); // 500 TPS each
            if let Err(e) = load_tester.run_test().await {
                error!("Producer {} error: {}", producer_id, e);
            }
        });
        producer_handles.push(handle);
    }

    // Wait for all producers to complete
    for (i, handle) in producer_handles.into_iter().enumerate() {
        if let Err(e) = handle.await {
            warn!("Producer {} task error: {}", i, e);
        }
    }

    // Get final stats
    let stats = buffer.get_stats();
    stats.print_summary();

    // Stop consumer
    consumer_handle.abort();

    // Validate concurrency results
    validate_concurrency_test_results(&stats)?;

    Ok(())
}

async fn run_drop_policy_test() -> Result<()> {
    info!("Testing different drop policies");

    let policies = vec![
        ("Oldest", DropPolicy::Oldest),
        ("Newest", DropPolicy::Newest),
        ("Random", DropPolicy::Random),
    ];

    for (name, policy) in policies {
        info!("Testing {} drop policy", name);

        let metrics = Arc::new(PrometheusMetrics::new()?);
        let config = BackpressureConfig {
            enabled: true,
            buffer_size: 500,
            high_watermark: 0.7,
            low_watermark: 0.5,
            drop_policy: policy,
        };

        let buffer = Arc::new(BackpressureBuffer::new(config, metrics));
        let load_tester = BackpressureLoadTester::new(buffer.clone(), 2000, 20); // High load to trigger drops

        // Slow consumer to ensure backpressure
        let buffer_clone = buffer.clone();
        let consumer_handle = tokio::spawn(async move {
            loop {
                let batch = buffer_clone.receive_batch(5).await;
                if !batch.is_empty() {
                    sleep(Duration::from_millis(10)).await; // Slow processing
                }
            }
        });

        let stats = load_tester.run_test().await?;
        consumer_handle.abort();

        info!(
            policy = name,
            drop_rate = format!("{:.2}%", stats.drop_rate * 100.0),
            utilization = format!("{:.1}%", stats.utilization * 100.0),
            "Drop policy test results"
        );
    }

    Ok(())
}

async fn run_cpu_pinning_test() -> Result<()> {
    info!("Testing CPU core pinning performance");

    // Test without CPU pinning
    info!("Running test WITHOUT CPU pinning");
    let stats_without = run_cpu_test(false).await?;

    // Test with CPU pinning
    info!("Running test WITH CPU pinning");
    let stats_with = run_cpu_test(true).await?;

    // Compare results
    info!("=== CPU Pinning Performance Comparison ===");
    info!(
        "Without pinning - TPS: {:.2}, Drop rate: {:.2}%",
        calculate_tps(&stats_without),
        stats_without.drop_rate * 100.0
    );
    info!(
        "With pinning - TPS: {:.2}, Drop rate: {:.2}%",
        calculate_tps(&stats_with),
        stats_with.drop_rate * 100.0
    );

    let improvement = (calculate_tps(&stats_with) - calculate_tps(&stats_without)) 
        / calculate_tps(&stats_without) * 100.0;
    
    if improvement > 0.0 {
        info!("✅ CPU pinning improved performance by {:.1}%", improvement);
    } else {
        info!("❌ CPU pinning decreased performance by {:.1}%", improvement.abs());
    }

    Ok(())
}

async fn run_cpu_test(enable_pinning: bool) -> Result<mev_mempool::BackpressureStats> {
    let cpu_config = CorePinningConfig {
        enabled: enable_pinning,
        core_ids: vec![0, 1, 2, 3], // Use first 4 cores
        worker_threads: 4,
    };

    if enable_pinning {
        let cpu_manager = CpuAffinityManager::new(cpu_config);
        cpu_manager.initialize()?;
    }

    let metrics = Arc::new(PrometheusMetrics::new()?);
    let config = BackpressureConfig {
        enabled: true,
        buffer_size: 3000,
        high_watermark: 0.8,
        low_watermark: 0.6,
        drop_policy: DropPolicy::Oldest,
    };

    let buffer = Arc::new(BackpressureBuffer::new(config, metrics));
    let load_tester = BackpressureLoadTester::new(buffer.clone(), 5000, 30); // 5k TPS for 30 seconds

    // Start consumer with potential CPU pinning
    let buffer_clone = buffer.clone();
    let consumer_handle = tokio::spawn(async move {
        loop {
            let batch = buffer_clone.receive_batch(25).await;
            if !batch.is_empty() {
                // Simulate CPU-intensive processing
                for _ in 0..batch.len() {
                    let _ = (0..1000).fold(0u64, |acc, x| acc.wrapping_add(x));
                }
            }
        }
    });

    let stats = load_tester.run_test().await?;
    consumer_handle.abort();

    Ok(stats)
}

fn calculate_tps(stats: &mev_mempool::BackpressureStats) -> f64 {
    if stats.total_received > 0 {
        stats.total_processed as f64 / 30.0 // Assuming 30-second test duration
    } else {
        0.0
    }
}

fn validate_basic_test_results(stats: &mev_mempool::BackpressureStats) -> Result<()> {
    info!("=== Basic Test Validation ===");

    // Check that we processed a reasonable number of transactions
    if stats.total_processed < 1000 {
        warn!("❌ Low transaction processing: {}", stats.total_processed);
    } else {
        info!("✅ Processed {} transactions", stats.total_processed);
    }

    // Check drop rate is reasonable
    if stats.drop_rate > 0.1 {
        warn!("❌ High drop rate: {:.2}%", stats.drop_rate * 100.0);
    } else {
        info!("✅ Drop rate acceptable: {:.2}%", stats.drop_rate * 100.0);
    }

    // Check buffer utilization
    if stats.utilization > 0.95 {
        warn!("❌ Buffer utilization too high: {:.1}%", stats.utilization * 100.0);
    } else {
        info!("✅ Buffer utilization: {:.1}%", stats.utilization * 100.0);
    }

    Ok(())
}

fn validate_load_test_results(stats: &mev_mempool::BackpressureStats) -> Result<()> {
    info!("=== Load Test Validation ===");

    let target_tps = 10000.0;
    let actual_tps = calculate_tps(stats);

    if actual_tps < target_tps * 0.5 {
        warn!("❌ Low throughput: {:.2} TPS (target: {:.2})", actual_tps, target_tps);
    } else {
        info!("✅ Throughput: {:.2} TPS", actual_tps);
    }

    // Under high load, some drops are expected
    if stats.drop_rate > 0.5 {
        warn!("❌ Very high drop rate: {:.2}%", stats.drop_rate * 100.0);
    } else {
        info!("✅ Drop rate under load: {:.2}%", stats.drop_rate * 100.0);
    }

    Ok(())
}

fn validate_concurrency_test_results(stats: &mev_mempool::BackpressureStats) -> Result<()> {
    info!("=== Concurrency Test Validation ===");

    // With 8 producers at 500 TPS each, we expect high throughput
    let expected_min_processed = 8 * 500 * 45 / 2; // At least 50% of target
    
    if stats.total_processed < expected_min_processed as u64 {
        warn!("❌ Low concurrent processing: {}", stats.total_processed);
    } else {
        info!("✅ Concurrent processing: {}", stats.total_processed);
    }

    // Check that backpressure was activated
    if !stats.backpressure_active && stats.drop_rate == 0.0 {
        warn!("❌ Backpressure may not have been tested properly");
    } else {
        info!("✅ Backpressure system activated");
    }

    Ok(())
}