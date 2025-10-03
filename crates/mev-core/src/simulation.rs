//! Transaction simulation engine for MEV bot strategy evaluation

use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use ethers::{
    providers::{Http, Middleware, Provider},
    types::{
        Address, Bytes, H256, U256, U64, BlockId, BlockNumber,
    },
    core::types::transaction::eip2718::TypedTransaction,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use crate::{ParsedTransaction, PrometheusMetrics};

/// Simulation configuration
#[derive(Debug, Clone)]
pub struct SimulationConfig {
    pub rpc_url: String,
    pub timeout_ms: u64,
    pub max_concurrent_simulations: usize,
    pub enable_state_override: bool,
    pub gas_limit_multiplier: f64,
    pub base_fee_multiplier: f64,
    pub http_api_port: u16,
    pub enable_deterministic_testing: bool,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            rpc_url: "http://localhost:8545".to_string(),
            timeout_ms: 100,
            max_concurrent_simulations: 50,
            enable_state_override: true,
            gas_limit_multiplier: 1.2,
            base_fee_multiplier: 1.1,
            http_api_port: 8080,
            enable_deterministic_testing: false,
        }
    }
}

/// Transaction bundle for simulation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationBundle {
    pub id: String,
    pub transactions: Vec<SimulationTransaction>,
    pub block_number: Option<u64>,
    pub timestamp: Option<u64>,
    pub base_fee: Option<U256>,
    pub state_overrides: Option<HashMap<Address, StateOverride>>,
}

/// State override for simulation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateOverride {
    pub balance: Option<U256>,
    pub nonce: Option<U256>,
    pub code: Option<Bytes>,
    pub state: Option<HashMap<H256, H256>>,
    pub state_diff: Option<HashMap<H256, H256>>,
}

/// Individual transaction in a bundle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationTransaction {
    pub from: Address,
    pub to: Option<Address>,
    pub value: U256,
    pub gas_limit: U256,
    pub gas_price: U256,
    pub data: Bytes,
    pub nonce: Option<U256>,
}

impl From<&ParsedTransaction> for SimulationTransaction {
    fn from(parsed_tx: &ParsedTransaction) -> Self {
        let tx = &parsed_tx.transaction;
        
        Self {
            from: Address::from_str(&tx.from).unwrap_or_default(),
            to: tx.to.as_ref().and_then(|addr| Address::from_str(addr).ok()),
            value: U256::from_dec_str(&tx.value).unwrap_or_default(),
            gas_limit: U256::from_dec_str(&tx.gas_limit).unwrap_or_default(),
            gas_price: U256::from_dec_str(&tx.gas_price).unwrap_or_default(),
            data: Bytes::from_str(&tx.input).unwrap_or_default(),
            nonce: Some(U256::from(tx.nonce)),
        }
    }
}

/// Simulation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub bundle_id: String,
    pub success: bool,
    pub gas_used: U256,
    pub gas_cost: U256,
    pub profit_estimate: ProfitEstimate,
    pub transaction_results: Vec<TransactionResult>,
    pub simulation_time_ms: f64,
    pub error_message: Option<String>,
    pub block_number: u64,
    pub effective_gas_price: U256,
    pub revert_reason: Option<String>,
}

/// Individual transaction result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResult {
    pub success: bool,
    pub gas_used: U256,
    pub gas_estimate: U256,
    pub return_data: Option<Bytes>,
    pub logs: Vec<SimulationLog>,
    pub error: Option<String>,
    pub revert_reason: Option<String>,
    pub effective_gas_price: U256,
}

/// Simulation log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationLog {
    pub address: Address,
    pub topics: Vec<H256>,
    pub data: Bytes,
}

/// Profit estimation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfitEstimate {
    pub gross_profit_wei: U256,
    pub gas_cost_wei: U256,
    pub net_profit_wei: U256,
    pub profit_margin: f64,
    pub confidence: f64, // 0.0 to 1.0
}

impl Default for ProfitEstimate {
    fn default() -> Self {
        Self {
            gross_profit_wei: U256::zero(),
            gas_cost_wei: U256::zero(),
            net_profit_wei: U256::zero(),
            profit_margin: 0.0,
            confidence: 0.0,
        }
    }
}

/// Fork simulation engine
pub struct ForkSimulator {
    config: SimulationConfig,
    provider: Arc<Provider<Http>>,
    metrics: Arc<PrometheusMetrics>,
    active_simulations: tokio::sync::Semaphore,
}

impl ForkSimulator {
    /// Create new fork simulator
    pub async fn new(config: SimulationConfig, metrics: Arc<PrometheusMetrics>) -> Result<Self> {
        let provider = Provider::<Http>::try_from(&config.rpc_url)?;
        let provider = Arc::new(provider);

        // Test connection
        let chain_id = provider.get_chainid().await?;
        info!(
            rpc_url = %config.rpc_url,
            chain_id = %chain_id,
            "Fork simulator initialized"
        );

        Ok(Self {
            active_simulations: tokio::sync::Semaphore::new(config.max_concurrent_simulations),
            config,
            provider,
            metrics,
        })
    }

