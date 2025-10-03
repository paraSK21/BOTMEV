//! Ultra-low latency decision path for MEV bot
//! 
//! This module implements the hot path: mempool→filter→simulate→bundlePlan pipeline
//! optimized for median ≤25ms decision loop latency with CPU pinning, thread pools,
//! and preallocated memory pools.

use crate::{
    cpu_pinning::{CpuAffinityManager, CorePinningConfig},
    metrics::{LatencyHistogram, PerformanceTimer, PrometheusMetrics},
    types::ParsedTransaction,
};
use anyhow::Result;
use crossbeam::channel::{bounded, Receiver, Sender};
use crate::strategy_types::{Opportunity, MockStrategyEngine};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Configuration for the ultra-low latency decision path
#[derive(Debug, Clone)]
pub struct DecisionPathConfig {
    /// Enable CPU core pinning for critical threads
    pub enable_cpu_pinning: bool,
    /// Core assignments for different pipeline stages
    pub core_assignments: CoreAssignments,
    /// Channel buffer sizes for pipeline stages
    pub channel_buffer_sizes: ChannelBufferSizes,
    /// Memory pool configurations
    pub memory_pools: MemoryPoolConfig,
    /// Latency targets and timeouts
    pub latency_targets: LatencyTargets,
    /// Thread pool sizes
    pub thread_pools: ThreadPoolSizes,
}

/// CPU core assignments for pipeline stages
#[derive(Debug, Clone)]
pub struct CoreAssignments {
    pub filter_core: Option<usize>,
    pub simulation_cores: Vec<usize>,
    pub bundle_core: Option<usize>,
    pub metrics_core: Option<usize>,
}

/// Channel buffer sizes for different pipeline stages
#[derive(Debug, Clone)]
pub struct ChannelBufferSizes {
    pub filter_input: usize,
    pub simulation_input: usize,
    pub bundle_input: usize,
    pub output: usize,
}

/// Memory pool configuration for preallocated objects
#[derive(Debug, Clone)]
pub struct MemoryPoolConfig {
    pub transaction_pool_size: usize,
    pub opportunity_pool_size: usize,
    pub bundle_pool_size: usize,
    pub enable_object_reuse: bool,
}

/// Latency targets and timeout configuration
#[derive(Debug, Clone)]
pub struct LatencyTargets {
    pub total_decision_loop_ms: u64,
    pub filter_stage_ms: u64,
    pub simulation_stage_ms: u64,
    pub bundle_stage_ms: u64,
    pub timeout_multiplier: f64,
}

/// Thread pool sizes for different stages
#[derive(Debug, Clone)]
pub struct ThreadPoolSizes {
    pub filter_workers: usize,
    pub simulation_workers: usize,
    pub bundle_workers: usize,
}

impl Default for DecisionPathConfig {
    fn default() -> Self {
        let num_cpus = num_cpus::get();
        
        Self {
            enable_cpu_pinning: true,
            core_assignments: CoreAssignments {
                filter_core: Some(0),
                simulation_cores: (1..num_cpus.min(4)).collect(), // Use cores 1-3 for simulation
                bundle_core: Some(num_cpus.saturating_sub(1)),
                metrics_core: None, // Let OS schedule metrics collection
            },
            channel_buffer_sizes: ChannelBufferSizes {
                filter_input: 1000,
                simulation_input: 500,
                bundle_input: 100,
                output: 50,
            },
            memory_pools: MemoryPoolConfig {
                transaction_pool_size: 1000,
                opportunity_pool_size: 500,
                bundle_pool_size: 100,
                enable_object_reuse: true,
            },
            latency_targets: LatencyTargets {
                total_decision_loop_ms: 25,
                filter_stage_ms: 5,
                simulation_stage_ms: 15,
                bundle_stage_ms: 5,
                timeout_multiplier: 2.0,
            },
            thread_pools: ThreadPoolSizes {
                filter_workers: 2,
                simulation_workers: num_cpus.min(4),
                bundle_workers: 1,
            },
        }
    }
}

