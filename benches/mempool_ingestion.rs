//! Benchmark tests for mempool ingestion performance
//! 
//! These benchmarks validate that the mempool ingestion system meets
//! the performance requirements specified in the design document.

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use std::time::Duration;
use tokio::runtime::Runtime;

use mev_mempool::{
    parser::TransactionParser,
    ring_buffer::RingBuffer,
    abi_decoder::ABIDecoder,
    filters::TransactionFilter,
};
use mev_core::types::ParsedTransaction;

/// Benchmark transaction parsing performance
fn bench_transaction_parsing(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    // Create test transaction data
    let test_transactions = generate_test_transactions(1000);
    let parser = rt.block_on(async {
        TransactionParser::new().await.expect("Failed to create parser")
    });
    
    let mut group = c.benchmark_group("transaction_parsing");
    group.measurement_time(Duration::from_secs(10));
    
    for size in [10, 100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::new("parse_transactions", size),
            size,
            |b, &size| {
                let txs = &test_transactions[..size];
                b.to_async(&rt).iter(|| async {
                    for tx in txs {
                        let _ = black_box(parser.parse_transaction(tx).await);
                    }
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark ring buffer performance under high load
fn bench_ring_buffer_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("ring_buffer");
    group.measurement_time(Duration::from_secs(10));
    
    for capacity in [1000, 10000, 100000].iter() {
        group.bench_with_input(
            BenchmarkId::new("producer_consumer", capacity),
            capacity,
            |b, &capacity| {
                b.to_async(&rt).iter(|| async {
                    let buffer = RingBuffer::new(capacity);
                    let test_data = generate_test_transactions(capacity / 10);
                    
                    // Simulate producer
                    let producer_handle = tokio::spawn({
                        let buffer = buffer.clone();
                        let data = test_data.clone();
                        async move {
                            for tx in data {
                                let _ = buffer.try_push(tx).await;
                            }
                        }
                    });
                    
                    // Simulate consumer
                    let consumer_handle = tokio::spawn({
                        let buffer = buffer.clone();
                        async move {
                            let mut count = 0;
                            while count < test_data.len() {
                                if let Ok(_) = buffer.try_pop().await {
                                    count += 1;
                                }
                                tokio::task::yield_now().await;
                            }
                        }
                    });
                    
                    let _ = tokio::join!(producer_handle, consumer_handle);
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmark ABI decoding performance
fn bench_abi_decoding(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let decoder = rt.block_on(async {
        ABIDecoder::new().await.expect("Failed to create ABI decoder")
    });
    
    // Generate test calldata for different function types
    let swap_calldata = generate_swap_calldata();
    let transfer_calldata = generate_transfer_calldata();
    let complex_calldata = generate_complex_calldata();
    
    let mut group = c.benchmark_group("abi_decoding");
    
    group.bench_function("decode_swap", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = black_box(decoder.decode_function_call(&swap_calldata).await);
        });
    });
    
    group.bench_function("decode_transfer", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = black_box(decoder.decode_function_call(&transfer_calldata).await);
        });
    });
    
    group.bench_function("decode_complex", |b| {
        b.to_async(&rt).iter(|| async {
            let _ = black_box(decoder.decode_function_call(&complex_calldata).await);
        });
    });
    
    group.finish();
}

/// Benchmark transaction filtering performance
fn bench_transaction_filtering(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let filter = rt.block_on(async {
        TransactionFilter::new().await.expect("Failed to create filter")
    });
    
    let test_transactions = generate_test_transactions(1000);
    
    let mut group = c.benchmark_group("transaction_filtering");
    
    for batch_size in [10, 100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::new("filter_batch", batch_size),
            batch_size,
            |b, &batch_size| {
                let txs = &test_transactions[..batch_size];
                b.to_async(&rt).iter(|| async {
                    for tx in txs {
                        let _ = black_box(filter.should_process(tx).await);
                    }
                });
            },
        );
    }
    
    group.finish();
}

/// Generate test transaction data for benchmarking
fn generate_test_transactions(count: usize) -> Vec<ParsedTransaction> {
    use ethers::types::{H256, Address, U256, Bytes};
    use chrono::Utc;
    
    (0..count)
        .map(|i| ParsedTransaction {
            hash: H256::from_low_u64_be(i as u64),
            from: Address::from_low_u64_be(i as u64),
            to: Some(Address::from_low_u64_be((i + 1) as u64)),
            value: U256::from(1000000000000000000u64), // 1 ETH
            gas_limit: U256::from(21000),
            gas_price: U256::from(20000000000u64), // 20 gwei
            nonce: U256::from(i),
            input: Bytes::from(vec![0u8; 100]),
            target_type: None,
            function_signature: None,
            decoded_params: None,
            timestamp: Utc::now(),
        })
        .collect()
}

/// Generate test swap calldata
fn generate_swap_calldata() -> Vec<u8> {
    // Simplified swap function signature and data
    hex::decode("38ed173900000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000080").unwrap()
}

/// Generate test transfer calldata
fn generate_transfer_calldata() -> Vec<u8> {
    // ERC20 transfer function signature and data
    hex::decode("a9059cbb000000000000000000000000742d35cc6634c0532925a3b8d0c9e3e4c5c5e0000000000000000000000000000000000000000000000000000de0b6b3a7640000").unwrap()
}

/// Generate complex calldata for testing
fn generate_complex_calldata() -> Vec<u8> {
    // Complex multi-call function with nested data
    vec![0u8; 1000] // Placeholder for complex calldata
}

criterion_group!(
    benches,
    bench_transaction_parsing,
    bench_ring_buffer_throughput,
    bench_abi_decoding,
    bench_transaction_filtering
);
criterion_main!(benches);