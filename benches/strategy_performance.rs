//! Benchmark tests for strategy performance
//! 
//! These benchmarks validate that the strategy evaluation system
//! meets performance requirements for real-time MEV detection.

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use std::time::Duration;
use tokio::runtime::Runtime;

use mev_strategies::{
    StrategyEngine,
    BackrunStrategy,
    SandwichStrategy,
    strategy_types::StrategyConfig,
};
use mev_core::types::ParsedTransaction;

/// Benchmark strategy evaluation performance
fn bench_strategy_evaluation(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    // Create strategy engine with all strategies
    let mut strategy_engine = rt.block_on(async {
        let mut engine = StrategyEngine::new();
        
        let backrun_config = StrategyConfig::default();
        let sandwich_config = StrategyConfig::default();
        
        engine.register_strategy(Box::new(BackrunStrategy::new(backrun_config)))
            .await
            .expect("Failed to register backrun strategy");
        
        engine.register_strategy(Box::new(SandwichStrategy::new(sandwich_config)))
            .await
            .expect("Failed to register sandwich strategy");
        
        engine
    });
    
    // Generate test transactions of different types
    let swap_transactions = generate_swap_transactions(1000);
    let transfer_transactions = generate_transfer_transactions(1000);
    let complex_transactions = generate_complex_transactions(1000);
    
    let mut group = c.benchmark_group("strategy_evaluation");
    group.measurement_time(Duration::from_secs(15));
    
    // Benchmark different transaction types
    group.bench_function("evaluate_swap_transactions", |b| {
        b.to_async(&rt).iter(|| async {
            for tx in &swap_transactions[..100] {
                let _opportunities = black_box(
                    strategy_engine.evaluate_transaction(tx).await
                );
            }
        });
    });
    
    group.bench_function("evaluate_transfer_transactions", |b| {
        b.to_async(&rt).iter(|| async {
            for tx in &transfer_transactions[..100] {
                let _opportunities = black_box(
                    strategy_engine.evaluate_transaction(tx).await
                );
            }
        });
    });
    
    group.bench_function("evaluate_complex_transactions", |b| {
        b.to_async(&rt).iter(|| async {
            for tx in &complex_transactions[..100] {
                let _opportunities = black_box(
                    strategy_engine.evaluate_transaction(tx).await
                );
            }
        });
    });
    
    group.finish();
}

/// Benchmark individual strategy performance
fn bench_individual_strategies(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let backrun_strategy = BackrunStrategy::new(StrategyConfig::default());
    let sandwich_strategy = SandwichStrategy::new(StrategyConfig::default());
    
    let test_transactions = generate_swap_transactions(100);
    
    let mut group = c.benchmark_group("individual_strategies");
    
    group.bench_function("backrun_strategy", |b| {
        b.to_async(&rt).iter(|| async {
            for tx in &test_transactions {
                let _opportunity = black_box(
                    backrun_strategy.evaluate(tx).await
                );
            }
        });
    });
    
    group.bench_function("sandwich_strategy", |b| {
        b.to_async(&rt).iter(|| async {
            for tx in &test_transactions {
                let _opportunity = black_box(
                    sandwich_strategy.evaluate(tx).await
                );
            }
        });
    });
    
    group.finish();
}