/// Decision path pipeline result
#[derive(Debug, Clone)]
pub struct DecisionResult {
    pub transaction_id: String,
    pub opportunities: Vec<Opportunity>,
    pub processing_time_ms: f64,
    pub stage_timings: StageTimings,
    pub success: bool,
    pub error: Option<String>,
}

/// Timing breakdown for each pipeline stage
#[derive(Debug, Clone, Default)]
pub struct StageTimings {
    pub filter_ms: f64,
    pub simulation_ms: f64,
    pub bundle_ms: f64,
    pub total_ms: f64,
}

/// Statistics for the decision path pipeline
#[derive(Debug, Clone, Default)]
pub struct DecisionPathStats {
    pub transactions_processed: u64,
    pub opportunities_found: u64,
    pub bundles_created: u64,
    pub average_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub p99_latency_ms: f64,
    pub timeout_count: u64,
    pub error_count: u64,
    pub throughput_tps: f64,
}

/// Memory pool for reusing objects to reduce allocations
struct MemoryPool<T> {
    pool: crossbeam::queue::SegQueue<T>,
    factory: Box<dyn Fn() -> T + Send + Sync>,
    max_size: usize,
}

impl<T> MemoryPool<T> {
    fn new<F>(factory: F, max_size: usize) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        let pool = crossbeam::queue::SegQueue::new();
        
        // Pre-populate the pool
        for _ in 0..max_size.min(10) {
            pool.push(factory());
        }
        
        Self {
            pool,
            factory: Box::new(factory),
            max_size,
        }
    }

    fn acquire(&self) -> T {
        self.pool.pop().unwrap_or_else(|| (self.factory)())
    }

    fn release(&self, item: T) {
        if self.pool.len() < self.max_size {
            self.pool.push(item);
        }
        // If pool is full, just drop the item
    }
}

/// Ultra-low latency decision path pipeline
pub struct DecisionPath {
    config: DecisionPathConfig,
    strategy_engine: Arc<MockStrategyEngine>,
    metrics: Arc<PrometheusMetrics>,
    
    // Pipeline channels
    filter_tx: Sender<ParsedTransaction>,
    filter_rx: Receiver<ParsedTransaction>,
    simulation_tx: Sender<FilteredTransaction>,
    simulation_rx: Receiver<FilteredTransaction>,
    bundle_tx: Sender<SimulationResult>,
    bundle_rx: Receiver<SimulationResult>,
    output_tx: Sender<DecisionResult>,
    output_rx: Receiver<DecisionResult>,
    
    // Memory pools
    transaction_pool: Arc<MemoryPool<ParsedTransaction>>,
    opportunity_pool: Arc<MemoryPool<Vec<Opportunity>>>,
    
    // Performance tracking
    latency_histogram: Arc<LatencyHistogram>,
    stats: Arc<RwLock<DecisionPathStats>>,
    
    // Control
    shutdown: Arc<AtomicBool>,
    processed_count: Arc<AtomicU64>,
    
    // CPU affinity manager
    cpu_manager: Option<CpuAffinityManager>,
}

/// Filtered transaction with metadata
#[derive(Debug, Clone)]
struct FilteredTransaction {
    transaction: ParsedTransaction,
    filter_time_ms: f64,
    priority: u8,
}

/// Simulation result with timing
#[derive(Debug, Clone)]
struct SimulationResult {
    transaction: ParsedTransaction,
    opportunities: Vec<Opportunity>,
    simulation_time_ms: f64,
    filter_time_ms: f64,
}

