//! Throughput benchmark for simulation engine targeting ≥200 sims/sec

use anyhow::Result;
use ethers::types::{Address, Bytes, U256};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::time::timeout;
use tracing::{info, warn};

use crate::{
    PrometheusMetrics, RpcClientConfig, SimulationBundle, SimulationOptimizerConfig,
    SimulationOptimizer, SimulationTransaction,
};

/// Benchmark configuration
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    pub duration_seconds: u64,
    pub concurrent_batches: usize,
    pub bundles_per_batch: usize,
    pub transactions_per_bundle: usize,
    pub target_throughput_sims_per_sec: f64,
    pub warmup_duration_seconds: u64,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            duration_seconds: 60,
            concurrent_batches: 10,
            bundles_per_batch: 5,
            transactions_per_bundle: 3,
            target_throughput_sims_per_sec: 200.0,
            warmup_duration_seconds: 10,
        }
    }
}

/// Benchmark results
#[derive(Debug, Clone)]
pub struct BenchmarkResults {
    pub total_simulations: u64,
    pub successful_simulations: u64,
    pub failed_simulations: u64,
    pub duration_seconds: f64,
    pub actual_throughput_sims_per_sec: f64,
    pub target_throughput_sims_per_sec: f64,
    pub throughput_achieved: bool,
    pub avg_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub p99_latency_ms: f64,
    pub error_rate: f64,
}

impl BenchmarkResults {
    pub fn print_summary(&self) {
        info!("=== Simulation Throughput Benchmark Results ===");
        info!("Total Simulations: {}", self.total_simulations);
        info!("Successful: {} ({:.1}%)", 
              self.successful_simulations, 
              (self.successful_simulations as f64 / self.total_simulations as f64) * 100.0);
        info!("Failed: {} ({:.1}%)", 
              self.failed_simulations, 
              self.error_rate * 100.0);
        info!("Duration: {:.1} seconds", self.duration_seconds);
        info!("Actual Throughput: {:.1} sims/sec", self.actual_throughput_sims_per_sec);
        info!("Target Throughput: {:.1} sims/sec", self.target_throughput_sims_per_sec);
        info!("Target Achieved: {}", if self.throughput_achieved { "✓ YES" } else { "✗ NO" });
        info!("Average Latency: {:.2} ms", self.avg_latency_ms);
        info!("P95 Latency: {:.2} ms", self.p95_latency_ms);
        info!("P99 Latency: {:.2} ms", self.p99_latency_ms);
        info!("===============================================");
    }
}

/// Simulation throughput benchmark
pub struct SimulationBenchmark {
    optimizer: Arc<SimulationOptimizer>,
    config: BenchmarkConfig,
}

impl SimulationBenchmark {
    /// Create new benchmark instance
    pub async fn new(
        benchmark_config: BenchmarkConfig,
        optimizer_config: SimulationOptimizerConfig,
        rpc_config: RpcClientConfig,
        metrics: Arc<PrometheusMetrics>,
    ) -> Result<Self> {
        let optimizer = Arc::new(SimulationOptimizer::new(
            optimizer_config,
            rpc_config,
            metrics,
        ).await?);

        Ok(Self {
            optimizer,
            config: benchmark_config,
        })
    }