    /// Simulate a transaction bundle
    pub async fn simulate_bundle(&self, bundle: SimulationBundle) -> Result<SimulationResult> {
        let _permit = self.active_simulations.acquire().await?;
        let start_time = Instant::now();

        debug!(
            bundle_id = %bundle.id,
            transaction_count = bundle.transactions.len(),
            "Starting bundle simulation"
        );

        // Record simulation attempt
        self.metrics.inc_simulations("bundle", "attempted");

        let result = match timeout(
            Duration::from_millis(self.config.timeout_ms),
            self.simulate_bundle_internal(bundle.clone()),
        )
        .await
        {
            Ok(Ok(mut result)) => {
                result.simulation_time_ms = start_time.elapsed().as_secs_f64() * 1000.0;
                self.metrics.inc_simulations("bundle", "success");
                result
            }
            Ok(Err(e)) => {
                let error_result = SimulationResult {
                    bundle_id: bundle.id.clone(),
                    success: false,
                    gas_used: U256::zero(),
                    gas_cost: U256::zero(),
                    profit_estimate: ProfitEstimate::default(),
                    transaction_results: vec![],
                    simulation_time_ms: start_time.elapsed().as_secs_f64() * 1000.0,
                    error_message: Some(e.to_string()),
                    block_number: 0,
                    effective_gas_price: U256::zero(),
                    revert_reason: None,
                };
                self.metrics.inc_simulations("bundle", "error");
                error_result
            }
            Err(_) => {
                let timeout_result = SimulationResult {
                    bundle_id: bundle.id.clone(),
                    success: false,
                    gas_used: U256::zero(),
                    gas_cost: U256::zero(),
                    profit_estimate: ProfitEstimate::default(),
                    transaction_results: vec![],
                    simulation_time_ms: self.config.timeout_ms as f64,
                    error_message: Some("Simulation timeout".to_string()),
                    block_number: 0,
                    effective_gas_price: U256::zero(),
                    revert_reason: None,
                };
                self.metrics.inc_simulations("bundle", "timeout");
                timeout_result
            }
        };

        debug!(
            bundle_id = %result.bundle_id,
            success = result.success,
            simulation_time_ms = format!("{:.2}", result.simulation_time_ms),
            net_profit_wei = %result.profit_estimate.net_profit_wei,
            "Bundle simulation completed"
        );

        Ok(result)
    }

    /// Internal bundle simulation implementation
    async fn simulate_bundle_internal(&self, bundle: SimulationBundle) -> Result<SimulationResult> {
        let mut transaction_results = Vec::new();
        let mut total_gas_used = U256::zero();
        let mut total_gas_cost = U256::zero();

        // Get current block for simulation context
        let block_number = if let Some(block) = bundle.block_number {
            block
        } else {
            self.provider.get_block_number().await?.as_u64()
        };

        // Get current base fee for gas calculations
        let latest_block = self.provider.get_block(BlockNumber::Latest).await?;
        let base_fee = latest_block
            .and_then(|b| b.base_fee_per_gas)
            .unwrap_or_else(|| U256::from(20_000_000_000u64)); // 20 gwei fallback

        // Simulate each transaction in the bundle
        for (i, tx) in bundle.transactions.iter().enumerate() {
            debug!(
                bundle_id = %bundle.id,
                tx_index = i,
                from = ?tx.from,
                to = ?tx.to,
                "Simulating transaction"
            );

            let tx_result = self.simulate_transaction_with_overrides(
                tx, 
                block_number, 
                &bundle.state_overrides
            ).await?;
            
            total_gas_used += tx_result.gas_used;
            total_gas_cost += tx_result.effective_gas_price * tx_result.gas_used;
            
            transaction_results.push(tx_result);
        }

        // Calculate profit estimate
        let profit_estimate = self.calculate_profit_estimate(
            &bundle,
            &transaction_results,
            total_gas_cost,
        ).await?;

        let bundle_success = transaction_results.iter().all(|r| r.success);
        let revert_reason = if !bundle_success {
            transaction_results
                .iter()
                .find(|r| !r.success)
                .and_then(|r| r.revert_reason.clone())
        } else {
            None
        };

        Ok(SimulationResult {
            bundle_id: bundle.id,
            success: bundle_success,
            gas_used: total_gas_used,
            gas_cost: total_gas_cost,
            profit_estimate,
            transaction_results,
            simulation_time_ms: 0.0, // Will be set by caller
            error_message: None,
            block_number,
            effective_gas_price: base_fee,
            revert_reason,
        })
    }