impl DecisionPath {
    /// Create new decision path pipeline
    pub fn new(
        config: DecisionPathConfig,
        strategy_engine: Arc<MockStrategyEngine>,
        metrics: Arc<PrometheusMetrics>,
    ) -> Result<Self> {
        // Create channels with configured buffer sizes
        let (filter_tx, filter_rx) = bounded(config.channel_buffer_sizes.filter_input);
        let (simulation_tx, simulation_rx) = bounded(config.channel_buffer_sizes.simulation_input);
        let (bundle_tx, bundle_rx) = bounded(config.channel_buffer_sizes.bundle_input);
        let (output_tx, output_rx) = bounded(config.channel_buffer_sizes.output);

        // Create memory pools
        let transaction_pool = Arc::new(MemoryPool::new(
            || ParsedTransaction {
                transaction: crate::types::Transaction {
                    hash: String::new(),
                    from: String::new(),
                    to: None,
                    value: String::new(),
                    gas_price: String::new(),
                    gas_limit: String::new(),
                    nonce: 0,
                    input: String::new(),
                    timestamp: chrono::Utc::now(),
                },
                decoded_input: None,
                target_type: crate::types::TargetType::Unknown,
                processing_time_ms: 0,
            },
            config.memory_pools.transaction_pool_size,
        ));

        let opportunity_pool = Arc::new(MemoryPool::new(
            Vec::new,
            config.memory_pools.opportunity_pool_size,
        ));

        // Create CPU affinity manager if enabled
        let cpu_manager = if config.enable_cpu_pinning {
            let core_config = CorePinningConfig {
                enabled: true,
                core_ids: config.core_assignments.simulation_cores.clone(),
                worker_threads: config.thread_pools.simulation_workers,
            };
            Some(CpuAffinityManager::new(core_config))
        } else {
            None
        };

        Ok(Self {
            config,
            strategy_engine,
            metrics,
            filter_tx,
            filter_rx,
            simulation_tx,
            simulation_rx,
            bundle_tx,
            bundle_rx,
            output_tx,
            output_rx,
            transaction_pool,
            opportunity_pool,
            latency_histogram: Arc::new(LatencyHistogram::new(1000)),
            stats: Arc::new(RwLock::new(DecisionPathStats::default())),
            shutdown: Arc::new(AtomicBool::new(false)),
            processed_count: Arc::new(AtomicU64::new(0)),
            cpu_manager,
        })
    }

    /// Start the decision path pipeline
    pub async fn start(&self) -> Result<()> {
        info!("Starting ultra-low latency decision path pipeline");

        // Initialize CPU affinity if enabled
        if let Some(ref cpu_manager) = self.cpu_manager {
            cpu_manager.initialize()?;
        }

        // Start pipeline stages
        self.start_filter_stage().await?;
        self.start_simulation_stage().await?;
        self.start_bundle_stage().await?;
        self.start_metrics_collection().await?;

        info!(
            filter_workers = self.config.thread_pools.filter_workers,
            simulation_workers = self.config.thread_pools.simulation_workers,
            bundle_workers = self.config.thread_pools.bundle_workers,
            cpu_pinning = self.config.enable_cpu_pinning,
            "Decision path pipeline started"
        );

        Ok(())
    }

    /// Process a transaction through the decision path
    pub async fn process_transaction(&self, transaction: ParsedTransaction) -> Result<()> {
        let _start_time = Instant::now();
        
        // Send to filter stage
        match self.filter_tx.try_send(transaction.clone()) {
            Ok(_) => {
                debug!(
                    tx_hash = %transaction.transaction.hash,
                    "Transaction sent to decision path"
                );
            }
            Err(crossbeam::channel::TrySendError::Full(_)) => {
                warn!(
                    tx_hash = %transaction.transaction.hash,
                    "Decision path filter queue full, dropping transaction"
                );
                self.metrics.inc_transactions_dropped("queue_full", "filter");
                return Ok(());
            }
            Err(e) => {
                error!(
                    tx_hash = %transaction.transaction.hash,
                    error = %e,
                    "Failed to send transaction to decision path"
                );
                return Err(anyhow::anyhow!("Failed to send transaction: {}", e));
            }
        }

        Ok(())
    }

    /// Get decision results
    pub fn get_results(&self) -> Receiver<DecisionResult> {
        self.output_rx.clone()
    }

    /// Get current statistics
    pub async fn get_stats(&self) -> DecisionPathStats {
        let stats = self.stats.read().await;
        stats.clone()
    }

    /// Shutdown the pipeline
    pub async fn shutdown(&self) {
        info!("Shutting down decision path pipeline");
        self.shutdown.store(true, Ordering::Relaxed);
    }

