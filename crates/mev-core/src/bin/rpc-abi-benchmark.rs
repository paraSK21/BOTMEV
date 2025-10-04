//! Performance benchmark for RPC and ABI decoder optimizations

use anyhow::Result;
use mev_core::{
    OptimizedAbiDecoder, OptimizedRpcClient, RpcClientConfig, BinaryRpcClient,
};
use std::{time::Instant};
use tokio::time::{sleep, Duration};
use tracing::{info, warn};
use tracing_subscriber;
use ethers::types::Bytes;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("Starting RPC and ABI Decoder Performance Benchmark");

    // Run ABI decoder benchmarks
    benchmark_abi_decoder().await?;

    // Run RPC client benchmarks (if RPC endpoint is available)
    if let Ok(rpc_url) = std::env::var("RPC_URL") {
        benchmark_rpc_client(&rpc_url).await?;
        benchmark_binary_vs_json_rpc(&rpc_url).await?;
    } else {
        warn!("RPC_URL not set, skipping RPC benchmarks");
    }

    Ok(())
}

async fn benchmark_abi_decoder() -> Result<()> {
    info!("=== ABI Decoder Benchmark ===");

    let mut decoder = OptimizedAbiDecoder::new();
    
    // Warm up the decoder
    decoder.warmup()?;

    // Test data: various transaction types
    let test_inputs = vec![
        // ERC20 transfer
        ("ERC20 Transfer", "a9059cbb000000000000000000000000742d35cc6634c0532925a3b8d4c9db96c4b5dd36000000000000000000000000000000000000000000000000000000000000000a"),
        // Uniswap V2 swap
        ("Uniswap V2 Swap", "7ff36ab50000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"),
        // ERC20 approve
        ("ERC20 Approve", "095ea7b3000000000000000000000000742d35cc6634c0532925a3b8d4c9db96c4b5dd36ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"),
        // Unknown function
        ("Unknown Function", "12345678000000000000000000000000742d35cc6634c0532925a3b8d4c9db96c4b5dd36000000000000000000000000000000000000000000000000000000000000000a"),
    ];

    // Single decode benchmark
    for (name, hex_data) in &test_inputs {
        let input_bytes = hex::decode(hex_data)?;
        let input = Bytes::from(input_bytes);

        let start = Instant::now();
        let result = decoder.decode_input(&input)?;
        let duration = start.elapsed();

        info!(
            test_case = name,
            duration_ns = duration.as_nanos(),
            success = result.is_some(),
            "Single decode benchmark"
        );
    }

    // Batch decode benchmark
    let batch_inputs: Vec<Bytes> = test_inputs
        .iter()
        .map(|(_, hex_data)| Bytes::from(hex::decode(hex_data).unwrap()))
        .collect();

    let batch_input_refs: Vec<&Bytes> = batch_inputs.iter().collect();

    let start = Instant::now();
    let batch_results = decoder.decode_batch(batch_input_refs)?;
    let batch_duration = start.elapsed();

    info!(
        batch_size = batch_results.len(),
        total_duration_ns = batch_duration.as_nanos(),
        avg_duration_ns = batch_duration.as_nanos() / batch_results.len() as u128,
        successful_decodes = batch_results.iter().filter(|r| r.is_some()).count(),
        "Batch decode benchmark"
    );

    // High-volume benchmark
    let iterations = 10000;
    let test_input = Bytes::from(hex::decode(&test_inputs[0].1)?);

    let start = Instant::now();
    for _ in 0..iterations {
        let _ = decoder.decode_input(&test_input)?;
    }
    let high_volume_duration = start.elapsed();

    info!(
        iterations = iterations,
        total_duration_ms = high_volume_duration.as_millis(),
        avg_duration_ns = high_volume_duration.as_nanos() / iterations,
        throughput_per_sec = iterations as f64 / high_volume_duration.as_secs_f64(),
        "High-volume decode benchmark"
    );

    // Print decoder statistics
    let stats = decoder.get_stats();
    info!(
        total_decodes = stats.total_decodes,
        successful_decodes = stats.successful_decodes,
        cache_hits = stats.cache_hits,
        avg_decode_time_ns = stats.avg_decode_time_ns,
        decoder_pool_utilization = format!("{:.2}%", stats.decoder_pool_utilization * 100.0),
        "ABI Decoder Statistics"
    );

    Ok(())
}

