//! Optimized simulation engine with batching, connection pooling, and safety checks

use anyhow::Result;
use ethers::types::{Address, U256};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime},
};
use tokio::{
    sync::{mpsc, Mutex, RwLock, Semaphore},
    time::timeout,
};
use tracing::{debug, error, info, warn};

use crate::{
    OptimizedRpcClient, PrometheusMetrics, RpcClientConfig, SimulationBundle, SimulationResult,
};

/// Configuration for simulation optimization
#[derive(Debug, Clone)]
pub struct SimulationOptimizerConfig {
    pub batch_size: usize,
    pub batch_timeout_ms: u64,
    pub max_concurrent_batches: usize,
    pub warm_pool_size: usize,
    pub account_reuse_count: u32,
    pub min_profit_threshold_wei: U256,
    pub max_stale_state_ms: u64,
    pub pnl_tracking_enabled: bool,
    pub safety_checks_enabled: bool,
    pub throughput_target_sims_per_sec: f64,
}

impl Default for SimulationOptimizerConfig {
    fn default() -> Self {
        Self {
            batch_size: 10,
            batch_timeout_ms: 50,
            max_concurrent_batches: 20,
            warm_pool_size: 50,
            account_reuse_count: 100,
            min_profit_threshold_wei: U256::from(1_000_000_000_000_000u64), // 0.001 ETH
            max_stale_state_ms: 1000,
            pnl_tracking_enabled: true,
            safety_checks_enabled: true,
            throughput_target_sims_per_sec: 200.0,
        }
    }
}

/// Batch simulation request
#[derive(Debug, Clone)]
pub struct BatchSimulationRequest {
    pub id: String,
    pub bundles: Vec<SimulationBundle>,
    pub priority: SimulationPriority,
    pub timestamp: SystemTime,
}

/// Simulation priority levels
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SimulationPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Batch simulation result
#[derive(Debug, Clone)]
pub struct BatchSimulationResult {
    pub batch_id: String,
    pub results: Vec<SimulationResult>,
    pub batch_latency_ms: f64,
    pub throughput_sims_per_sec: f64,
    pub safety_violations: Vec<SafetyViolation>,
}

/// Safety violation types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SafetyViolation {
    ProfitBelowThreshold {
        bundle_id: String,
        actual_profit_wei: U256,
        threshold_wei: U256,
    },
    StaleState {
        bundle_id: String,
        state_age_ms: u64,
        max_age_ms: u64,
    },
    ExcessiveGasCost {
        bundle_id: String,
        gas_cost_wei: U256,
        profit_wei: U256,
    },
    NegativePnL {
        bundle_id: String,
        net_loss_wei: U256,
    },
}

/// PnL tracking for safety checks
#[derive(Debug, Clone)]
pub struct PnLTracker {
    pub total_simulated_profit_wei: U256,
    pub total_gas_costs_wei: U256,
    pub successful_simulations: u64,
    pub failed_simulations: u64,
    pub safety_violations: u64,
    pub last_update: SystemTime,
}

impl Default for PnLTracker {
    fn default() -> Self {
        Self {
            total_simulated_profit_wei: U256::zero(),
            total_gas_costs_wei: U256::zero(),
            successful_simulations: 0,
            failed_simulations: 0,
            safety_violations: 0,
            last_update: SystemTime::now(),
        }
    }
}

/// Reusable account pool for simulation optimization
#[derive(Debug)]
pub struct AccountPool {
    accounts: VecDeque<Address>,
    usage_count: HashMap<Address, u32>,
    max_reuse: u32,
}

impl AccountPool {
    pub fn new(accounts: Vec<Address>, max_reuse: u32) -> Self {
        let mut usage_count = HashMap::new();
        for account in &accounts {
            usage_count.insert(*account, 0);
        }

        Self {
            accounts: accounts.into(),
            usage_count,
            max_reuse,
        }
    }

    /// Get next available account for simulation
    pub fn get_account(&mut self) -> Option<Address> {
        // Find account with lowest usage count
        if let Some(account) = self.accounts.front().copied() {
            let usage = self.usage_count.get(&account).copied().unwrap_or(0);
            
            if usage < self.max_reuse {
                self.usage_count.insert(account, usage + 1);
                // Rotate to back of queue
                self.accounts.rotate_left(1);
                Some(account)
            } else {
                // Reset usage and use account
                self.usage_count.insert(account, 1);
                self.accounts.rotate_left(1);
                Some(account)
            }
        } else {
            None
        }
    }