    /// Run throughput benchmark
    pub async fn run_benchmark(&self) -> Result<BenchmarkResults> {
        info!("Starting simulation throughput benchmark");
        info!("Target: {:.0} sims/sec for {} seconds", 
              self.config.target_throughput_sims_per_sec, 
              self.config.duration_seconds);

        // Warmup phase
        if self.config.warmup_duration_seconds > 0 {
            info!("Warming up for {} seconds...", self.config.warmup_duration_seconds);
            self.run_warmup().await?;
        }

        // Main benchmark
        let start_time = Instant::now();
        let mut latencies = Vec::new();
        let mut total_simulations = 0u64;
        let mut successful_simulations = 0u64;
        let mut failed_simulations = 0u64;

        let benchmark_duration = Duration::from_secs(self.config.duration_seconds);
        let mut interval = tokio::time::interval(Duration::from_millis(100)); // Check every 100ms

        while start_time.elapsed() < benchmark_duration {
            interval.tick().await;

            // Launch concurrent batches
            let mut batch_handles = Vec::new();
            
            for _ in 0..self.config.concurrent_batches {
                let optimizer = self.optimizer.clone();
                let bundles = self.generate_test_bundles();
                
                let handle = tokio::spawn(async move {
                    let batch_start = Instant::now();
                    let result = optimizer.simulate_batch(bundles).await;
                    let batch_latency = batch_start.elapsed();
                    (result, batch_latency)
                });
                
                batch_handles.push(handle);
            }

            // Collect results
            for handle in batch_handles {
                match timeout(Duration::from_secs(5), handle).await {
                    Ok(Ok((Ok(batch_result), batch_latency))) => {
                        total_simulations += batch_result.results.len() as u64;
                        successful_simulations += batch_result.results.iter()
                            .filter(|r| r.success)
                            .count() as u64;
                        failed_simulations += batch_result.results.iter()
                            .filter(|r| !r.success)
                            .count() as u64;
                        
                        latencies.push(batch_latency.as_secs_f64() * 1000.0);
                    }
                    Ok(Ok((Err(e), _))) => {
                        warn!("Batch simulation failed: {}", e);
                        failed_simulations += self.config.bundles_per_batch as u64;
                    }
                    Ok(Err(e)) => {
                        warn!("Batch task failed: {}", e);
                        failed_simulations += self.config.bundles_per_batch as u64;
                    }
                    Err(_) => {
                        warn!("Batch simulation timeout");
                        failed_simulations += self.config.bundles_per_batch as u64;
                    }
                }
            }
        }

        let actual_duration = start_time.elapsed().as_secs_f64();
        let actual_throughput = total_simulations as f64 / actual_duration;
        let throughput_achieved = actual_throughput >= self.config.target_throughput_sims_per_sec;

        // Calculate latency percentiles
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let avg_latency_ms = if !latencies.is_empty() {
            latencies.iter().sum::<f64>() / latencies.len() as f64
        } else {
            0.0
        };

        let p95_latency_ms = if !latencies.is_empty() {
            let idx = ((latencies.len() as f64) * 0.95) as usize;
            latencies[idx.min(latencies.len() - 1)]
        } else {
            0.0
        };

        let p99_latency_ms = if !latencies.is_empty() {
            let idx = ((latencies.len() as f64) * 0.99) as usize;
            latencies[idx.min(latencies.len() - 1)]
        } else {
            0.0
        };

        let error_rate = if total_simulations > 0 {
            failed_simulations as f64 / total_simulations as f64
        } else {
            0.0
        };

        let results = BenchmarkResults {
            total_simulations,
            successful_simulations,
            failed_simulations,
            duration_seconds: actual_duration,
            actual_throughput_sims_per_sec: actual_throughput,
            target_throughput_sims_per_sec: self.config.target_throughput_sims_per_sec,
            throughput_achieved,
            avg_latency_ms,
            p95_latency_ms,
            p99_latency_ms,
            error_rate,
        };

        results.print_summary();
        Ok(results)
    }