/// Benchmark strategy evaluation under different loads
fn bench_strategy_load_testing(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut strategy_engine = rt.block_on(async {
        let mut engine = StrategyEngine::new();
        engine.register_strategy(Box::new(BackrunStrategy::new(StrategyConfig::default())))
            .await
            .expect("Failed to register strategy");
        engine
    });
    
    let test_transactions = generate_mixed_transactions(2000);
    
    let mut group = c.benchmark_group("strategy_load_testing");
    group.measurement_time(Duration::from_secs(20));
    
    for load in [10, 50, 100, 500, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::new("evaluate_transactions", load),
            load,
            |b, &load| {
                let txs = &test_transactions[..load];
                b.to_async(&rt).iter(|| async {
                    for tx in txs {
                        let _opportunities = black_box(
                            strategy_engine.evaluate_transaction(tx).await
                        );
                    }
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark concurrent strategy evaluation
fn bench_concurrent_strategy_evaluation(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let strategy_engine = rt.block_on(async {
        let mut engine = StrategyEngine::new();
        engine.register_strategy(Box::new(BackrunStrategy::new(StrategyConfig::default())))
            .await
            .expect("Failed to register strategy");
        std::sync::Arc::new(engine)
    });
    
    let test_transactions = generate_mixed_transactions(500);
    
    let mut group = c.benchmark_group("concurrent_strategy_evaluation");
    group.measurement_time(Duration::from_secs(20));
    
    for concurrency in [1, 5, 10, 20].iter() {
        group.bench_with_input(
            BenchmarkId::new("concurrent_evaluation", concurrency),
            concurrency,
            |b, &concurrency| {
                b.to_async(&rt).iter(|| async {
                    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
                    let mut handles = Vec::new();
                    
                    for tx in &test_transactions[..200] {
                        let permit = semaphore.clone().acquire_owned().await.unwrap();
                        let engine = strategy_engine.clone();
                        let tx = tx.clone();
                        
                        handles.push(tokio::spawn(async move {
                            let _permit = permit;
                            engine.evaluate_transaction(&tx).await
                        }));
                    }
                    
                    // Wait for all evaluations
                    for handle in handles {
                        let _ = black_box(handle.await);
                    }
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark strategy configuration loading and validation
fn bench_strategy_configuration(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("strategy_configuration");
    
    group.bench_function("load_strategy_config", |b| {
        b.to_async(&rt).iter(|| async {
            for _ in 0..100 {
                let _config = black_box(
                    StrategyConfig::from_yaml_str(SAMPLE_CONFIG).unwrap()
                );
            }
        });
    });
    
    group.bench_function("validate_strategy_config", |b| {
        b.to_async(&rt).iter(|| async {
            let config = StrategyConfig::from_yaml_str(SAMPLE_CONFIG).unwrap();
            for _ in 0..100 {
                let _is_valid = black_box(config.validate());
            }
        });
    });
    
    group.finish();
}

/// Generate swap transactions for testing
fn generate_swap_transactions(count: usize) -> Vec<ParsedTransaction> {
    use ethers::types::{H256, Address, U256, Bytes};
    use chrono::Utc;
    
    (0..count)
        .map(|i| ParsedTransaction {
            hash: H256::from_low_u64_be(i as u64),
            from: Address::from_low_u64_be(i as u64),
            to: Some(Address::from_low_u64_be(0x1234)), // Uniswap router
            value: U256::zero(),
            gas_limit: U256::from(200000),
            gas_price: U256::from(20000000000u64),
            nonce: U256::from(i),
            input: Bytes::from(hex::decode("38ed1739").unwrap()), // swapExactTokensForTokens
            target_type: Some(mev_core::types::TargetType::UniswapV2),
            function_signature: Some("swapExactTokensForTokens".to_string()),
            decoded_params: None,
            timestamp: Utc::now(),
        })
        .collect()
}

/// Generate transfer transactions for testing
fn generate_transfer_transactions(count: usize) -> Vec<ParsedTransaction> {
    use ethers::types::{H256, Address, U256, Bytes};
    use chrono::Utc;
    
    (0..count)
        .map(|i| ParsedTransaction {
            hash: H256::from_low_u64_be(i as u64),
            from: Address::from_low_u64_be(i as u64),
            to: Some(Address::from_low_u64_be(0x5678)), // ERC20 token
            value: U256::zero(),
            gas_limit: U256::from(50000),
            gas_price: U256::from(20000000000u64),
            nonce: U256::from(i),
            input: Bytes::from(hex::decode("a9059cbb").unwrap()), // transfer
            target_type: Some(mev_core::types::TargetType::ERC20Transfer),
            function_signature: Some("transfer".to_string()),
            decoded_params: None,
            timestamp: Utc::now(),
        })
        .collect()
}

/// Generate complex transactions for testing
fn generate_complex_transactions(count: usize) -> Vec<ParsedTransaction> {
    use ethers::types::{H256, Address, U256, Bytes};
    use chrono::Utc;
    
    (0..count)
        .map(|i| ParsedTransaction {
            hash: H256::from_low_u64_be(i as u64),
            from: Address::from_low_u64_be(i as u64),
            to: Some(Address::from_low_u64_be(0x9abc)), // Complex contract
            value: U256::from(1000000000000000000u64),
            gas_limit: U256::from(500000),
            gas_price: U256::from(50000000000u64),
            nonce: U256::from(i),
            input: Bytes::from(vec![0u8; 1000]), // Complex calldata
            target_type: None,
            function_signature: Some("complexFunction".to_string()),
            decoded_params: None,
            timestamp: Utc::now(),
        })
        .collect()
}

/// Generate mixed transaction types for testing
fn generate_mixed_transactions(count: usize) -> Vec<ParsedTransaction> {
    let mut transactions = Vec::new();
    
    let swap_count = count / 3;
    let transfer_count = count / 3;
    let complex_count = count - swap_count - transfer_count;
    
    transactions.extend(generate_swap_transactions(swap_count));
    transactions.extend(generate_transfer_transactions(transfer_count));
    transactions.extend(generate_complex_transactions(complex_count));
    
    transactions
}

const SAMPLE_CONFIG: &str = r#"
strategies:
  backrun:
    enabled: true
    min_profit_wei: 1000000000000000
    max_gas_price: 100000000000
    slippage_tolerance: 0.01
  sandwich:
    enabled: false
    min_profit_wei: 5000000000000000
    max_gas_price: 200000000000
    slippage_tolerance: 0.005
"#;

criterion_group!(
    benches,
    bench_strategy_evaluation,
    bench_individual_strategies,
    bench_strategy_load_testing,
    bench_concurrent_strategy_evaluation,
    bench_strategy_configuration
);
criterion_main!(benches);