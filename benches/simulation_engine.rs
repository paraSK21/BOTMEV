//! Benchmark tests for simulation engine performance
//! 
//! These benchmarks validate that the simulation engine meets the
//! performance requirements of ≥200 sims/sec and ≤25ms decision latency.

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use std::time::Duration;
use tokio::runtime::Runtime;

use mev_core::{
    simulation::ForkSimulator,
    bundle::{Bundle, BundleBuilder},
    types::{Opportunity, SimulationResult},
};

/// Benchmark simulation throughput
fn bench_simulation_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    // Create simulator (using mock RPC for benchmarking)
    let simulator = rt.block_on(async {
        ForkSimulator::new("http://localhost:8545")
            .await
            .expect("Failed to create simulator")
    });
    
    // Generate test bundles
    let test_bundles = generate_test_bundles(200);
    
    let mut group = c.benchmark_group("simulation_throughput");
    group.measurement_time(Duration::from_secs(30));
    group.sample_size(50);
    
    // Test different batch sizes
    for batch_size in [1, 10, 50, 100, 200].iter() {
        group.bench_with_input(
            BenchmarkId::new("simulate_bundles", batch_size),
            batch_size,
            |b, &batch_size| {
                let bundles = &test_bundles[..batch_size];
                b.to_async(&rt).iter(|| async {
                    let mut handles = Vec::new();
                    
                    for bundle in bundles {
                        let sim = simulator.clone();
                        let bundle = bundle.clone();
                        handles.push(tokio::spawn(async move {
                            sim.simulate_bundle(&bundle).await
                        }));
                    }
                    
                    // Wait for all simulations to complete
                    for handle in handles {
                        let _ = black_box(handle.await);
                    }
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark decision loop latency
fn bench_decision_loop_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let simulator = rt.block_on(async {
        ForkSimulator::new("http://localhost:8545")
            .await
            .expect("Failed to create simulator")
    });
    
    let bundle_builder = rt.block_on(async {
        BundleBuilder::new("http://localhost:8545", 31337)
            .await
            .expect("Failed to create bundle builder")
    });
    
    let test_opportunities = generate_test_opportunities(100);
    
    let mut group = c.benchmark_group("decision_loop");
    group.measurement_time(Duration::from_secs(10));
    
    group.bench_function("complete_decision_loop", |b| {
        b.to_async(&rt).iter(|| async {
            for opportunity in &test_opportunities {
                let start = std::time::Instant::now();
                
                // Build bundle
                let bundle = bundle_builder
                    .build_bundle(opportunity)
                    .await
                    .expect("Failed to build bundle");
                
                // Simulate bundle
                let _result = simulator
                    .simulate_bundle(&bundle)
                    .await
                    .expect("Failed to simulate bundle");
                
                let latency = start.elapsed();
                black_box(latency);
                
                // Verify latency requirement (≤25ms)
                assert!(
                    latency.as_millis() <= 25,
                    "Decision loop latency exceeded 25ms: {}ms",
                    latency.as_millis()
                );
            }
        });
    });
    
    group.finish();
}

/// Benchmark gas estimation accuracy and speed
fn bench_gas_estimation(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let simulator = rt.block_on(async {
        ForkSimulator::new("http://localhost:8545")
            .await
            .expect("Failed to create simulator")
    });
    
    let test_bundles = generate_test_bundles(100);
    
    let mut group = c.benchmark_group("gas_estimation");
    
    group.bench_function("estimate_gas_batch", |b| {
        b.to_async(&rt).iter(|| async {
            for bundle in &test_bundles {
                let _gas_estimate = black_box(
                    simulator.estimate_bundle_gas(bundle).await
                );
            }
        });
    });
    
    group.finish();
}

/// Benchmark profit calculation performance
fn bench_profit_calculation(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let simulator = rt.block_on(async {
        ForkSimulator::new("http://localhost:8545")
            .await
            .expect("Failed to create simulator")
    });
    
    let test_bundles = generate_test_bundles(100);
    
    let mut group = c.benchmark_group("profit_calculation");
    
    group.bench_function("calculate_profit_batch", |b| {
        b.to_async(&rt).iter(|| async {
            for bundle in &test_bundles {
                let _profit = black_box(
                    simulator.calculate_bundle_profit(bundle).await
                );
            }
        });
    });
    
    group.finish();
}

/// Benchmark concurrent simulation performance
fn bench_concurrent_simulations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let simulator = rt.block_on(async {
        ForkSimulator::new("http://localhost:8545")
            .await
            .expect("Failed to create simulator")
    });
    
    let test_bundles = generate_test_bundles(500);
    
    let mut group = c.benchmark_group("concurrent_simulations");
    group.measurement_time(Duration::from_secs(30));
    
    for concurrency in [1, 5, 10, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::new("concurrent_sims", concurrency),
            concurrency,
            |b, &concurrency| {
                b.to_async(&rt).iter(|| async {
                    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
                    let mut handles = Vec::new();
                    
                    for bundle in &test_bundles[..200] {
                        let permit = semaphore.clone().acquire_owned().await.unwrap();
                        let sim = simulator.clone();
                        let bundle = bundle.clone();
                        
                        handles.push(tokio::spawn(async move {
                            let _permit = permit;
                            sim.simulate_bundle(&bundle).await
                        }));
                    }
                    
                    // Wait for all simulations
                    for handle in handles {
                        let _ = black_box(handle.await);
                    }
                });
            },
        );
    }
    
    group.finish();
}

/// Generate test bundles for benchmarking
fn generate_test_bundles(count: usize) -> Vec<Bundle> {
    use ethers::types::{H256, Address, U256, Bytes, Transaction};
    use mev_core::bundle::SignedTransaction;
    
    (0..count)
        .map(|i| {
            let tx = SignedTransaction {
                transaction: Transaction {
                    hash: H256::from_low_u64_be(i as u64),
                    from: Address::from_low_u64_be(i as u64),
                    to: Some(Address::from_low_u64_be((i + 1) as u64)),
                    value: U256::from(1000000000000000000u64),
                    gas: U256::from(200000),
                    gas_price: Some(U256::from(20000000000u64)),
                    input: Bytes::from(vec![0u8; 100]),
                    nonce: U256::from(i),
                    ..Default::default()
                },
                signature: vec![0u8; 65],
            };
            
            Bundle {
                id: format!("test_bundle_{}", i),
                transactions: vec![tx],
                target_block: 1000000 + i as u64,
                max_gas_price: U256::from(50000000000u64),
                profit_estimate: U256::from(100000000000000000u64), // 0.1 ETH
            }
        })
        .collect()
}

/// Generate test opportunities for benchmarking
fn generate_test_opportunities(count: usize) -> Vec<Opportunity> {
    use ethers::types::{H256, Address, U256, Bytes};
    use mev_core::types::ParsedTransaction;
    use chrono::Utc;
    
    (0..count)
        .map(|i| {
            let target_tx = ParsedTransaction {
                hash: H256::from_low_u64_be(i as u64),
                from: Address::from_low_u64_be(i as u64),
                to: Some(Address::from_low_u64_be((i + 1) as u64)),
                value: U256::from(10000000000000000000u64), // 10 ETH
                gas_limit: U256::from(200000),
                gas_price: U256::from(20000000000u64),
                nonce: U256::from(i),
                input: Bytes::from(vec![0u8; 100]),
                target_type: None,
                function_signature: Some("swapExactTokensForTokens".to_string()),
                decoded_params: None,
                timestamp: Utc::now(),
            };
            
            Opportunity {
                id: format!("test_opportunity_{}", i),
                strategy_name: "backrun".to_string(),
                target_transaction: target_tx,
                estimated_profit_wei: 100000000000000000u64, // 0.1 ETH
                gas_estimate: 200000,
                confidence_score: 0.85,
                expiry_time: Utc::now() + chrono::Duration::minutes(5),
                bundle_transactions: vec![],
            }
        })
        .collect()
}

criterion_group!(
    benches,
    bench_simulation_throughput,
    bench_decision_loop_latency,
    bench_gas_estimation,
    bench_profit_calculation,
    bench_concurrent_simulations
);
criterion_main!(benches);