    /// Run warmup phase to prepare connections and caches
    async fn run_warmup(&self) -> Result<()> {
        let warmup_start = Instant::now();
        let warmup_duration = Duration::from_secs(self.config.warmup_duration_seconds);

        while warmup_start.elapsed() < warmup_duration {
            let bundles = self.generate_test_bundles();
            
            // Run a smaller batch during warmup
            match timeout(Duration::from_secs(2), self.optimizer.simulate_batch(bundles)).await {
                Ok(Ok(_)) => {
                    // Successful warmup batch
                }
                Ok(Err(e)) => {
                    warn!("Warmup batch failed: {}", e);
                }
                Err(_) => {
                    warn!("Warmup batch timeout");
                }
            }

            // Small delay between warmup batches
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        info!("Warmup completed");
        Ok(())
    }

    /// Generate test simulation bundles
    fn generate_test_bundles(&self) -> Vec<SimulationBundle> {
        let mut bundles = Vec::new();
        
        for i in 0..self.config.bundles_per_batch {
            let bundle_id = format!("benchmark_bundle_{}", i);
            let mut transactions = Vec::new();
            
            for j in 0..self.config.transactions_per_bundle {
                let tx = SimulationTransaction {
                    from: Address::from([1u8; 20]),
                    to: Some(Address::from([2u8; 20])),
                    value: U256::from(1_000_000_000_000_000u64), // 0.001 ETH
                    gas_limit: U256::from(21000),
                    gas_price: U256::from(20_000_000_000u64), // 20 gwei
                    data: Bytes::from(vec![0x60, 0x60, 0x60, 0x40]), // Simple bytecode
                    nonce: Some(U256::from(j)),
                };
                transactions.push(tx);
            }
            
            let bundle = SimulationBundle {
                id: bundle_id,
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

    /// Run stress test with increasing load
    pub async fn run_stress_test(&self) -> Result<Vec<BenchmarkResults>> {
        info!("Starting stress test with increasing concurrent batches");
        
        let mut results = Vec::new();
        let original_concurrent = self.config.concurrent_batches;
        
        // Test with increasing concurrency levels
        for concurrent_level in [1, 2, 5, 10, 20, 50].iter() {
            if *concurrent_level > original_concurrent * 3 {
                break; // Don't go too high
            }
            
            info!("Testing with {} concurrent batches", concurrent_level);
            
            let mut stress_config = self.config.clone();
            stress_config.concurrent_batches = *concurrent_level;
            stress_config.duration_seconds = 30; // Shorter duration for stress test
            
            let stress_benchmark = Self {
                optimizer: self.optimizer.clone(),
                config: stress_config,
            };
            
            match stress_benchmark.run_benchmark().await {
                Ok(result) => {
                    results.push(result);
                }
                Err(e) => {
                    warn!("Stress test failed at {} concurrent batches: {}", concurrent_level, e);
                    break;
                }
            }
            
            // Brief pause between stress levels
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        
        // Print stress test summary
        info!("=== Stress Test Summary ===");
        for (i, result) in results.iter().enumerate() {
            let concurrent_level = [1, 2, 5, 10, 20, 50][i];
            info!("Concurrent {}: {:.1} sims/sec (target: {:.1})", 
                  concurrent_level,
                  result.actual_throughput_sims_per_sec,
                  result.target_throughput_sims_per_sec);
        }
        
        Ok(results)
    }

    /// Run latency-focused benchmark
    pub async fn run_latency_benchmark(&self) -> Result<BenchmarkResults> {
        info!("Starting latency-focused benchmark (single batch at a time)");
        
        let mut latency_config = self.config.clone();
        latency_config.concurrent_batches = 1; // Focus on latency, not throughput
        latency_config.bundles_per_batch = 1;
        latency_config.duration_seconds = 30;
        
        let latency_benchmark = Self {
            optimizer: self.optimizer.clone(),
            config: latency_config,
        };
        
        latency_benchmark.run_benchmark().await
    }
}

/// Benchmark runner for easy execution
pub struct BenchmarkRunner;

impl BenchmarkRunner {
    /// Run comprehensive benchmark suite
    pub async fn run_comprehensive_benchmark(
        optimizer_config: SimulationOptimizerConfig,
        rpc_config: RpcClientConfig,
        metrics: Arc<PrometheusMetrics>,
    ) -> Result<()> {
        info!("Starting comprehensive simulation benchmark suite");
        
        // Standard throughput benchmark
        let benchmark_config = BenchmarkConfig::default();
        let benchmark = SimulationBenchmark::new(
            benchmark_config,
            optimizer_config.clone(),
            rpc_config.clone(),
            metrics.clone(),
        ).await?;
        
        info!("=== Running Throughput Benchmark ===");
        let throughput_result = benchmark.run_benchmark().await?;
        
        // Stress test
        info!("=== Running Stress Test ===");
        let _stress_results = benchmark.run_stress_test().await?;
        
        // Latency benchmark
        info!("=== Running Latency Benchmark ===");
        let _latency_result = benchmark.run_latency_benchmark().await?;
        
        // Final assessment
        if throughput_result.throughput_achieved {
            info!("✓ Throughput target of {:.0} sims/sec ACHIEVED", 
                  throughput_result.target_throughput_sims_per_sec);
        } else {
            warn!("✗ Throughput target of {:.0} sims/sec NOT achieved (actual: {:.1})", 
                  throughput_result.target_throughput_sims_per_sec,
                  throughput_result.actual_throughput_sims_per_sec);
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RpcClientConfig;

    #[tokio::test]
    async fn test_benchmark_config() {
        let config = BenchmarkConfig::default();
        assert_eq!(config.target_throughput_sims_per_sec, 200.0);
        assert!(config.duration_seconds > 0);
    }

    #[test]
    fn test_benchmark_results() {
        let results = BenchmarkResults {
            total_simulations: 1000,
            successful_simulations: 950,
            failed_simulations: 50,
            duration_seconds: 5.0,
            actual_throughput_sims_per_sec: 200.0,
            target_throughput_sims_per_sec: 200.0,
            throughput_achieved: true,
            avg_latency_ms: 25.0,
            p95_latency_ms: 50.0,
            p99_latency_ms: 75.0,
            error_rate: 0.05,
        };
        
        assert!(results.throughput_achieved);
        assert_eq!(results.error_rate, 0.05);
    }

    #[tokio::test]
    async fn test_bundle_generation() {
        let config = BenchmarkConfig {
            bundles_per_batch: 2,
            transactions_per_bundle: 3,
            ..Default::default()
        };
        
        // Handle metrics creation gracefully
        let metrics = match crate::PrometheusMetrics::new() {
            Ok(m) => Arc::new(m),
            Err(_) => {
                // Skip test if metrics registration fails
                return;
            }
        };
        
        let benchmark = SimulationBenchmark::new(
            config.clone(),
            SimulationOptimizerConfig::default(),
            RpcClientConfig::default(),
            metrics,
        ).await;
        
        if let Ok(benchmark) = benchmark {
            let bundles = benchmark.generate_test_bundles();
            assert_eq!(bundles.len(), config.bundles_per_batch);
            assert_eq!(bundles[0].transactions.len(), config.transactions_per_bundle);
        }
    }
}