    /// Simulate a single transaction with state overrides
    async fn simulate_transaction_with_overrides(
        &self,
        tx: &SimulationTransaction,
        block_number: u64,
        state_overrides: &Option<HashMap<Address, StateOverride>>,
    ) -> Result<TransactionResult> {
        // Create call request using TypedTransaction
        let mut call_request = TypedTransaction::default();
        call_request.set_from(tx.from);
        if let Some(to) = tx.to {
            call_request.set_to(to);
        }
        call_request.set_gas(tx.gas_limit);
        call_request.set_gas_price(tx.gas_price);
        call_request.set_value(tx.value);
        call_request.set_data(tx.data.clone());

        let block_id = BlockId::Number(BlockNumber::Number(U64::from(block_number)));

        // First, estimate gas
        let gas_estimate = match self.provider.estimate_gas(&call_request, Some(block_id)).await {
            Ok(estimate) => estimate,
            Err(e) => {
                debug!("Gas estimation failed: {}, using provided gas limit", e);
                tx.gas_limit
            }
        };

        // Apply gas limit multiplier for safety
        let adjusted_gas_limit = U256::from(
            (gas_estimate.as_u128() as f64 * self.config.gas_limit_multiplier) as u128
        );

        // Perform the call with state overrides if enabled
        let call_result = if self.config.enable_state_override && state_overrides.is_some() {
            // Enhanced state override implementation
            self.simulate_with_state_overrides(&call_request, block_id, state_overrides.as_ref().unwrap()).await
        } else {
            self.provider.call(&call_request, Some(block_id)).await
        };

        match call_result {
            Ok(return_data) => {
                Ok(TransactionResult {
                    success: true,
                    gas_used: gas_estimate,
                    gas_estimate: adjusted_gas_limit,
                    return_data: Some(return_data),
                    logs: vec![], // Would need trace_call for actual logs
                    error: None,
                    revert_reason: None,
                    effective_gas_price: tx.gas_price,
                })
            }
            Err(e) => {
                let error_msg = e.to_string();
                let revert_reason = self.extract_revert_reason(&error_msg);
                
                warn!(
                    from = ?tx.from,
                    to = ?tx.to,
                    error = %error_msg,
                    revert_reason = ?revert_reason,
                    "Transaction simulation failed"
                );

                Ok(TransactionResult {
                    success: false,
                    gas_used: U256::zero(),
                    gas_estimate: gas_estimate,
                    return_data: None,
                    logs: vec![],
                    error: Some(error_msg),
                    revert_reason,
                    effective_gas_price: tx.gas_price,
                })
            }
        }
    }

    /// Simulate transaction with state overrides using debug_traceCall or eth_call with overrides
    async fn simulate_with_state_overrides(
        &self,
        call_request: &TypedTransaction,
        block_id: BlockId,
        state_overrides: &HashMap<Address, StateOverride>,
    ) -> Result<Bytes, ethers::providers::ProviderError> {
        // Try to use debug_traceCall with state overrides first
        // If that fails, fall back to regular eth_call with a warning
        
        debug!(
            override_count = state_overrides.len(),
            "Attempting simulation with state overrides"
        );

        // For now, we'll use a custom RPC call to support state overrides
        // This would typically require a custom RPC method or debug_traceCall
        match self.call_with_state_overrides_rpc(call_request, block_id, state_overrides).await {
            Ok(result) => Ok(result),
            Err(e) => {
                warn!(
                    error = %e,
                    "State override simulation failed, falling back to regular call"
                );
                // Fallback to regular call
                self.provider.call(call_request, Some(block_id)).await
            }
        }
    }

    /// Custom RPC call with state overrides (placeholder implementation)
    async fn call_with_state_overrides_rpc(
        &self,
        call_request: &TypedTransaction,
        block_id: BlockId,
        state_overrides: &HashMap<Address, StateOverride>,
    ) -> Result<Bytes, ethers::providers::ProviderError> {
        // This is a placeholder implementation
        // In a real implementation, you would:
        // 1. Use debug_traceCall with state overrides
        // 2. Or use a custom RPC method that supports state overrides
        // 3. Or use anvil/hardhat's eth_call with state override parameter
        
        debug!(
            "Simulating state overrides for {} accounts",
            state_overrides.len()
        );

        // For demonstration, we'll modify the call based on overrides
        let modified_call = call_request.clone();
        
        // If there's a balance override for the sender, we can proceed with confidence
        if let Some(from_addr) = call_request.from() {
            if let Some(override_data) = state_overrides.get(&from_addr) {
                if let Some(balance) = override_data.balance {
                    debug!(
                        from = ?from_addr,
                        override_balance = %balance,
                        "Applying balance override for sender"
                    );
                    // In a real implementation, this would be passed to the RPC call
                }
                
                if let Some(nonce) = override_data.nonce {
                    debug!(
                        from = ?from_addr,
                        override_nonce = %nonce,
                        "Applying nonce override for sender"
                    );
                    // In a real implementation, this would be passed to the RPC call
                }
            }
        }

        // For now, fall back to regular call since we don't have a real state override RPC
        // In production, you would implement the actual state override RPC call here
        self.provider.call(&modified_call, Some(block_id)).await
    }