async fn benchmark_rpc_client(rpc_url: &str) -> Result<()> {
    info!("=== RPC Client Benchmark ===");

    let config = RpcClientConfig {
        endpoints: vec![rpc_url.to_string()],
        timeout_ms: 5000,
        max_concurrent_requests: 100,
        connection_pool_size: 20,
        retry_attempts: 3,
        retry_delay_ms: 100,
        enable_failover: true,
        health_check_interval_ms: 30000,
    };

    let client = OptimizedRpcClient::new(config).await?;

    // Single request benchmark
    let start = Instant::now();
    match client.get_block_number().await {
        Ok(block_number) => {
            let duration = start.elapsed();
            info!(
                block_number = %block_number,
                duration_ms = duration.as_millis(),
                "Single RPC request benchmark"
            );
        }
        Err(e) => {
            warn!("Single RPC request failed: {}", e);
        }
    }

    // Concurrent requests benchmark
    let concurrent_requests = 50;
    let start = Instant::now();
    
    let mut handles = Vec::new();
    for _ in 0..concurrent_requests {
        let client_clone = client.clone();
        let handle = tokio::spawn(async move {
            client_clone.get_block_number().await
        });
        handles.push(handle);
    }

    let mut successful_requests = 0;
    for handle in handles {
        match handle.await? {
            Ok(_) => successful_requests += 1,
            Err(e) => warn!("Concurrent request failed: {}", e),
        }
    }

    let concurrent_duration = start.elapsed();
    info!(
        concurrent_requests = concurrent_requests,
        successful_requests = successful_requests,
        total_duration_ms = concurrent_duration.as_millis(),
        avg_duration_ms = concurrent_duration.as_millis() / concurrent_requests,
        throughput_per_sec = successful_requests as f64 / concurrent_duration.as_secs_f64(),
        "Concurrent RPC requests benchmark"
    );

    // Batch requests benchmark
    let batch_size = 10;
    let start = Instant::now();
    let batch_results = client.get_block_numbers_batch(batch_size).await?;
    let batch_duration = start.elapsed();

    info!(
        batch_size = batch_size,
        successful_results = batch_results.len(),
        total_duration_ms = batch_duration.as_millis(),
        avg_duration_ms = batch_duration.as_millis() / batch_size as u128,
        "Batch RPC requests benchmark"
    );

    // Print RPC client statistics
    let stats = client.get_stats().await;
    info!(
        healthy_endpoints = format!("{}/{}", stats.healthy_endpoints, stats.total_endpoints),
        avg_response_time_ms = format!("{:.2}", stats.avg_response_time_ms),
        success_rate = format!("{:.2}%", stats.success_rate * 100.0),
        total_requests = stats.total_requests,
        active_requests = format!("{}/{}", stats.active_requests, stats.max_concurrent_requests),
        "RPC Client Statistics"
    );

    Ok(())
}