    /// Reset usage counts for all accounts
    pub fn reset_usage(&mut self) {
        for count in self.usage_count.values_mut() {
            *count = 0;
        }
    }
}

/// Optimized simulation engine with batching and safety checks
pub struct SimulationOptimizer {
    config: SimulationOptimizerConfig,
    rpc_client: Arc<OptimizedRpcClient>,
    metrics: Arc<PrometheusMetrics>,
    account_pool: Arc<Mutex<AccountPool>>,
    pnl_tracker: Arc<RwLock<PnLTracker>>,
    batch_semaphore: Arc<Semaphore>,
    simulation_counter: Arc<AtomicU64>,
    last_state_update: Arc<RwLock<SystemTime>>,
    
    // Batch processing channels
    batch_sender: mpsc::UnboundedSender<BatchSimulationRequest>,
    result_receivers: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<BatchSimulationResult>>>>,
}

impl SimulationOptimizer {
    /// Create new simulation optimizer
    pub async fn new(
        config: SimulationOptimizerConfig,
        rpc_config: RpcClientConfig,
        metrics: Arc<PrometheusMetrics>,
    ) -> Result<Self> {
        let rpc_client = Arc::new(OptimizedRpcClient::new(rpc_config).await?);
        
        // Create account pool with dummy accounts for testing
        let test_accounts = vec![
            Address::from([1u8; 20]),
            Address::from([2u8; 20]),
            Address::from([3u8; 20]),
            Address::from([4u8; 20]),
            Address::from([5u8; 20]),
        ];
        let account_pool = Arc::new(Mutex::new(AccountPool::new(
            test_accounts,
            config.account_reuse_count,
        )));

        let (batch_sender, batch_receiver) = mpsc::unbounded_channel();
        let result_receivers = Arc::new(Mutex::new(HashMap::new()));

        let optimizer = Self {
            batch_semaphore: Arc::new(Semaphore::new(config.max_concurrent_batches)),
            simulation_counter: Arc::new(AtomicU64::new(0)),
            last_state_update: Arc::new(RwLock::new(SystemTime::now())),
            config,
            rpc_client,
            metrics,
            account_pool,
            pnl_tracker: Arc::new(RwLock::new(PnLTracker::default())),
            batch_sender,
            result_receivers,
        };

        // Start batch processing task
        optimizer.start_batch_processor(batch_receiver).await;

        info!(
            batch_size = optimizer.config.batch_size,
            max_concurrent_batches = optimizer.config.max_concurrent_batches,
            throughput_target = optimizer.config.throughput_target_sims_per_sec,
            "Simulation optimizer initialized"
        );

        Ok(optimizer)
    }

    /// Submit simulation batch for processing
    pub async fn simulate_batch(&self, bundles: Vec<SimulationBundle>) -> Result<BatchSimulationResult> {
        let batch_id = format!("batch_{}", self.simulation_counter.fetch_add(1, Ordering::Relaxed));
        
        let request = BatchSimulationRequest {
            id: batch_id.clone(),
            bundles,
            priority: SimulationPriority::Normal,
            timestamp: SystemTime::now(),
        };

        // Create result channel
        let (result_sender, mut result_receiver) = mpsc::unbounded_channel();
        {
            let mut receivers = self.result_receivers.lock().await;
            receivers.insert(batch_id.clone(), result_sender);
        }

        // Send batch request
        self.batch_sender.send(request)?;

        // Wait for result with timeout
        match timeout(
            Duration::from_millis(self.config.batch_timeout_ms * 10), // Allow extra time for batching
            result_receiver.recv(),
        ).await {
            Ok(Some(result)) => {
                // Clean up receiver
                let mut receivers = self.result_receivers.lock().await;
                receivers.remove(&batch_id);
                Ok(result)
            }
            Ok(None) => Err(anyhow::anyhow!("Batch processing channel closed")),
            Err(_) => Err(anyhow::anyhow!("Batch simulation timeout")),
        }
    }