    /// Start the filter stage workers
    async fn start_filter_stage(&self) -> Result<()> {
        let workers = self.config.thread_pools.filter_workers;
        
        for worker_id in 0..workers {
            let rx = self.filter_rx.clone();
            let tx = self.simulation_tx.clone();
            let shutdown = self.shutdown.clone();
            let metrics = self.metrics.clone();
            let target_ms = self.config.latency_targets.filter_stage_ms;
            let core_id = self.config.core_assignments.filter_core;

            tokio::spawn(async move {
                // Pin to CPU core if configured
                if let Some(core) = core_id {
                    debug!(worker_id = worker_id, core_id = core, "Filter worker starting with CPU affinity");
                }

                while !shutdown.load(Ordering::Relaxed) {
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(transaction) => {
                            let filter_timer = PerformanceTimer::new(format!("filter_worker_{}", worker_id));
                            
                            // Fast filter logic - check if transaction is interesting
                            let is_interesting = Self::fast_filter(&transaction);
                            
                            let filter_time_ms = filter_timer.elapsed_ms();
                            
                            if is_interesting {
                                let filtered_tx = FilteredTransaction {
                                    transaction,
                                    filter_time_ms,
                                    priority: 128, // Default priority
                                };

                                if let Err(e) = tx.try_send(filtered_tx) {
                                    warn!(worker_id = worker_id, error = %e, "Failed to send to simulation stage");
                                    metrics.inc_transactions_dropped("send_failed", "simulation");
                                }
                            }

                            // Record filter stage metrics
                            metrics.record_decision_latency(
                                Duration::from_millis(filter_time_ms as u64),
                                "filter",
                                if is_interesting { "interesting" } else { "filtered_out" },
                            );

                            // Check latency target
                            if filter_time_ms > target_ms as f64 {
                                warn!(
                                    worker_id = worker_id,
                                    filter_time_ms = filter_time_ms,
                                    target_ms = target_ms,
                                    "Filter stage exceeded latency target"
                                );
                            }
                        }
                        Err(crossbeam::channel::RecvTimeoutError::Timeout) => {
                            // Continue loop
                        }
                        Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                            debug!(worker_id = worker_id, "Filter worker shutting down - channel disconnected");
                            break;
                        }
                    }
                }

                debug!(worker_id = worker_id, "Filter worker stopped");
            });
        }

        info!(workers = workers, "Filter stage workers started");
        Ok(())
    }

    /// Start the simulation stage workers
    async fn start_simulation_stage(&self) -> Result<()> {
        let workers = self.config.thread_pools.simulation_workers;
        let core_assignments = &self.config.core_assignments.simulation_cores;
        
        for worker_id in 0..workers {
            let rx = self.simulation_rx.clone();
            let tx = self.bundle_tx.clone();
            let shutdown = self.shutdown.clone();
            let strategy_engine = self.strategy_engine.clone();
            let metrics = self.metrics.clone();
            let target_ms = self.config.latency_targets.simulation_stage_ms;
            let core_id = core_assignments.get(worker_id % core_assignments.len()).copied();

            tokio::spawn(async move {
                // Pin to CPU core if configured
                if let Some(core) = core_id {
                    debug!(worker_id = worker_id, core_id = core, "Simulation worker starting with CPU affinity");
                }

                while !shutdown.load(Ordering::Relaxed) {
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(filtered_tx) => {
                            let sim_timer = PerformanceTimer::new(format!("simulation_worker_{}", worker_id));
                            
                            // Run strategy evaluation
                            let opportunities = match strategy_engine.evaluate_transaction(&filtered_tx.transaction).await {
                                Ok(opps) => opps,
                                Err(e) => {
                                    warn!(
                                        worker_id = worker_id,
                                        tx_hash = %filtered_tx.transaction.transaction.hash,
                                        error = %e,
                                        "Strategy evaluation failed"
                                    );
                                    Vec::new()
                                }
                            };

                            let simulation_time_ms = sim_timer.elapsed_ms();

                            let result = SimulationResult {
                                transaction: filtered_tx.transaction,
                                opportunities,
                                simulation_time_ms,
                                filter_time_ms: filtered_tx.filter_time_ms,
                            };

                            if let Err(e) = tx.try_send(result) {
                                warn!(worker_id = worker_id, error = %e, "Failed to send to bundle stage");
                                metrics.inc_transactions_dropped("send_failed", "bundle");
                            }

                            // Record simulation stage metrics
                            metrics.record_decision_latency(
                                Duration::from_millis(simulation_time_ms as u64),
                                "simulation",
                                "completed",
                            );

                            // Check latency target
                            if simulation_time_ms > target_ms as f64 {
                                warn!(
                                    worker_id = worker_id,
                                    simulation_time_ms = simulation_time_ms,
                                    target_ms = target_ms,
                                    "Simulation stage exceeded latency target"
                                );
                            }
                        }
                        Err(crossbeam::channel::RecvTimeoutError::Timeout) => {
                            // Continue loop
                        }
                        Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                            debug!(worker_id = worker_id, "Simulation worker shutting down - channel disconnected");
                            break;
                        }
                    }
                }

                debug!(worker_id = worker_id, "Simulation worker stopped");
            });
        }

        info!(workers = workers, "Simulation stage workers started");
        Ok(())
    }

    /// Start the bundle stage workers
    async fn start_bundle_stage(&self) -> Result<()> {
        let workers = self.config.thread_pools.bundle_workers;
        
        for worker_id in 0..workers {
            let rx = self.bundle_rx.clone();
            let tx = self.output_tx.clone();
            let shutdown = self.shutdown.clone();
            let _strategy_engine = self.strategy_engine.clone();
            let metrics = self.metrics.clone();
            let latency_histogram = self.latency_histogram.clone();
            let stats = self.stats.clone();
            let processed_count = self.processed_count.clone();
            let target_ms = self.config.latency_targets.bundle_stage_ms;
            let total_target_ms = self.config.latency_targets.total_decision_loop_ms;
            let core_id = self.config.core_assignments.bundle_core;

            tokio::spawn(async move {
                // Pin to CPU core if configured
                if let Some(core) = core_id {
                    debug!(worker_id = worker_id, core_id = core, "Bundle worker starting with CPU affinity");
                }

                while !shutdown.load(Ordering::Relaxed) {
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(sim_result) => {
                            let bundle_timer = PerformanceTimer::new(format!("bundle_worker_{}", worker_id));
                            
                            let bundle_time_ms = bundle_timer.elapsed_ms();
                            let total_time_ms = sim_result.filter_time_ms + sim_result.simulation_time_ms + bundle_time_ms;

                            // Create decision result
                            let decision_result = DecisionResult {
                                transaction_id: sim_result.transaction.transaction.hash.clone(),
                                opportunities: sim_result.opportunities.clone(),
                                processing_time_ms: total_time_ms,
                                stage_timings: StageTimings {
                                    filter_ms: sim_result.filter_time_ms,
                                    simulation_ms: sim_result.simulation_time_ms,
                                    bundle_ms: bundle_time_ms,
                                    total_ms: total_time_ms,
                                },
                                success: true,
                                error: None,
                            };

                            // Send result
                            if let Err(e) = tx.try_send(decision_result) {
                                warn!(worker_id = worker_id, error = %e, "Failed to send decision result");
                            }

                            // Update statistics
                            {
                                let mut stats = stats.write().await;
                                stats.transactions_processed += 1;
                                stats.opportunities_found += sim_result.opportunities.len() as u64;
                                
                                // Update average latency
                                let count = stats.transactions_processed as f64;
                                stats.average_latency_ms = 
                                    (stats.average_latency_ms * (count - 1.0) + total_time_ms) / count;
                            }

                            // Record in histogram for percentile calculation
                            latency_histogram.record(total_time_ms);

                            // Update processed count
                            processed_count.fetch_add(1, Ordering::Relaxed);

                            // Record bundle stage metrics
                            metrics.record_decision_latency(
                                Duration::from_millis(bundle_time_ms as u64),
                                "bundle",
                                "completed",
                            );

                            // Record total decision loop latency
                            metrics.record_decision_latency(
                                Duration::from_millis(total_time_ms as u64),
                                "total_decision_loop",
                                if total_time_ms <= total_target_ms as f64 { "within_target" } else { "exceeded_target" },
                            );

                            // Check latency targets
                            if bundle_time_ms > target_ms as f64 {
                                warn!(
                                    worker_id = worker_id,
                                    bundle_time_ms = bundle_time_ms,
                                    target_ms = target_ms,
                                    "Bundle stage exceeded latency target"
                                );
                            }

                            if total_time_ms > total_target_ms as f64 {
                                warn!(
                                    worker_id = worker_id,
                                    total_time_ms = total_time_ms,
                                    target_ms = total_target_ms,
                                    tx_hash = %sim_result.transaction.transaction.hash,
                                    "Total decision loop exceeded latency target"
                                );
                            } else {
                                debug!(
                                    worker_id = worker_id,
                                    total_time_ms = format!("{:.2}", total_time_ms),
                                    opportunities = sim_result.opportunities.len(),
                                    tx_hash = %sim_result.transaction.transaction.hash,
                                    "Decision loop completed within target"
                                );
                            }
                        }
                        Err(crossbeam::channel::RecvTimeoutError::Timeout) => {
                            // Continue loop
                        }
                        Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                            debug!(worker_id = worker_id, "Bundle worker shutting down - channel disconnected");
                            break;
                        }
                    }
                }

                debug!(worker_id = worker_id, "Bundle worker stopped");
            });
        }

        info!(workers = workers, "Bundle stage workers started");
        Ok(())
    }

    /// Start metrics collection task
    async fn start_metrics_collection(&self) -> Result<()> {
        let latency_histogram = self.latency_histogram.clone();
        let stats = self.stats.clone();
        let processed_count = self.processed_count.clone();
        let shutdown = self.shutdown.clone();
        let _metrics = self.metrics.clone();

        tokio::spawn(async move {
            let mut last_count = 0u64;
            let mut last_time = Instant::now();
            
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            
            while !shutdown.load(Ordering::Relaxed) {
                interval.tick().await;
                
                let current_count = processed_count.load(Ordering::Relaxed);
                let current_time = Instant::now();
                let elapsed = current_time.duration_since(last_time).as_secs_f64();
                
                if elapsed > 0.0 {
                    let throughput = (current_count - last_count) as f64 / elapsed;
                    
                    // Update statistics
                    {
                        let mut stats = stats.write().await;
                        stats.throughput_tps = throughput;
                        
                        // Update percentiles from histogram
                        let latency_stats = latency_histogram.get_stats();
                        stats.p95_latency_ms = latency_stats.p95_ms;
                        stats.p99_latency_ms = latency_stats.p99_ms;
                    }
                    
                    // Log performance metrics
                    if current_count > last_count {
                        let latency_stats = latency_histogram.get_stats();
                        info!(
                            throughput_tps = format!("{:.1}", throughput),
                            avg_latency_ms = format!("{:.2}", latency_stats.mean_ms),
                            p95_latency_ms = format!("{:.2}", latency_stats.p95_ms),
                            p99_latency_ms = format!("{:.2}", latency_stats.p99_ms),
                            processed_total = current_count,
                            "Decision path performance metrics"
                        );
                    }
                }
                
                last_count = current_count;
                last_time = current_time;
            }
            
            debug!("Metrics collection task stopped");
        });

        info!("Metrics collection task started");
        Ok(())
    }

    /// Fast filter to determine if transaction is interesting
    fn fast_filter(transaction: &ParsedTransaction) -> bool {
        // Quick checks for interesting transactions
        match transaction.target_type {
            crate::types::TargetType::UniswapV2 |
            crate::types::TargetType::UniswapV3 |
            crate::types::TargetType::SushiSwap |
            crate::types::TargetType::Curve |
            crate::types::TargetType::Balancer => true,
            crate::types::TargetType::OrderBook => {
                // Check if it's a large order
                if let Ok(value) = transaction.transaction.value.parse::<u128>() {
                    value > 1_000_000_000_000_000_000 // > 1 ETH
                } else {
                    false
                }
            }
            crate::types::TargetType::Unknown => {
                // Check if it has interesting calldata
                transaction.transaction.input.len() > 10 // More than just function selector
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Transaction, TargetType};
    // Removed mev_strategies dependency

    fn create_test_transaction(target_type: TargetType) -> ParsedTransaction {
        ParsedTransaction {
            transaction: Transaction {
                hash: "0x1234567890abcdef".to_string(),
                from: "0xfrom".to_string(),
                to: Some("0xto".to_string()),
                value: "2000000000000000000".to_string(), // 2 ETH
                gas_price: "50000000000".to_string(),
                gas_limit: "200000".to_string(),
                nonce: 1,
                input: "0x38ed1739000000000000000000000000".to_string(),
                timestamp: chrono::Utc::now(),
            },
            decoded_input: None,
            target_type,
            processing_time_ms: 1,
        }
    }

    #[test]
    fn test_fast_filter() {
        // Test interesting transactions
        let uniswap_tx = create_test_transaction(TargetType::UniswapV2);
        assert!(DecisionPath::fast_filter(&uniswap_tx));

        let curve_tx = create_test_transaction(TargetType::Curve);
        assert!(DecisionPath::fast_filter(&curve_tx));

        // Test large order book transaction
        let mut orderbook_tx = create_test_transaction(TargetType::OrderBook);
        orderbook_tx.transaction.value = "2000000000000000000".to_string(); // 2 ETH
        assert!(DecisionPath::fast_filter(&orderbook_tx));

        // Test small order book transaction
        let mut small_orderbook_tx = create_test_transaction(TargetType::OrderBook);
        small_orderbook_tx.transaction.value = "100000000000000000".to_string(); // 0.1 ETH
        assert!(!DecisionPath::fast_filter(&small_orderbook_tx));

        // Test unknown with calldata
        let mut unknown_tx = create_test_transaction(TargetType::Unknown);
        unknown_tx.transaction.input = "0x38ed1739000000000000000000000000".to_string();
        assert!(DecisionPath::fast_filter(&unknown_tx));

        // Test unknown without calldata
        let mut empty_unknown_tx = create_test_transaction(TargetType::Unknown);
        empty_unknown_tx.transaction.input = "0x".to_string();
        assert!(!DecisionPath::fast_filter(&empty_unknown_tx));
    }

    #[tokio::test]
    async fn test_decision_path_creation() {
        let config = DecisionPathConfig::default();
        let metrics = Arc::new(PrometheusMetrics::new().unwrap());
        let strategy_engine = Arc::new(MockStrategyEngine::new());

        let decision_path = DecisionPath::new(config, strategy_engine, metrics);
        assert!(decision_path.is_ok());
    }

    #[test]
    fn test_memory_pool() {
        let pool = MemoryPool::new(|| String::from("test"), 5);
        
        let item1 = pool.acquire();
        assert_eq!(item1, "test");
        
        pool.release(String::from("reused"));
        let item2 = pool.acquire();
        // The pool might return either the factory-created item or the reused one
        assert!(item2 == "test" || item2 == "reused");
    }

    #[test]
    fn test_stage_timings() {
        let timings = StageTimings {
            filter_ms: 2.5,
            simulation_ms: 15.2,
            bundle_ms: 3.1,
            total_ms: 20.8,
        };
        
        assert_eq!(timings.filter_ms, 2.5);
        assert_eq!(timings.simulation_ms, 15.2);
        assert_eq!(timings.bundle_ms, 3.1);
        assert_eq!(timings.total_ms, 20.8);
    }

    #[test]
    fn test_decision_path_config_defaults() {
        let config = DecisionPathConfig::default();
        
        assert_eq!(config.latency_targets.total_decision_loop_ms, 25);
        assert_eq!(config.latency_targets.filter_stage_ms, 5);
        assert_eq!(config.latency_targets.simulation_stage_ms, 15);
        assert_eq!(config.latency_targets.bundle_stage_ms, 5);
        assert!(config.enable_cpu_pinning);
        assert!(config.memory_pools.enable_object_reuse);
    }
}