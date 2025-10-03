//! Performance testing utility for MEV bot
//! 
//! This tool provides comprehensive performance testing including synthetic load
//! generation to simulate 10k txs/sec locally and validate system performance.

use anyhow::Result;
use mev_core::{
    CorePinningConfig, CpuAffinityManager, CpuPerformanceMonitor, LatencyHistogram,
    PerformanceTimer, PrometheusMetrics,
};
use mev_mempool::{BackpressureBuffer, BackpressureConfig, DropPolicy, MempoolService};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    fs::File,
    io::AsyncWriteExt,
    signal,
    sync::mpsc,
    time::{interval, sleep},
};
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceTestResult {
    pub test_name: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub duration_seconds: f64,
    pub target_tps: u64,
    pub actual_tps: f64,
    pub total_transactions: u64,
    pub successful_transactions: u64,
    pub dropped_transactions: u64,
    pub latency_stats: LatencyTestStats,
    pub cpu_stats: CpuTestStats,
    pub memory_stats: MemoryTestStats,
    pub performance_grade: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyTestStats {
    pub median_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub mean_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuTestStats {
    pub usage_percent: f64,
    pub cores_used: usize,
    pub pinning_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryTestStats {
    pub peak_usage_mb: f64,
    pub buffer_utilization_percent: f64,
}

/// Synthetic load generator for performance testing
pub struct SyntheticLoadGenerator {
    target_tps: u64,
    duration_seconds: u64,
    transaction_counter: Arc<AtomicU64>,
    latency_histogram: Arc<LatencyHistogram>,
    start_time: Instant,
}

impl SyntheticLoadGenerator {
    pub fn new(target_tps: u64, duration_seconds: u64) -> Self {
        Self {
            target_tps,
            duration_seconds,
            transaction_counter: Arc::new(AtomicU64::new(0)),
            latency_histogram: Arc::new(LatencyHistogram::new(100000)), // Keep 100k samples
            start_time: Instant::now(),
        }
    }

    /// Generate synthetic transaction load
    pub async fn generate_load<F>(&self, mut processor: F) -> Result<()>
    where
        F: FnMut(SyntheticTransaction) -> Result<()> + Send,
    {
        info!(
            target_tps = self.target_tps,
            duration_seconds = self.duration_seconds,
            "Starting synthetic load generation"
        );

        let interval_ns = 1_000_000_000 / self.target_tps;
        let mut next_send_time = Instant::now();
        let end_time = self.start_time + Duration::from_secs(self.duration_seconds);

        let mut transaction_id = 0u64;

        while Instant::now() < end_time {
            let processing_timer = PerformanceTimer::new("synthetic_tx_processing".to_string());
            
            // Create synthetic transaction
            let tx = self.create_synthetic_transaction(transaction_id);
            
            // Process transaction
            if let Err(e) = processor(tx) {
                warn!("Failed to process synthetic transaction {}: {}", transaction_id, e);
            } else {
                let processing_time_ms = processing_timer.elapsed_ms();
                self.latency_histogram.record(processing_time_ms);
                self.transaction_counter.fetch_add(1, Ordering::Relaxed);
            }

            transaction_id += 1;

            // Wait for next send time
            next_send_time += Duration::from_nanos(interval_ns);
            let now = Instant::now();
            if next_send_time > now {
                sleep(next_send_time - now).await;
            }

            // Print progress every 1000 transactions
            if transaction_id % 1000 == 0 {
                let elapsed = self.start_time.elapsed();
                let current_tps = self.transaction_counter.load(Ordering::Relaxed) as f64 
                    / elapsed.as_secs_f64();
                
                info!(
                    "Generated {} transactions, Current TPS: {:.2}, Target: {}",
                    transaction_id, current_tps, self.target_tps
                );
            }
        }

        let final_count = self.transaction_counter.load(Ordering::Relaxed);
        let final_tps = final_count as f64 / self.duration_seconds as f64;
        
        info!(
            "Load generation completed - Total: {}, Actual TPS: {:.2}",
            final_count, final_tps
        );

        Ok(())
    }

    fn create_synthetic_transaction(&self, id: u64) -> SyntheticTransaction {
        use rand::Rng;
        let mut rng = rand::thread_rng();

        // Create different types of transactions to simulate real mempool diversity
        let tx_type = match id % 10 {
            0..=3 => SyntheticTxType::Transfer,
            4..=6 => SyntheticTxType::UniswapSwap,
            7..=8 => SyntheticTxType::ERC20Transfer,
            _ => SyntheticTxType::ContractCall,
        };

        SyntheticTransaction {
            id,
            tx_type,
            from: format!("0x{:040x}", rng.gen::<u64>()),
            to: format!("0x{:040x}", rng.gen::<u64>()),
            value: rng.gen_range(0..1000000000000000000u64),
            gas_price: rng.gen_range(1000000000..100000000000u64), // 1-100 gwei
            gas_limit: rng.gen_range(21000..500000u64),
            nonce: rng.gen_range(0..1000u64),
            data_size: rng.gen_range(0..1024),
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn get_stats(&self) -> (u64, f64, mev_core::LatencyStats) {
        let total = self.transaction_counter.load(Ordering::Relaxed);
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let tps = total as f64 / elapsed;
        let latency_stats = self.latency_histogram.get_stats();
        
        (total, tps, latency_stats)
    }
}

#[derive(Debug, Clone)]
pub struct SyntheticTransaction {
    pub id: u64,
    pub tx_type: SyntheticTxType,
    pub from: String,
    pub to: String,
    pub value: u64,
    pub gas_price: u64,
    pub gas_limit: u64,
    pub nonce: u64,
    pub data_size: usize,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub enum SyntheticTxType {
    Transfer,
    ERC20Transfer,
    UniswapSwap,
    ContractCall,
}

/// Comprehensive performance test suite
pub struct PerformanceTestSuite {
    cpu_manager: CpuAffinityManager,
    cpu_monitor: CpuPerformanceMonitor,
    metrics: Arc<PrometheusMetrics>,
}

impl PerformanceTestSuite {
    pub fn new(cpu_config: CorePinningConfig) -> Result<Self> {
        let cpu_manager = CpuAffinityManager::new(cpu_config);
        let cpu_monitor = CpuPerformanceMonitor::new();
        let metrics = Arc::new(PrometheusMetrics::new()?);

        Ok(Self {
            cpu_manager,
            cpu_monitor,
            metrics,
        })
    }

    /// Run comprehensive performance test
    pub async fn run_comprehensive_test(&mut self) -> Result<Vec<PerformanceTestResult>> {
        info!("Starting comprehensive performance test suite");

        let mut results = Vec::new();

        // Test 1: Baseline performance (1k TPS)
        results.push(self.run_baseline_test().await?);

        // Test 2: Medium load (5k TPS)
        results.push(self.run_medium_load_test().await?);

        // Test 3: High load (10k TPS)
        results.push(self.run_high_load_test().await?);

        // Test 4: Burst load (20k TPS for short duration)
        results.push(self.run_burst_load_test().await?);

        // Test 5: Sustained load (2k TPS for extended period)
        results.push(self.run_sustained_load_test().await?);

        // Generate summary report
        self.generate_summary_report(&results).await?;

        Ok(results)
    }

    async fn run_baseline_test(&mut self) -> Result<PerformanceTestResult> {
        info!("Running baseline performance test (1k TPS)");
        self.run_load_test("baseline", 1000, 60).await
    }

    async fn run_medium_load_test(&mut self) -> Result<PerformanceTestResult> {
        info!("Running medium load test (5k TPS)");
        self.run_load_test("medium_load", 5000, 60).await
    }

    async fn run_high_load_test(&mut self) -> Result<PerformanceTestResult> {
        info!("Running high load test (10k TPS)");
        self.run_load_test("high_load", 10000, 60).await
    }

    async fn run_burst_load_test(&mut self) -> Result<PerformanceTestResult> {
        info!("Running burst load test (20k TPS for 30s)");
        self.run_load_test("burst_load", 20000, 30).await
    }

    async fn run_sustained_load_test(&mut self) -> Result<PerformanceTestResult> {
        info!("Running sustained load test (2k TPS for 300s)");
        self.run_load_test("sustained_load", 2000, 300).await
    }

    async fn run_load_test(
        &mut self,
        test_name: &str,
        target_tps: u64,
        duration_seconds: u64,
    ) -> Result<PerformanceTestResult> {
        let start_time = Instant::now();

        // Initialize CPU affinity
        self.cpu_manager.initialize()?;

        // Create backpressure buffer
        let config = BackpressureConfig {
            enabled: true,
            buffer_size: (target_tps * 2) as usize, // 2 seconds of buffer
            high_watermark: 0.8,
            low_watermark: 0.6,
            drop_policy: DropPolicy::Oldest,
        };

        let buffer = Arc::new(BackpressureBuffer::new(config, self.metrics.clone()));

        // Start consumer tasks
        let num_consumers = std::cmp::min(8, num_cpus::get());
        let mut consumer_handles = Vec::new();
        
        for i in 0..num_consumers {
            let buffer_clone = buffer.clone();
            let handle = tokio::spawn(async move {
                let mut processed = 0u64;
                loop {
                    let batch = buffer_clone.receive_batch(50).await;
                    if batch.is_empty() {
                        sleep(Duration::from_millis(1)).await;
                        continue;
                    }

                    // Simulate realistic processing
                    for tx in &batch {
                        // Simulate ABI decoding and validation
                        let _ = format!("{:?}", tx.target_type);
                        
                        // Simulate some CPU work
                        let _ = (0..100).fold(0u64, |acc, x| acc.wrapping_add(x));
                    }

                    processed += batch.len() as u64;

                    if processed % 1000 == 0 {
                        info!("Consumer {} processed {} transactions", i, processed);
                    }
                }
            });
            consumer_handles.push(handle);
        }

        // Create synthetic load generator
        let load_generator = SyntheticLoadGenerator::new(target_tps, duration_seconds);
        let buffer_clone = buffer.clone();

        // Run load generation
        load_generator
            .generate_load(move |synthetic_tx| {
                // Convert synthetic transaction to parsed transaction
                let parsed_tx = convert_synthetic_to_parsed(synthetic_tx);
                
                // Push to buffer (this will block if needed)
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        buffer_clone.push(parsed_tx).await
                    })
                })?;
                
                Ok(())
            })
            .await?;

        // Stop consumers
        for handle in consumer_handles {
            handle.abort();
        }

        // Collect final statistics
        let (total_generated, actual_tps, latency_stats) = load_generator.get_stats();
        let buffer_stats = buffer.get_stats();
        let cpu_stats = self.cpu_monitor.get_cpu_stats();

        let test_duration = start_time.elapsed().as_secs_f64();

        let result = PerformanceTestResult {
            test_name: test_name.to_string(),
            timestamp: chrono::Utc::now(),
            duration_seconds: test_duration,
            target_tps,
            actual_tps,
            total_transactions: total_generated,
            successful_transactions: buffer_stats.total_processed,
            dropped_transactions: buffer_stats.total_dropped,
            latency_stats: LatencyTestStats {
                median_ms: latency_stats.median_ms,
                p95_ms: latency_stats.p95_ms,
                p99_ms: latency_stats.p99_ms,
                mean_ms: latency_stats.mean_ms,
                min_ms: 0.0, // Would need separate tracking
                max_ms: 0.0, // Would need separate tracking
            },
            cpu_stats: CpuTestStats {
                usage_percent: cpu_stats.cpu_usage_percent,
                cores_used: cpu_stats.num_cores,
                pinning_enabled: !cpu_stats.pinned_cores.is_empty(),
            },
            memory_stats: MemoryTestStats {
                peak_usage_mb: 0.0, // Would need memory tracking
                buffer_utilization_percent: buffer_stats.utilization * 100.0,
            },
            performance_grade: calculate_performance_grade(target_tps, actual_tps, &latency_stats),
        };

        info!(
            test = test_name,
            target_tps = target_tps,
            actual_tps = format!("{:.2}", actual_tps),
            median_latency = format!("{:.2}ms", latency_stats.median_ms),
            drop_rate = format!("{:.2}%", buffer_stats.drop_rate * 100.0),
            grade = result.performance_grade,
            "Performance test completed"
        );

        Ok(result)
    }

    async fn generate_summary_report(&self, results: &[PerformanceTestResult]) -> Result<()> {
        let filename = format!(
            "performance_report_{}.json",
            chrono::Utc::now().format("%Y%m%d_%H%M%S")
        );

        let json_data = serde_json::to_string_pretty(results)?;
        let mut file = File::create(&filename).await?;
        file.write_all(json_data.as_bytes()).await?;

        info!("Performance report saved to: {}", filename);

        // Print summary to console
        println!("\n=== MEV Bot Performance Test Summary ===");
        for result in results {
            println!(
                "{}: {:.0} TPS ({:.1}% of target), {}ms median latency, Grade: {}",
                result.test_name,
                result.actual_tps,
                (result.actual_tps / result.target_tps as f64) * 100.0,
                result.latency_stats.median_ms,
                result.performance_grade
            );
        }

        Ok(())
    }
}

fn convert_synthetic_to_parsed(synthetic_tx: SyntheticTransaction) -> mev_core::ParsedTransaction {
    use mev_core::{DecodedInput, TargetType, Transaction};

    let target_type = match synthetic_tx.tx_type {
        SyntheticTxType::Transfer => TargetType::Unknown,
        SyntheticTxType::ERC20Transfer => TargetType::Unknown,
        SyntheticTxType::UniswapSwap => TargetType::UniswapV2,
        SyntheticTxType::ContractCall => TargetType::Unknown,
    };

    let decoded_input = match synthetic_tx.tx_type {
        SyntheticTxType::UniswapSwap => Some(DecodedInput {
            function_name: "swapExactTokensForTokens".to_string(),
            function_signature: "0x38ed1739".to_string(),
            parameters: vec![],
        }),
        SyntheticTxType::ERC20Transfer => Some(DecodedInput {
            function_name: "transfer".to_string(),
            function_signature: "0xa9059cbb".to_string(),
            parameters: vec![],
        }),
        _ => None,
    };

    mev_core::ParsedTransaction {
        transaction: Transaction {
            hash: format!("0x{:064x}", synthetic_tx.id),
            from: synthetic_tx.from,
            to: Some(synthetic_tx.to),
            value: synthetic_tx.value.to_string(),
            gas_price: synthetic_tx.gas_price.to_string(),
            gas_limit: synthetic_tx.gas_limit.to_string(),
            nonce: synthetic_tx.nonce,
            input: "0x".to_string(),
            timestamp: synthetic_tx.timestamp,
        },
        decoded_input,
        target_type,
        processing_time_ms: 0,
    }
}

fn calculate_performance_grade(target_tps: u64, actual_tps: f64, latency_stats: &mev_core::LatencyStats) -> String {
    let tps_ratio = actual_tps / target_tps as f64;
    let latency_score = if latency_stats.median_ms <= 20.0 { 1.0 } else { 20.0 / latency_stats.median_ms };
    
    let overall_score = (tps_ratio * 0.7) + (latency_score * 0.3);
    
    match overall_score {
        s if s >= 0.9 => "A+".to_string(),
        s if s >= 0.8 => "A".to_string(),
        s if s >= 0.7 => "B+".to_string(),
        s if s >= 0.6 => "B".to_string(),
        s if s >= 0.5 => "C+".to_string(),
        s if s >= 0.4 => "C".to_string(),
        _ => "D".to_string(),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("info,mev_mempool=debug")
        .with_target(false)
        .with_thread_ids(true)
        .init();

    info!("Starting MEV Bot Performance Test Suite");

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let test_type = args.get(1).map(|s| s.as_str()).unwrap_or("comprehensive");

    // Configure CPU pinning
    let cpu_config = CorePinningConfig {
        enabled: std::env::var("ENABLE_CPU_PINNING").unwrap_or_else(|_| "false".to_string()) == "true",
        core_ids: (0..num_cpus::get()).collect(),
        worker_threads: num_cpus::get(),
    };

    let mut test_suite = PerformanceTestSuite::new(cpu_config)?;

    match test_type {
        "comprehensive" => {
            let results = test_suite.run_comprehensive_test().await?;
            info!("Comprehensive test completed with {} results", results.len());
        }
        "baseline" => {
            let result = test_suite.run_baseline_test().await?;
            info!("Baseline test completed: Grade {}", result.performance_grade);
        }
        "high-load" => {
            let result = test_suite.run_high_load_test().await?;
            info!("High load test completed: Grade {}", result.performance_grade);
        }
        _ => {
            println!("Usage: performance-test [comprehensive|baseline|high-load]");
            return Ok(());
        }
    }

    Ok(())
}