    /// Start batch processing background task
    async fn start_batch_processor(&self, mut batch_receiver: mpsc::UnboundedReceiver<BatchSimulationRequest>) {
        let config = self.config.clone();
        let rpc_client = self.rpc_client.clone();
        let metrics = self.metrics.clone();
        let account_pool = self.account_pool.clone();
        let pnl_tracker = self.pnl_tracker.clone();
        let batch_semaphore = self.batch_semaphore.clone();
        let result_receivers = self.result_receivers.clone();
        let last_state_update = self.last_state_update.clone();

        tokio::spawn(async move {
            let mut pending_batches: Vec<BatchSimulationRequest> = Vec::new();
            let mut batch_timer = tokio::time::interval(Duration::from_millis(config.batch_timeout_ms));

            loop {
                tokio::select! {
                    // Receive new batch requests
                    request = batch_receiver.recv() => {
                        match request {
                            Some(req) => {
                                pending_batches.push(req);
                                
                                // Process if batch is full
                                if pending_batches.len() >= config.batch_size {
                                    Self::process_pending_batches(
                                        &mut pending_batches,
                                        &config,
                                        &rpc_client,
                                        &metrics,
                                        &account_pool,
                                        &pnl_tracker,
                                        &batch_semaphore,
                                        &result_receivers,
                                        &last_state_update,
                                    ).await;
                                }
                            }
                            None => {
                                warn!("Batch receiver channel closed");
                                break;
                            }
                        }
                    }
                    
                    // Process pending batches on timeout
                    _ = batch_timer.tick() => {
                        if !pending_batches.is_empty() {
                            Self::process_pending_batches(
                                &mut pending_batches,
                                &config,
                                &rpc_client,
                                &metrics,
                                &account_pool,
                                &pnl_tracker,
                                &batch_semaphore,
                                &result_receivers,
                                &last_state_update,
                            ).await;
                        }
                    }
                }
            }
        });
    }

    /// Process accumulated batch requests
    async fn process_pending_batches(
        pending_batches: &mut Vec<BatchSimulationRequest>,
        config: &SimulationOptimizerConfig,
        rpc_client: &Arc<OptimizedRpcClient>,
        metrics: &Arc<PrometheusMetrics>,
        account_pool: &Arc<Mutex<AccountPool>>,
        pnl_tracker: &Arc<RwLock<PnLTracker>>,
        batch_semaphore: &Arc<Semaphore>,
        result_receivers: &Arc<Mutex<HashMap<String, mpsc::UnboundedSender<BatchSimulationResult>>>>,
        last_state_update: &Arc<RwLock<SystemTime>>,
    ) {
        if pending_batches.is_empty() {
            return;
        }

        // Sort by priority
        pending_batches.sort_by(|a, b| b.priority.cmp(&a.priority));

        // Process batches concurrently
        let mut handles = Vec::new();
        
        for request in pending_batches.drain(..) {
            let permit = match batch_semaphore.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => {
                    error!("Failed to acquire batch semaphore");
                    continue;
                }
            };

            let config = config.clone();
            let rpc_client = rpc_client.clone();
            let metrics = metrics.clone();
            let account_pool = account_pool.clone();
            let pnl_tracker = pnl_tracker.clone();
            let result_receivers = result_receivers.clone();
            let last_state_update = last_state_update.clone();

            let handle = tokio::spawn(async move {
                let _permit = permit; // Keep permit alive
                
                let result = Self::process_single_batch(
                    request,
                    &config,
                    &rpc_client,
                    &metrics,
                    &account_pool,
                    &pnl_tracker,
                    &last_state_update,
                ).await;

                // Send result back
                if let Ok(batch_result) = result {
                    let batch_id = batch_result.batch_id.clone();
                    let receivers = result_receivers.lock().await;
                    if let Some(sender) = receivers.get(&batch_id) {
                        if let Err(e) = sender.send(batch_result) {
                            error!("Failed to send batch result: {}", e);
                        }
                    }
                }
            });

            handles.push(handle);
        }