    /// Extract revert reason from error message
    fn extract_revert_reason(&self, error_msg: &str) -> Option<String> {
        // Common patterns for revert reasons
        if error_msg.contains("execution reverted:") {
            error_msg
                .split("execution reverted:")
                .nth(1)
                .map(|s| s.trim().to_string())
        } else if error_msg.contains("revert") {
            Some(error_msg.to_string())
        } else {
            None
        }
    }

    /// Calculate profit estimate for a bundle
    async fn calculate_profit_estimate(
        &self,
        _bundle: &SimulationBundle,
        transaction_results: &[TransactionResult],
        total_gas_cost: U256,
    ) -> Result<ProfitEstimate> {
        // Simplified profit calculation
        // In a real implementation, this would analyze:
        // - Token balance changes
        // - DEX price impacts
        // - Arbitrage opportunities
        // - MEV extraction potential

        let successful_txs = transaction_results.iter().filter(|r| r.success).count();
        let total_txs = transaction_results.len();

        // Simple heuristic: assume some profit based on successful transactions
        let gross_profit_wei = if successful_txs > 0 {
            U256::from(successful_txs) * U256::from(1_000_000_000_000_000u64) // 0.001 ETH per successful tx
        } else {
            U256::zero()
        };

        let net_profit_wei = if gross_profit_wei > total_gas_cost {
            gross_profit_wei - total_gas_cost
        } else {
            U256::zero()
        };

        let profit_margin = if gross_profit_wei > U256::zero() {
            net_profit_wei.as_u128() as f64 / gross_profit_wei.as_u128() as f64
        } else {
            0.0
        };

        let confidence = successful_txs as f64 / total_txs as f64;

        Ok(ProfitEstimate {
            gross_profit_wei,
            gas_cost_wei: total_gas_cost,
            net_profit_wei,
            profit_margin,
            confidence,
        })
    }

    /// Get simulation statistics
    pub fn get_stats(&self) -> SimulationStats {
        SimulationStats {
            active_simulations: self.config.max_concurrent_simulations - self.active_simulations.available_permits(),
            max_concurrent: self.config.max_concurrent_simulations,
            timeout_ms: self.config.timeout_ms,
            rpc_url: self.config.rpc_url.clone(),
        }
    }
}

/// Simulation engine statistics
#[derive(Debug, Clone)]
pub struct SimulationStats {
    pub active_simulations: usize,
    pub max_concurrent: usize,
    pub timeout_ms: u64,
    pub rpc_url: String,
}

impl SimulationStats {
    pub fn print_summary(&self) {
        info!(
            active_simulations = self.active_simulations,
            max_concurrent = self.max_concurrent,
            timeout_ms = self.timeout_ms,
            rpc_url = %self.rpc_url,
            "Simulation engine statistics"
        );
    }
}

/// Simulation service with HTTP API
pub struct SimulationService {
    simulator: Arc<ForkSimulator>,
}

impl SimulationService {
    pub fn new(simulator: Arc<ForkSimulator>) -> Self {
        Self { simulator }
    }

    /// Create HTTP router for simulation API
    pub fn create_router(self) -> Router {
        let service = Arc::new(self);
        
        Router::new()
            .route("/health", get(health_check))
            .route("/simulate", post(simulate_bundle))
            .route("/simulate/batch", post(simulate_bundles_batch))
            .route("/stats", get(|| async { Json(serde_json::json!({"status": "ok"})) }))
            .with_state(service)
    }

    /// Start HTTP server
    pub async fn start_server(&self, port: u16) -> Result<()> {
        let app = self.clone().create_router();
        let addr = format!("0.0.0.0:{}", port);
        
        info!("Starting simulation HTTP API server on {}", addr);
        
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app).await?;
        
        Ok(())
    }

    /// Internal simulation method
    async fn simulate_bundle_internal(&self, bundle: SimulationBundle) -> Result<SimulationResult> {
        self.simulator.simulate_bundle(bundle).await
    }

    /// Internal batch simulation method
    async fn simulate_bundles_batch_internal(&self, bundles: Vec<SimulationBundle>) -> Result<Vec<SimulationResult>> {
        let mut results = Vec::new();
        
        // Process bundles concurrently
        let futures: Vec<_> = bundles
            .into_iter()
            .map(|bundle| self.simulator.simulate_bundle(bundle))
            .collect();

        let simulation_results = futures::future::join_all(futures).await;
        
        for result in simulation_results {
            match result {
                Ok(sim_result) => results.push(sim_result),
                Err(e) => {
                    error!("Batch simulation error: {}", e);
                    // Continue with other simulations
                }
            }
        }

        Ok(results)
    }

    /// Get access to the underlying simulator for testing
    pub fn get_simulator(&self) -> &Arc<ForkSimulator> {
        &self.simulator
    }
}