async fn benchmark_binary_vs_json_rpc(rpc_url: &str) -> Result<()> {
    info!("=== Binary vs JSON RPC Benchmark ===");

    // JSON RPC benchmark (using existing optimized client)
    let config = RpcClientConfig {
        endpoints: vec![rpc_url.to_string()],
        timeout_ms: 5000,
        max_concurrent_requests: 50,
        connection_pool_size: 10,
        retry_attempts: 1,
        retry_delay_ms: 100,
        enable_failover: false,
        health_check_interval_ms: 60000,
    };

    let json_client = OptimizedRpcClient::new(config).await?;
    
    // Binary RPC benchmark (placeholder implementation)
    let mut binary_client = BinaryRpcClient::new(rpc_url.to_string());

    let iterations = 100;

    // JSON RPC benchmark
    let start = Instant::now();
    let mut json_successful = 0;
    for _ in 0..iterations {
        match json_client.get_block_number().await {
            Ok(_) => json_successful += 1,
            Err(_) => {}
        }
    }
    let json_duration = start.elapsed();

    info!(
        protocol = "JSON RPC",
        iterations = iterations,
        successful = json_successful,
        total_duration_ms = json_duration.as_millis(),
        avg_duration_ms = json_duration.as_millis() / iterations,
        throughput_per_sec = json_successful as f64 / json_duration.as_secs_f64(),
        "JSON RPC benchmark results"
    );

    // Binary RPC benchmark (simulated)
    let start = Instant::now();
    let mut binary_successful = 0;
    for _ in 0..iterations {
        match binary_client.send_binary_request("eth_blockNumber", &[]).await {
            Ok(_) => binary_successful += 1,
            Err(_) => {}
        }
    }
    let binary_duration = start.elapsed();

    info!(
        protocol = "Binary RPC",
        iterations = iterations,
        successful = binary_successful,
        total_duration_ms = binary_duration.as_millis(),
        avg_duration_ms = binary_duration.as_millis() / iterations,
        throughput_per_sec = binary_successful as f64 / binary_duration.as_secs_f64(),
        "Binary RPC benchmark results"
    );

    // Performance comparison
    if json_successful > 0 && binary_successful > 0 {
        let json_avg_ms = json_duration.as_millis() as f64 / json_successful as f64;
        let binary_avg_ms = binary_duration.as_millis() as f64 / binary_successful as f64;
        let improvement = ((json_avg_ms - binary_avg_ms) / json_avg_ms) * 100.0;

        info!(
            json_avg_ms = format!("{:.2}", json_avg_ms),
            binary_avg_ms = format!("{:.2}", binary_avg_ms),
            improvement_percent = format!("{:.2}%", improvement),
            "Performance comparison (negative = binary is slower)"
        );
    }

    // Print binary RPC statistics
    let binary_stats = binary_client.get_stats();
    info!(
        total_requests = binary_stats.total_requests,
        successful_requests = binary_stats.successful_requests,
        avg_response_time_ms = format!("{:.2}", binary_stats.avg_response_time_ms),
        bytes_sent = binary_stats.bytes_sent,
        bytes_received = binary_stats.bytes_received,
        "Binary RPC Statistics"
    );

    Ok(())
}

/// Simulate load testing with concurrent ABI decoding and RPC calls
async fn simulate_production_load() -> Result<()> {
    info!("=== Production Load Simulation ===");

    let mut decoder = OptimizedAbiDecoder::new();
    decoder.warmup()?;

    // Simulate 1000 transactions per second for 10 seconds
    let duration = Duration::from_secs(10);
    let target_tps = 1000;
    let interval = Duration::from_millis(1000 / target_tps);

    let test_input = Bytes::from(hex::decode("a9059cbb000000000000000000000000742d35cc6634c0532925a3b8d4c9db96c4b5dd36000000000000000000000000000000000000000000000000000000000000000a")?);

    let start_time = Instant::now();
    let mut processed_count = 0;
    let mut successful_count = 0;

    while start_time.elapsed() < duration {
        let decode_start = Instant::now();
        
        match decoder.decode_input(&test_input) {
            Ok(Some(_)) => {
                successful_count += 1;
                processed_count += 1;
            }
            Ok(None) => {
                processed_count += 1;
            }
            Err(_) => {
                processed_count += 1;
            }
        }

        let decode_duration = decode_start.elapsed();
        
        // Maintain target rate
        if decode_duration < interval {
            sleep(interval - decode_duration).await;
        }
    }

    let total_duration = start_time.elapsed();
    let actual_tps = processed_count as f64 / total_duration.as_secs_f64();

    info!(
        target_tps = target_tps,
        actual_tps = format!("{:.2}", actual_tps),
        processed_count = processed_count,
        successful_count = successful_count,
        success_rate = format!("{:.2}%", (successful_count as f64 / processed_count as f64) * 100.0),
        duration_secs = total_duration.as_secs(),
        "Production load simulation results"
    );

    Ok(())
}