        // Wait for all batches to complete
        for handle in handles {
            if let Err(e) = handle.await {
                error!("Batch processing task failed: {}", e);
            }
        }
    }

    /// Process a single batch of simulations
    async fn process_single_batch(
        request: BatchSimulationRequest,
        config: &SimulationOptimizerConfig,
        rpc_client: &Arc<OptimizedRpcClient>,
        metrics: &Arc<PrometheusMetrics>,
        account_pool: &Arc<Mutex<AccountPool>>,
        pnl_tracker: &Arc<RwLock<PnLTracker>>,
        last_state_update: &Arc<RwLock<SystemTime>>,
    ) -> Result<BatchSimulationResult> {
        let start_time = Instant::now();
        let batch_id = request.id.clone();
        let bundle_count = request.bundles.len();

        debug!(
            batch_id = %batch_id,
            bundle_count = bundle_count,
            "Processing simulation batch"
        );

        // Check for stale state
        let state_age = {
            let last_update = last_state_update.read().await;
            last_update.elapsed().unwrap_or(Duration::ZERO).as_millis() as u64
        };

        let mut safety_violations = Vec::new();
        if config.safety_checks_enabled && state_age > config.max_stale_state_ms {
            for bundle in &request.bundles {
                safety_violations.push(SafetyViolation::StaleState {
                    bundle_id: bundle.id.clone(),
                    state_age_ms: state_age,
                    max_age_ms: config.max_stale_state_ms,
                });
            }
        }

        // Process bundles concurrently with account reuse
        let mut simulation_handles = Vec::new();
        
        for bundle in request.bundles {
            let rpc_client = rpc_client.clone();
            let account_pool = account_pool.clone();
            let config = config.clone();
            
            let handle = tokio::spawn(async move {
                // Get reusable account
                let account = {
                    let mut pool = account_pool.lock().await;
                    pool.get_account()
                };

                Self::simulate_bundle_optimized(bundle, rpc_client, account, &config).await
            });
            
            simulation_handles.push(handle);
        }

        // Collect results
        let mut results = Vec::new();
        for handle in simulation_handles {
            match handle.await {
                Ok(Ok(result)) => {
                    // Apply safety checks
                    if config.safety_checks_enabled {
                        Self::apply_safety_checks(&result, config, &mut safety_violations);
                    }
                    results.push(result);
                }
                Ok(Err(e)) => {
                    error!("Simulation failed: {}", e);
                    metrics.inc_simulations("batch", "error");
                }
                Err(e) => {
                    error!("Simulation task failed: {}", e);
                    metrics.inc_simulations("batch", "task_error");
                }
            }
        }

        // Update PnL tracking
        if config.pnl_tracking_enabled {
            Self::update_pnl_tracking(&results, pnl_tracker).await;
        }

        let batch_latency_ms = start_time.elapsed().as_secs_f64() * 1000.0;
        let throughput_sims_per_sec = if batch_latency_ms > 0.0 {
            (results.len() as f64) / (batch_latency_ms / 1000.0)
        } else {
            0.0
        };

        // Record metrics
        metrics.inc_simulations("batch", "completed");
        metrics.record_decision_latency(
            start_time.elapsed(),
            "batch_simulation",
            "success",
        );

        debug!(
            batch_id = %batch_id,
            results_count = results.len(),
            batch_latency_ms = format!("{:.2}", batch_latency_ms),
            throughput_sims_per_sec = format!("{:.2}", throughput_sims_per_sec),
            safety_violations = safety_violations.len(),
            "Batch simulation completed"
        );

        Ok(BatchSimulationResult {
            batch_id,
            results,
            batch_latency_ms,
            throughput_sims_per_sec,
            safety_violations,
        })
    }

    /// Simulate single bundle with optimizations
    async fn simulate_bundle_optimized(
        bundle: SimulationBundle,
        rpc_client: Arc<OptimizedRpcClient>,
        _reuse_account: Option<Address>,
        _config: &SimulationOptimizerConfig,
    ) -> Result<SimulationResult> {
        let start_time = Instant::now();
        
        // Use optimized RPC client for simulation
        let block_number = rpc_client.get_block_number().await?;
        
        // Simplified simulation using eth_call
        let mut total_gas_used = U256::zero();
        let mut total_gas_cost = U256::zero();
        let mut success = true;

        for tx in &bundle.transactions {
            // Create eth_call request
            let mut call_request = ethers::types::transaction::eip2718::TypedTransaction::default();
            call_request.set_from(tx.from);
            if let Some(to) = tx.to {
                call_request.set_to(to);
            }
            call_request.set_gas(tx.gas_limit);
            call_request.set_gas_price(tx.gas_price);
            call_request.set_value(tx.value);
            call_request.set_data(tx.data.clone());

            match rpc_client.eth_call_optimized(&call_request, Some(block_number.into())).await {
                Ok(_) => {
                    total_gas_used += tx.gas_limit; // Simplified
                    total_gas_cost += tx.gas_price * tx.gas_limit;
                }
                Err(_) => {
                    success = false;
                    break;
                }
            }
        }

        // Calculate profit estimate
        let gross_profit_wei = if success {
            U256::from(1_000_000_000_000_000u64) // 0.001 ETH simplified profit
        } else {
            U256::zero()
        };

        let net_profit_wei = if gross_profit_wei > total_gas_cost {
            gross_profit_wei - total_gas_cost
        } else {
            U256::zero()
        };

        let profit_estimate = crate::ProfitEstimate {
            gross_profit_wei,
            gas_cost_wei: total_gas_cost,
            net_profit_wei,
            profit_margin: if gross_profit_wei > U256::zero() {
                net_profit_wei.as_u128() as f64 / gross_profit_wei.as_u128() as f64
            } else {
                0.0
            },
            confidence: if success { 0.8 } else { 0.0 },
        };

        Ok(SimulationResult {
            bundle_id: bundle.id,
            success,
            gas_used: total_gas_used,
            gas_cost: total_gas_cost,
            profit_estimate,
            transaction_results: vec![], // Simplified
            simulation_time_ms: start_time.elapsed().as_secs_f64() * 1000.0,
            error_message: None,
            block_number: 0,
            effective_gas_price: U256::zero(),
            revert_reason: None,
        })
    }

    /// Apply safety checks to simulation results
    fn apply_safety_checks(
        result: &SimulationResult,
        config: &SimulationOptimizerConfig,
        violations: &mut Vec<SafetyViolation>,
    ) {
        // Check minimum profit threshold
        if result.profit_estimate.net_profit_wei < config.min_profit_threshold_wei {
            violations.push(SafetyViolation::ProfitBelowThreshold {
                bundle_id: result.bundle_id.clone(),
                actual_profit_wei: result.profit_estimate.net_profit_wei,
                threshold_wei: config.min_profit_threshold_wei,
            });
        }

        // Check for excessive gas costs
        if result.profit_estimate.gas_cost_wei > result.profit_estimate.gross_profit_wei {
            violations.push(SafetyViolation::ExcessiveGasCost {
                bundle_id: result.bundle_id.clone(),
                gas_cost_wei: result.profit_estimate.gas_cost_wei,
                profit_wei: result.profit_estimate.gross_profit_wei,
            });
        }

        // Check for negative PnL
        if result.profit_estimate.net_profit_wei == U256::zero() && result.profit_estimate.gas_cost_wei > U256::zero() {
            violations.push(SafetyViolation::NegativePnL {
                bundle_id: result.bundle_id.clone(),
                net_loss_wei: result.profit_estimate.gas_cost_wei,
            });
        }
    }

    /// Update PnL tracking with simulation results
    async fn update_pnl_tracking(
        results: &[SimulationResult],
        pnl_tracker: &Arc<RwLock<PnLTracker>>,
    ) {
        let mut tracker = pnl_tracker.write().await;
        
        for result in results {
            tracker.total_simulated_profit_wei += result.profit_estimate.gross_profit_wei;
            tracker.total_gas_costs_wei += result.profit_estimate.gas_cost_wei;
            
            if result.success {
                tracker.successful_simulations += 1;
            } else {
                tracker.failed_simulations += 1;
            }
        }
        
        tracker.last_update = SystemTime::now();
    }

    /// Update state timestamp to prevent stale state violations
    pub async fn update_state_timestamp(&self) {
        let mut last_update = self.last_state_update.write().await;
        *last_update = SystemTime::now();
    }

    /// Get current PnL statistics
    pub async fn get_pnl_stats(&self) -> PnLTracker {
        self.pnl_tracker.read().await.clone()
    }

    /// Get throughput statistics
    pub async fn get_throughput_stats(&self) -> ThroughputStats {
        let simulation_count = self.simulation_counter.load(Ordering::Relaxed);
        let active_batches = self.config.max_concurrent_batches - self.batch_semaphore.available_permits();
        
        ThroughputStats {
            total_simulations: simulation_count,
            active_batches,
            max_concurrent_batches: self.config.max_concurrent_batches,
            target_throughput_sims_per_sec: self.config.throughput_target_sims_per_sec,
            batch_size: self.config.batch_size,
        }
    }
}

/// Throughput statistics
#[derive(Debug, Clone)]
pub struct ThroughputStats {
    pub total_simulations: u64,
    pub active_batches: usize,
    pub max_concurrent_batches: usize,
    pub target_throughput_sims_per_sec: f64,
    pub batch_size: usize,
}

impl ThroughputStats {
    pub fn print_summary(&self) {
        info!(
            total_simulations = self.total_simulations,
            active_batches = format!("{}/{}", self.active_batches, self.max_concurrent_batches),
            target_throughput = format!("{:.0} sims/sec", self.target_throughput_sims_per_sec),
            batch_size = self.batch_size,
            "Simulation throughput statistics"
        );
    }
}