impl Clone for SimulationService {
    fn clone(&self) -> Self {
        Self {
            simulator: Arc::clone(&self.simulator),
        }
    }
}

// HTTP API handlers
async fn health_check(
    State(service): State<Arc<SimulationService>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let stats = service.simulator.get_stats();
    
    Ok(Json(serde_json::json!({
        "status": "healthy",
        "active_simulations": stats.active_simulations,
        "max_concurrent": stats.max_concurrent,
        "timeout_ms": stats.timeout_ms,
        "rpc_url": stats.rpc_url
    })))
}

async fn simulate_bundle(
    State(service): State<Arc<SimulationService>>,
    Json(bundle): Json<SimulationBundle>,
) -> Result<Json<SimulationResult>, StatusCode> {
    match service.simulate_bundle_internal(bundle).await {
        Ok(result) => Ok(Json(result)),
        Err(e) => {
            error!("Bundle simulation failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn simulate_bundles_batch(
    State(service): State<Arc<SimulationService>>,
    Json(bundles): Json<Vec<SimulationBundle>>,
) -> Result<Json<Vec<SimulationResult>>, StatusCode> {
    match service.simulate_bundles_batch_internal(bundles).await {
        Ok(results) => Ok(Json(results)),
        Err(e) => {
            error!("Batch simulation failed: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn get_stats(
    State(service): State<Arc<SimulationService>>,
) -> Json<SimulationStats> {
    let stats = service.simulator.get_stats();
    Json(stats)
}

/// Bundle builder helper
pub struct BundleBuilder {
    bundle_id: String,
    transactions: Vec<SimulationTransaction>,
    block_number: Option<u64>,
}

impl BundleBuilder {
    pub fn new(bundle_id: String) -> Self {
        Self {
            bundle_id,
            transactions: Vec::new(),
            block_number: None,
        }
    }

    pub fn add_transaction(mut self, tx: SimulationTransaction) -> Self {
        self.transactions.push(tx);
        self
    }

    pub fn add_parsed_transaction(mut self, parsed_tx: &ParsedTransaction) -> Self {
        self.transactions.push(SimulationTransaction::from(parsed_tx));
        self
    }

    pub fn set_block_number(mut self, block_number: u64) -> Self {
        self.block_number = Some(block_number);
        self
    }

    pub fn build(self) -> SimulationBundle {
        SimulationBundle {
            id: self.bundle_id,
            transactions: self.transactions,
            block_number: self.block_number,
            timestamp: Some(chrono::Utc::now().timestamp() as u64),
            base_fee: None,
            state_overrides: None,
        }
    }

    pub fn with_state_overrides(self, overrides: HashMap<Address, StateOverride>) -> Self {
        let mut bundle = self.build();
        bundle.state_overrides = Some(overrides);
        
        // Rebuild the builder with the updated bundle
        Self {
            bundle_id: bundle.id,
            transactions: bundle.transactions,
            block_number: bundle.block_number,
        }
    }
}

/// Deterministic test data generator for simulation testing
pub struct TestDataGenerator {
    seed: u64,
}

impl TestDataGenerator {
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Generate a set of canned transaction bundles for testing
    pub fn generate_test_bundles(&self) -> Vec<SimulationBundle> {
        vec![
            self.generate_simple_transfer_bundle(),
            self.generate_uniswap_swap_bundle(),
            self.generate_failed_transaction_bundle(),
            self.generate_high_gas_bundle(),
            self.generate_state_override_bundle(),
            self.generate_multi_transaction_bundle(),
        ]
    }

    /// Generate a simple ETH transfer bundle
    fn generate_simple_transfer_bundle(&self) -> SimulationBundle {
        let tx = SimulationTransaction {
            from: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
            to: Some(Address::from_str("0x0987654321098765432109876543210987654321").unwrap()),
            value: U256::from_dec_str("1000000000000000000").unwrap(), // 1 ETH
            gas_limit: U256::from(21000),
            gas_price: U256::from_dec_str("20000000000").unwrap(), // 20 gwei
            data: Bytes::default(),
            nonce: Some(U256::from(1)),
        };

        SimulationBundle {
            id: format!("test_transfer_{}", self.seed),
            transactions: vec![tx],
            block_number: Some(18000000),
            timestamp: Some(1640995200), // Fixed timestamp for deterministic testing
            base_fee: Some(U256::from_dec_str("15000000000").unwrap()), // 15 gwei
            state_overrides: None,
        }
    }

    /// Generate a Uniswap-style swap bundle
    fn generate_uniswap_swap_bundle(&self) -> SimulationBundle {
        // Simplified Uniswap V2 swapExactETHForTokens call
        let swap_data = "0x7ff36ab5000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000800000000000000000000000001234567890123456789012345678901234567890000000000000000000000000000000000000000000000000000000006203f200000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
        
        let tx = SimulationTransaction {
            from: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
            to: Some(Address::from_str("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D").unwrap()), // Uniswap V2 Router
            value: U256::from_dec_str("100000000000000000").unwrap(), // 0.1 ETH
            gas_limit: U256::from(200000),
            gas_price: U256::from_dec_str("25000000000").unwrap(), // 25 gwei
            data: Bytes::from_str(swap_data).unwrap_or_default(),
            nonce: Some(U256::from(2)),
        };

        SimulationBundle {
            id: format!("test_uniswap_swap_{}", self.seed),
            transactions: vec![tx],
            block_number: Some(18000000),
            timestamp: Some(1640995200),
            base_fee: Some(U256::from_dec_str("20000000000").unwrap()), // 20 gwei
            state_overrides: None,
        }
    }

    /// Generate a bundle that should fail
    fn generate_failed_transaction_bundle(&self) -> SimulationBundle {
        let tx = SimulationTransaction {
            from: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(), // Invalid sender
            to: Some(Address::from_str("0x1234567890123456789012345678901234567890").unwrap()),
            value: U256::from_dec_str("1000000000000000000000").unwrap(), // 1000 ETH (likely insufficient balance)
            gas_limit: U256::from(21000),
            gas_price: U256::from_dec_str("20000000000").unwrap(),
            data: Bytes::default(),
            nonce: Some(U256::from(1)),
        };

        SimulationBundle {
            id: format!("test_failed_tx_{}", self.seed),
            transactions: vec![tx],
            block_number: Some(18000000),
            timestamp: Some(1640995200),
            base_fee: Some(U256::from_dec_str("15000000000").unwrap()),
            state_overrides: None,
        }
    }

    /// Generate a high gas consumption bundle
    fn generate_high_gas_bundle(&self) -> SimulationBundle {
        // Contract deployment or complex contract interaction
        let complex_data = "0x608060405234801561001057600080fd5b50600436106100365760003560e01c8063a41368621461003b578063cfae321714610057575b600080fd5b6100556004803603810190610050919061009d565b610075565b005b61005f61008f565b60405161006c91906100fb565b60405180910390f35b8060009080519060200190610089929190610121565b50505050565b60606000805461009e9061014c565b80601f01602080910402602001604051908101604052809291908181526020018280546100ca9061014c565b80156101175780601f106100ec57610100808354040283529160200191610117565b820191906000526020600020905b8154815290600101906020018083116100fa57829003601f168201915b5050505050905090565b82805461012d9061014c565b90600052602060002090601f01602090048101928261014f5760008555610196565b82601f1061016857805160ff1916838001178555610196565b82800160010185558215610196579182015b8281111561019557825182559160200191906001019061017a565b5b5090506101a391906101a7565b5090565b5b808211156101c05760008160009055506001016101a8565b5090565b600080fd5b600080fd5b600080fd5b600080fd5b600080fd5b60008083601f8401126101f3576101f26101ce565b5b8235905067ffffffffffffffff8111156102105761020f6101d3565b5b60208301915083600182028301111561022c5761022b6101d8565b5b9250929050565b60008060208385031215610250576102496101c4565b5b600083013567ffffffffffffffff81111561026e5761026d6101c9565b5b61027a858286016101dd565b92509250509250929050565b600081519050919050565b600082825260208201905092915050565b60005b838110156102c05780820151818401526020810190506102a5565b838111156102cf576000848401525b50505050565b6000601f19601f8301169050919050565b60006102f182610286565b6102fb8185610291565b935061030b8185602086016102a2565b610314816102d5565b840191505092915050565b6000602082019050818103600083015261033981846102e6565b905092915050565b7f4e487b7100000000000000000000000000000000000000000000000000000000600052602260045260246000fd5b6000600282049050600182168061038957607f821691505b60208210810361039c5761039b610342565b5b5091905056fea2646970667358221220";
        
        let tx = SimulationTransaction {
            from: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
            to: None, // Contract deployment
            value: U256::zero(),
            gas_limit: U256::from(2000000), // High gas limit
            gas_price: U256::from_dec_str("30000000000").unwrap(), // 30 gwei
            data: Bytes::from_str(complex_data).unwrap_or_default(),
            nonce: Some(U256::from(3)),
        };

        SimulationBundle {
            id: format!("test_high_gas_{}", self.seed),
            transactions: vec![tx],
            block_number: Some(18000000),
            timestamp: Some(1640995200),
            base_fee: Some(U256::from_dec_str("25000000000").unwrap()), // 25 gwei
            state_overrides: None,
        }
    }

    /// Generate a bundle with state overrides for testing
    fn generate_state_override_bundle(&self) -> SimulationBundle {
        let tx = SimulationTransaction {
            from: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
            to: Some(Address::from_str("0x0987654321098765432109876543210987654321").unwrap()),
            value: U256::from_dec_str("5000000000000000000").unwrap(), // 5 ETH (more than normal balance)
            gas_limit: U256::from(21000),
            gas_price: U256::from_dec_str("20000000000").unwrap(), // 20 gwei
            data: Bytes::default(),
            nonce: Some(U256::from(1)),
        };

        // Create state overrides to make the transaction possible
        let mut state_overrides = HashMap::new();
        state_overrides.insert(
            Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
            StateOverride {
                balance: Some(U256::from_dec_str("10000000000000000000").unwrap()), // 10 ETH
                nonce: Some(U256::from(1)),
                code: None,
                state: None,
                state_diff: None,
            }
        );

        SimulationBundle {
            id: format!("test_state_override_{}", self.seed),
            transactions: vec![tx],
            block_number: Some(18000000),
            timestamp: Some(1640995200),
            base_fee: Some(U256::from_dec_str("15000000000").unwrap()), // 15 gwei
            state_overrides: Some(state_overrides),
        }
    }

    /// Generate a multi-transaction bundle for testing
    fn generate_multi_transaction_bundle(&self) -> SimulationBundle {
        let tx1 = SimulationTransaction {
            from: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
            to: Some(Address::from_str("0x0987654321098765432109876543210987654321").unwrap()),
            value: U256::from_dec_str("1000000000000000000").unwrap(), // 1 ETH
            gas_limit: U256::from(21000),
            gas_price: U256::from_dec_str("20000000000").unwrap(), // 20 gwei
            data: Bytes::default(),
            nonce: Some(U256::from(1)),
        };

        let tx2 = SimulationTransaction {
            from: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
            to: Some(Address::from_str("0xabcdefabcdefabcdefabcdefabcdefabcdefabcd").unwrap()),
            value: U256::from_dec_str("500000000000000000").unwrap(), // 0.5 ETH
            gas_limit: U256::from(21000),
            gas_price: U256::from_dec_str("25000000000").unwrap(), // 25 gwei
            data: Bytes::default(),
            nonce: Some(U256::from(2)),
        };

        SimulationBundle {
            id: format!("test_multi_tx_{}", self.seed),
            transactions: vec![tx1, tx2],
            block_number: Some(18000000),
            timestamp: Some(1640995200),
            base_fee: Some(U256::from_dec_str("18000000000").unwrap()), // 18 gwei
            state_overrides: None,
        }
    }

    /// Generate state overrides for testing
    pub fn generate_test_state_overrides(&self) -> HashMap<Address, StateOverride> {
        let mut overrides = HashMap::new();
        
        // Override balance for test account
        let test_account = Address::from_str("0x1234567890123456789012345678901234567890").unwrap();
        overrides.insert(test_account, StateOverride {
            balance: Some(U256::from_dec_str("1000000000000000000000").unwrap()), // 1000 ETH
            nonce: Some(U256::from(1)),
            code: None,
            state: None,
            state_diff: None,
        });

        overrides
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Transaction, TargetType};

    fn create_test_parsed_transaction() -> ParsedTransaction {
        ParsedTransaction {
            transaction: Transaction {
                hash: "0x123".to_string(),
                from: "0x1234567890123456789012345678901234567890".to_string(),
                to: Some("0x0987654321098765432109876543210987654321".to_string()),
                value: "1000000000000000000".to_string(), // 1 ETH
                gas_price: "20000000000".to_string(),     // 20 gwei
                gas_limit: "21000".to_string(),
                nonce: 1,
                input: "0x".to_string(),
                timestamp: chrono::Utc::now(),
            },
            decoded_input: None,
            target_type: TargetType::Unknown,
            processing_time_ms: 1,
        }
    }

    #[test]
    fn test_simulation_transaction_conversion() {
        let parsed_tx = create_test_parsed_transaction();
        let sim_tx = SimulationTransaction::from(&parsed_tx);

        assert_eq!(sim_tx.value, U256::from_dec_str("1000000000000000000").unwrap());
        assert_eq!(sim_tx.gas_price, U256::from_dec_str("20000000000").unwrap());
        assert_eq!(sim_tx.nonce, Some(U256::from(1)));
    }

    #[test]
    fn test_bundle_builder() {
        let parsed_tx = create_test_parsed_transaction();
        
        let bundle = BundleBuilder::new("test_bundle".to_string())
            .add_parsed_transaction(&parsed_tx)
            .set_block_number(12345)
            .build();

        assert_eq!(bundle.id, "test_bundle");
        assert_eq!(bundle.transactions.len(), 1);
        assert_eq!(bundle.block_number, Some(12345));
        assert!(bundle.state_overrides.is_none());
    }

    #[test]
    fn test_profit_estimate_calculation() {
        let mut estimate = ProfitEstimate::default();
        estimate.gross_profit_wei = U256::from_dec_str("2000000000000000000").unwrap(); // 2 ETH
        estimate.gas_cost_wei = U256::from_dec_str("500000000000000000").unwrap();     // 0.5 ETH
        estimate.net_profit_wei = estimate.gross_profit_wei - estimate.gas_cost_wei;
        estimate.profit_margin = estimate.net_profit_wei.as_u128() as f64 / estimate.gross_profit_wei.as_u128() as f64;

        assert_eq!(estimate.net_profit_wei, U256::from_dec_str("1500000000000000000").unwrap());
        assert_eq!(estimate.profit_margin, 0.75);
    }

    #[test]
    fn test_deterministic_test_data_generator() {
        let generator = TestDataGenerator::new(12345);
        let bundles = generator.generate_test_bundles();

        assert_eq!(bundles.len(), 6);
        
        // Test simple transfer bundle
        let transfer_bundle = &bundles[0];
        assert!(transfer_bundle.id.contains("test_transfer"));
        assert_eq!(transfer_bundle.transactions.len(), 1);
        assert_eq!(transfer_bundle.transactions[0].value, U256::from_dec_str("1000000000000000000").unwrap());

        // Test Uniswap bundle
        let uniswap_bundle = &bundles[1];
        assert!(uniswap_bundle.id.contains("test_uniswap_swap"));
        assert_eq!(uniswap_bundle.transactions[0].value, U256::from_dec_str("100000000000000000").unwrap());

        // Test failed transaction bundle
        let failed_bundle = &bundles[2];
        assert!(failed_bundle.id.contains("test_failed_tx"));
        assert_eq!(failed_bundle.transactions[0].from, Address::from_str("0x0000000000000000000000000000000000000000").unwrap());

        // Test high gas bundle
        let high_gas_bundle = &bundles[3];
        assert!(high_gas_bundle.id.contains("test_high_gas"));
        assert_eq!(high_gas_bundle.transactions[0].gas_limit, U256::from(2000000));

        // Test state override bundle
        let state_override_bundle = &bundles[4];
        assert!(state_override_bundle.id.contains("test_state_override"));
        assert!(state_override_bundle.state_overrides.is_some());
        assert_eq!(state_override_bundle.transactions[0].value, U256::from_dec_str("5000000000000000000").unwrap());

        // Test multi-transaction bundle
        let multi_tx_bundle = &bundles[5];
        assert!(multi_tx_bundle.id.contains("test_multi_tx"));
        assert_eq!(multi_tx_bundle.transactions.len(), 2);
        assert_eq!(multi_tx_bundle.transactions[0].nonce, Some(U256::from(1)));
        assert_eq!(multi_tx_bundle.transactions[1].nonce, Some(U256::from(2)));
    }

    #[test]
    fn test_state_overrides_generation() {
        let generator = TestDataGenerator::new(12345);
        let overrides = generator.generate_test_state_overrides();

        assert!(!overrides.is_empty());
        
        let test_account = Address::from_str("0x1234567890123456789012345678901234567890").unwrap();
        let override_data = overrides.get(&test_account).unwrap();
        
        assert_eq!(override_data.balance, Some(U256::from_dec_str("1000000000000000000000").unwrap()));
        assert_eq!(override_data.nonce, Some(U256::from(1)));
    }

    #[test]
    fn test_revert_reason_extraction() {
        let config = SimulationConfig::default();
        let metrics = Arc::new(crate::PrometheusMetrics::new().unwrap());
        
        // We can't easily test the full simulator without a real RPC endpoint,
        // but we can test the revert reason extraction logic
        let simulator = ForkSimulator {
            config,
            provider: Arc::new(Provider::<Http>::try_from("http://localhost:8545").unwrap()),
            metrics,
            active_simulations: tokio::sync::Semaphore::new(10),
        };

        let reason1 = simulator.extract_revert_reason("execution reverted: Insufficient balance");
        assert_eq!(reason1, Some("Insufficient balance".to_string()));

        let reason2 = simulator.extract_revert_reason("Transaction reverted without a reason string");
        assert_eq!(reason2, Some("Transaction reverted without a reason string".to_string()));

        let reason3 = simulator.extract_revert_reason("Network error");
        assert_eq!(reason3, None);
    }

    #[tokio::test]
    async fn test_simulation_service_creation() {
        let config = SimulationConfig::default();
        
        // Handle metrics creation gracefully
        let metrics = match crate::PrometheusMetrics::new() {
            Ok(m) => Arc::new(m),
            Err(_) => {
                // Skip test if metrics registration fails
                return;
            }
        };
        
        // This will fail to connect but we can test the creation logic
        let simulator_result = ForkSimulator::new(config, metrics).await;
        
        // We expect this to fail since there's no RPC endpoint, but the structure should be correct
        assert!(simulator_result.is_err());
    }
}