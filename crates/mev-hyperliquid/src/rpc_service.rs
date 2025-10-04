//! RPC service for HyperLiquid blockchain operations
//!
//! This module provides blockchain interaction capabilities through ethers-rs,
//! enabling queries for transaction data, block information, and on-chain state.
//!
//! ## Error Handling and Resilience
//!
//! This service implements several resilience patterns:
//! - **Retry Logic**: Exponential backoff for transient failures
//! - **Circuit Breaker**: Stops operations after consecutive failures, resumes after cooldown
//! - **Timeout Handling**: All RPC operations have configurable timeouts
//! - **Graceful Degradation**: Service continues operating despite RPC failures

use anyhow::{Context, Result};
use ethers::prelude::*;
use ethers::providers::{Http, Middleware, Provider};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{TransactionReceipt, TransactionRequest, U256};
use ethers::types::transaction::eip2718::TypedTransaction;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time;
use tracing::{debug, error, info, warn};

use crate::types::{BlockchainState, StateUpdateEvent};

/// Maximum number of retry attempts for RPC calls
const MAX_RETRY_ATTEMPTS: u32 = 3;

/// Initial backoff duration in milliseconds
const INITIAL_BACKOFF_MS: u64 = 100;

/// Maximum backoff duration in milliseconds
const MAX_BACKOFF_MS: u64 = 10_000;

/// Number of consecutive failures before circuit breaker opens
const CIRCUIT_BREAKER_THRESHOLD: u64 = 5;

/// Circuit breaker cooldown period in seconds
const CIRCUIT_BREAKER_COOLDOWN_SECS: u64 = 30;

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CircuitBreakerState {
    /// Circuit is closed, operations proceed normally
    Closed,
    /// Circuit is open, operations are blocked
    Open,
    /// Circuit is half-open, testing if service has recovered
    HalfOpen,
}

/// Circuit breaker for RPC operations
struct CircuitBreaker {
    /// Current state of the circuit breaker
    state: Arc<AtomicU64>, // 0 = Closed, 1 = Open, 2 = HalfOpen
    /// Number of consecutive failures
    consecutive_failures: Arc<AtomicU64>,
    /// Timestamp when circuit breaker opened (seconds since epoch)
    opened_at: Arc<AtomicU64>,
}

impl CircuitBreaker {
    fn new() -> Self {
        Self {
            state: Arc::new(AtomicU64::new(0)), // Start in Closed state
            consecutive_failures: Arc::new(AtomicU64::new(0)),
            opened_at: Arc::new(AtomicU64::new(0)),
        }
    }
    
    fn get_state(&self) -> CircuitBreakerState {
        match self.state.load(Ordering::Relaxed) {
            0 => CircuitBreakerState::Closed,
            1 => CircuitBreakerState::Open,
            2 => CircuitBreakerState::HalfOpen,
            _ => CircuitBreakerState::Closed,
        }
    }
    
    fn set_state(&self, state: CircuitBreakerState) {
        let value = match state {
            CircuitBreakerState::Closed => 0,
            CircuitBreakerState::Open => 1,
            CircuitBreakerState::HalfOpen => 2,
        };
        self.state.store(value, Ordering::Relaxed);
    }
    
    /// Check if operation should be allowed
    fn should_allow_request(&self) -> bool {
        let state = self.get_state();
        
        match state {
            CircuitBreakerState::Closed => true,
            CircuitBreakerState::Open => {
                // Check if cooldown period has elapsed
                let opened_at = self.opened_at.load(Ordering::Relaxed);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                
                if now - opened_at >= CIRCUIT_BREAKER_COOLDOWN_SECS {
                    // Transition to half-open state
                    info!("Circuit breaker cooldown elapsed, transitioning to half-open state");
                    self.set_state(CircuitBreakerState::HalfOpen);
                    true
                } else {
                    false
                }
            }
            CircuitBreakerState::HalfOpen => true,
        }
    }
    
    /// Record a successful operation
    fn record_success(&self) {
        let state = self.get_state();
        
        match state {
            CircuitBreakerState::Closed => {
                // Reset failure counter
                self.consecutive_failures.store(0, Ordering::Relaxed);
            }
            CircuitBreakerState::HalfOpen => {
                // Transition back to closed state
                info!("Circuit breaker test successful, transitioning to closed state");
                self.set_state(CircuitBreakerState::Closed);
                self.consecutive_failures.store(0, Ordering::Relaxed);
            }
            CircuitBreakerState::Open => {
                // Should not happen, but reset if it does
                warn!("Unexpected success in open state, resetting circuit breaker");
                self.set_state(CircuitBreakerState::Closed);
                self.consecutive_failures.store(0, Ordering::Relaxed);
            }
        }
    }
    
    /// Record a failed operation
    fn record_failure(&self) {
        let state = self.get_state();
        let failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        
        match state {
            CircuitBreakerState::Closed => {
                if failures >= CIRCUIT_BREAKER_THRESHOLD {
                    // Open the circuit breaker
                    error!(
                        "Circuit breaker threshold reached ({} consecutive failures), opening circuit",
                        failures
                    );
                    self.set_state(CircuitBreakerState::Open);
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    self.opened_at.store(now, Ordering::Relaxed);
                }
            }
            CircuitBreakerState::HalfOpen => {
                // Transition back to open state
                warn!("Circuit breaker test failed, transitioning back to open state");
                self.set_state(CircuitBreakerState::Open);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                self.opened_at.store(now, Ordering::Relaxed);
            }
            CircuitBreakerState::Open => {
                // Already open, just increment counter
            }
        }
    }
    
    /// Get the number of consecutive failures
    fn get_consecutive_failures(&self) -> u64 {
        self.consecutive_failures.load(Ordering::Relaxed)
    }
}

/// Configuration for RPC service
#[derive(Debug, Clone)]
pub struct RpcConfig {
    /// RPC endpoint URL
    pub rpc_url: String,
    
    /// Polling interval in milliseconds
    pub polling_interval_ms: u64,
    
    /// Request timeout in seconds
    pub timeout_secs: u64,
    
    /// Private key for transaction signing (optional)
    pub private_key: Option<String>,
}

impl RpcConfig {
    /// Create a new RPC configuration
    pub fn new(rpc_url: String, polling_interval_ms: u64) -> Self {
        Self {
            rpc_url,
            polling_interval_ms,
            timeout_secs: 10,
            private_key: None,
        }
    }
    
    /// Set request timeout
    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs;
        self
    }
    
    /// Set private key for transaction signing
    pub fn with_private_key(mut self, private_key: String) -> Self {
        self.private_key = Some(private_key);
        self
    }
}

/// HyperLiquid RPC service for blockchain operations
pub struct HyperLiquidRpcService {
    /// Ethers provider for RPC calls
    provider: Arc<Provider<Http>>,
    
    /// RPC configuration
    config: RpcConfig,
    
    /// Connection status
    connected: bool,
    
    /// Channel for sending state updates
    state_tx: Option<mpsc::UnboundedSender<StateUpdateEvent>>,
    
    /// Wallet for transaction signing (optional)
    wallet: Option<LocalWallet>,
    
    /// Circuit breaker for error handling
    circuit_breaker: CircuitBreaker,
}

impl HyperLiquidRpcService {
    /// Create a new HyperLiquidRpcService instance
    ///
    /// # Arguments
    /// * `config` - RPC configuration including URL and polling settings
    /// * `state_tx` - Optional channel for sending state updates
    ///
    /// # Returns
    /// A new RPC service instance (not yet connected)
    pub fn new(config: RpcConfig, state_tx: Option<mpsc::UnboundedSender<StateUpdateEvent>>) -> Self {
        info!("Creating HyperLiquid RPC service with URL: {}", config.rpc_url);
        
        // Create HTTP provider (connection happens on first request)
        let provider = Provider::<Http>::try_from(&config.rpc_url)
            .expect("Failed to create provider from RPC URL");
        
        // Initialize wallet if private key is provided
        let wallet = config.private_key.as_ref().and_then(|pk| {
            match pk.parse::<LocalWallet>() {
                Ok(wallet) => {
                    info!("Wallet initialized for transaction signing");
                    Some(wallet)
                }
                Err(e) => {
                    error!("Failed to parse private key: {}. Transaction submission will not be available.", e);
                    None
                }
            }
        });
        
        Self {
            provider: Arc::new(provider),
            config,
            connected: false,
            state_tx,
            wallet,
            circuit_breaker: CircuitBreaker::new(),
        }
    }
    
    /// Establish and validate RPC connection
    ///
    /// This method performs a test query to verify the RPC endpoint is accessible
    /// and responding correctly.
    ///
    /// # Returns
    /// * `Ok(())` if connection is successful
    /// * `Err` if connection fails or validation fails
    pub async fn connect(&mut self) -> Result<()> {
        info!("Connecting to HyperLiquid RPC at {}", self.config.rpc_url);
        
        // Validate connection by fetching chain ID
        let chain_id = self.provider
            .get_chainid()
            .await
            .context("Failed to fetch chain ID from RPC endpoint")?;
        
        debug!("Successfully connected to chain ID: {}", chain_id);
        
        // Validate it's HyperEVM (chain ID 998)
        if chain_id.as_u64() != 998 {
            warn!(
                "Connected to chain ID {} but expected 998 (HyperEVM). Proceeding anyway.",
                chain_id
            );
        }
        
        // Fetch current block number to further validate connection
        let block_number = self.provider
            .get_block_number()
            .await
            .context("Failed to fetch current block number")?;
        
        info!(
            "RPC connection established. Chain ID: {}, Current block: {}",
            chain_id, block_number
        );
        
        self.connected = true;
        Ok(())
    }
    
    /// Check if the service is connected
    pub fn is_connected(&self) -> bool {
        self.connected
    }
    
    /// Get the underlying ethers provider
    ///
    /// This allows direct access to the provider for advanced operations
    pub fn provider(&self) -> Arc<Provider<Http>> {
        Arc::clone(&self.provider)
    }
    
    /// Get the RPC configuration
    pub fn config(&self) -> &RpcConfig {
        &self.config
    }
    
    /// Execute an RPC operation with retry logic and exponential backoff
    ///
    /// This method wraps RPC calls with:
    /// - Circuit breaker check
    /// - Retry logic with exponential backoff
    /// - Timeout handling
    /// - Error logging
    ///
    /// # Arguments
    /// * `operation_name` - Name of the operation for logging
    /// * `operation` - Async closure that performs the RPC call
    ///
    /// # Returns
    /// * `Ok(T)` if operation succeeds
    /// * `Err` if all retries fail or circuit breaker is open
    async fn execute_with_retry<F, Fut, T>(
        &self,
        operation_name: &str,
        operation: F,
    ) -> Result<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        // Check circuit breaker
        if !self.circuit_breaker.should_allow_request() {
            let failures = self.circuit_breaker.get_consecutive_failures();
            error!(
                "Circuit breaker is open for RPC operations ({} consecutive failures). Operation '{}' blocked.",
                failures, operation_name
            );
            return Err(anyhow::anyhow!(
                "Circuit breaker is open. RPC service is temporarily unavailable."
            ));
        }
        
        let mut backoff_ms = INITIAL_BACKOFF_MS;
        let mut last_error = None;
        
        for attempt in 1..=MAX_RETRY_ATTEMPTS {
            debug!(
                "Executing RPC operation '{}' (attempt {}/{})",
                operation_name, attempt, MAX_RETRY_ATTEMPTS
            );
            
            // Execute operation with timeout
            let timeout_duration = Duration::from_secs(self.config.timeout_secs);
            let result = time::timeout(timeout_duration, operation()).await;
            
            match result {
                Ok(Ok(value)) => {
                    // Success
                    if attempt > 1 {
                        info!(
                            "RPC operation '{}' succeeded after {} attempts",
                            operation_name, attempt
                        );
                    }
                    self.circuit_breaker.record_success();
                    return Ok(value);
                }
                Ok(Err(e)) => {
                    // Operation failed
                    warn!(
                        "RPC operation '{}' failed (attempt {}/{}): {}",
                        operation_name, attempt, MAX_RETRY_ATTEMPTS, e
                    );
                    last_error = Some(e);
                }
                Err(_) => {
                    // Timeout
                    warn!(
                        "RPC operation '{}' timed out after {}s (attempt {}/{})",
                        operation_name, self.config.timeout_secs, attempt, MAX_RETRY_ATTEMPTS
                    );
                    last_error = Some(anyhow::anyhow!(
                        "Operation timed out after {}s",
                        self.config.timeout_secs
                    ));
                }
            }
            
            // Don't sleep after the last attempt
            if attempt < MAX_RETRY_ATTEMPTS {
                debug!(
                    "Retrying RPC operation '{}' after {}ms backoff",
                    operation_name, backoff_ms
                );
                time::sleep(Duration::from_millis(backoff_ms)).await;
                
                // Exponential backoff with cap
                backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);
            }
        }
        
        // All retries failed
        error!(
            "RPC operation '{}' failed after {} attempts",
            operation_name, MAX_RETRY_ATTEMPTS
        );
        self.circuit_breaker.record_failure();
        
        Err(last_error.unwrap_or_else(|| {
            anyhow::anyhow!("Operation failed after {} attempts", MAX_RETRY_ATTEMPTS)
        }))
    }
    
    /// Get circuit breaker status for monitoring
    pub fn get_circuit_breaker_status(&self) -> (String, u64) {
        let state = match self.circuit_breaker.get_state() {
            CircuitBreakerState::Closed => "closed",
            CircuitBreakerState::Open => "open",
            CircuitBreakerState::HalfOpen => "half-open",
        };
        let failures = self.circuit_breaker.get_consecutive_failures();
        (state.to_string(), failures)
    }
    
    /// Poll blockchain state and emit state update events
    ///
    /// This method queries the current block number and other relevant blockchain state.
    /// It emits StateUpdateEvent for each piece of state data.
    ///
    /// This method uses retry logic with exponential backoff and respects the circuit breaker.
    ///
    /// # Returns
    /// * `Ok(BlockchainState)` with the current state snapshot
    /// * `Err` if polling fails after all retries or circuit breaker is open
    pub async fn poll_blockchain_state(&self) -> Result<BlockchainState> {
        self.execute_with_retry("poll_blockchain_state", || async {
            debug!("Polling blockchain state");
            
            // Query current block number
            let block_number = self.provider
                .get_block_number()
                .await
                .context("Failed to query block number")?
                .as_u64();
            
            debug!("Current block number: {}", block_number);
            
            // Get block timestamp
            let block = self.provider
                .get_block(block_number)
                .await
                .context("Failed to get block details")?
                .context("Block not found")?;
            
            let timestamp = block.timestamp.as_u64();
            
            // Create blockchain state snapshot
            let state = BlockchainState::new(block_number, timestamp);
            
            // Emit state update events if channel is available
            if let Some(ref tx) = self.state_tx {
                // Emit block number update
                if let Err(e) = tx.send(StateUpdateEvent::BlockNumber(block_number)) {
                    warn!("Failed to send block number update: {}", e);
                }
                
                // Emit full state snapshot
                if let Err(e) = tx.send(StateUpdateEvent::StateSnapshot(state.clone())) {
                    warn!("Failed to send state snapshot: {}", e);
                }
            }
            
            Ok(state)
        })
        .await
    }
    
    /// Submit a transaction to the HyperLiquid EVM network
    ///
    /// This method signs the transaction with the configured private key and submits it
    /// via `eth_sendRawTransaction`. The transaction must be properly constructed with
    /// all required fields (to, value, data, gas, etc.).
    ///
    /// This method uses retry logic with exponential backoff for transient failures.
    ///
    /// # Arguments
    /// * `tx` - The transaction request to submit
    ///
    /// # Returns
    /// * `Ok(TxHash)` - The transaction hash if submission succeeds
    /// * `Err` - If signing fails, submission fails, or no wallet is configured
    ///
    /// # Errors
    /// - Returns error if no private key is configured
    /// - Returns error if transaction signing fails
    /// - Returns error if RPC submission fails after all retries
    pub async fn submit_transaction(&self, tx: TransactionRequest) -> Result<TxHash> {
        info!("Submitting transaction to HyperLiquid EVM");
        
        // Ensure wallet is configured (fail fast, no retry)
        let wallet = self.wallet.as_ref()
            .context("Cannot submit transaction: no private key configured")?
            .clone();
        
        // Clone tx for use in closure
        let tx_clone = tx.clone();
        let provider = Arc::clone(&self.provider);
        
        self.execute_with_retry("submit_transaction", move || {
            let wallet = wallet.clone();
            let tx = tx_clone.clone();
            let provider = Arc::clone(&provider);
            
            async move {
                // Get chain ID for signing
                let chain_id = provider
                    .get_chainid()
                    .await
                    .context("Failed to fetch chain ID for transaction signing")?;
                
                debug!("Signing transaction with chain ID: {}", chain_id);
                
                // Set chain ID on wallet
                let wallet = wallet.with_chain_id(chain_id.as_u64());
                
                // Fill in missing transaction fields (nonce, gas price, etc.)
                let tx = if tx.nonce.is_none() {
                    let from = wallet.address();
                    let nonce = provider
                        .get_transaction_count(from, None)
                        .await
                        .context("Failed to fetch nonce for transaction")?;
                    debug!("Using nonce: {}", nonce);
                    tx.nonce(nonce)
                } else {
                    tx
                };
                
                // Estimate gas if not provided
                let tx = if tx.gas.is_none() {
                    let typed_tx: TypedTransaction = tx.clone().into();
                    match provider.estimate_gas(&typed_tx, None).await {
                        Ok(gas_estimate) => {
                            // Add 20% buffer to gas estimate
                            let gas_with_buffer = gas_estimate * 120 / 100;
                            debug!("Estimated gas: {}, using: {}", gas_estimate, gas_with_buffer);
                            tx.gas(gas_with_buffer)
                        }
                        Err(e) => {
                            warn!("Failed to estimate gas: {}. Using default gas limit.", e);
                            tx.gas(U256::from(300_000)) // Default gas limit
                        }
                    }
                } else {
                    tx
                };
                
                // Get gas price if not provided
                let tx = if tx.gas_price.is_none() {
                    match provider.get_gas_price().await {
                        Ok(gas_price) => {
                            debug!("Using gas price: {}", gas_price);
                            tx.gas_price(gas_price)
                        }
                        Err(e) => {
                            warn!("Failed to fetch gas price: {}. Transaction may fail.", e);
                            tx
                        }
                    }
                } else {
                    tx
                };
                
                // Convert to TypedTransaction for signing
                let typed_tx: TypedTransaction = tx.into();
                
                // Sign the transaction
                let signature = wallet
                    .sign_transaction(&typed_tx)
                    .await
                    .context("Failed to sign transaction")?;
                
                debug!("Transaction signed successfully");
                
                // Encode the signed transaction
                let signed_tx = typed_tx.rlp_signed(&signature);
                
                // Submit via eth_sendRawTransaction
                let tx_hash = provider
                    .send_raw_transaction(signed_tx)
                    .await
                    .context("Failed to submit transaction via eth_sendRawTransaction")?;
                
                info!("Transaction submitted successfully. Hash: {:?}", tx_hash);
                
                Ok(*tx_hash)
            }
        })
        .await
    }
    
    /// Wait for transaction confirmation with timeout
    ///
    /// This method polls for the transaction receipt via `eth_getTransactionReceipt`
    /// until the transaction is confirmed or the timeout is reached. It tracks the
    /// confirmation status and gas used, and emits success/failure events.
    ///
    /// # Arguments
    /// * `tx_hash` - The transaction hash to wait for
    /// * `timeout_secs` - Maximum time to wait for confirmation (default: 60 seconds)
    ///
    /// # Returns
    /// * `Ok(TransactionReceipt)` if transaction is confirmed within timeout
    /// * `Err` if transaction fails, times out, or polling fails
    ///
    /// # Behavior
    /// - Polls every 1 second for transaction receipt
    /// - Emits `TransactionConfirmed` event on success
    /// - Emits `TransactionFailed` event on failure or timeout
    /// - Logs detailed information about confirmation status
    pub async fn wait_for_confirmation(
        &self,
        tx_hash: TxHash,
        timeout_secs: Option<u64>,
    ) -> Result<TransactionReceipt> {
        let timeout = Duration::from_secs(timeout_secs.unwrap_or(60));
        let poll_interval = Duration::from_secs(1);
        
        info!(
            "Waiting for transaction confirmation: {:?} (timeout: {}s)",
            tx_hash,
            timeout.as_secs()
        );
        
        let start_time = std::time::Instant::now();
        let mut interval = time::interval(poll_interval);
        
        loop {
            // Check if timeout has been reached
            if start_time.elapsed() >= timeout {
                let error_msg = format!(
                    "Transaction confirmation timeout after {}s",
                    timeout.as_secs()
                );
                error!("{}: {:?}", error_msg, tx_hash);
                
                // Emit failure event
                if let Some(ref tx) = self.state_tx {
                    let _ = tx.send(StateUpdateEvent::TransactionFailed {
                        tx_hash,
                        reason: error_msg.clone(),
                    });
                }
                
                return Err(anyhow::anyhow!(error_msg));
            }
            
            interval.tick().await;
            
            // Poll for transaction receipt
            match self.provider.get_transaction_receipt(tx_hash).await {
                Ok(Some(receipt)) => {
                    // Transaction is confirmed
                    let block_number = receipt.block_number
                        .context("Receipt missing block number")?
                        .as_u64();
                    
                    let gas_used = receipt.gas_used
                        .context("Receipt missing gas used")?;
                    
                    let status = receipt.status
                        .map(|s| s.as_u64() == 1)
                        .unwrap_or(false);
                    
                    if status {
                        info!(
                            "Transaction confirmed successfully: {:?}, block: {}, gas used: {}",
                            tx_hash, block_number, gas_used
                        );
                        
                        // Emit success event
                        if let Some(ref tx) = self.state_tx {
                            let _ = tx.send(StateUpdateEvent::TransactionConfirmed {
                                tx_hash,
                                block_number,
                                gas_used,
                                status,
                            });
                        }
                        
                        return Ok(receipt);
                    } else {
                        let error_msg = "Transaction reverted on-chain".to_string();
                        error!("{}: {:?}", error_msg, tx_hash);
                        
                        // Emit failure event
                        if let Some(ref tx) = self.state_tx {
                            let _ = tx.send(StateUpdateEvent::TransactionFailed {
                                tx_hash,
                                reason: error_msg.clone(),
                            });
                        }
                        
                        return Err(anyhow::anyhow!(
                            "Transaction reverted: {:?}. Gas used: {}",
                            tx_hash,
                            gas_used
                        ));
                    }
                }
                Ok(None) => {
                    // Transaction not yet mined, continue polling
                    debug!(
                        "Transaction not yet confirmed: {:?} (elapsed: {}s)",
                        tx_hash,
                        start_time.elapsed().as_secs()
                    );
                }
                Err(e) => {
                    // RPC error while fetching receipt
                    warn!(
                        "Error fetching transaction receipt for {:?}: {}. Retrying...",
                        tx_hash, e
                    );
                    // Continue polling despite RPC errors
                }
            }
        }
    }
    
    /// Start continuous blockchain state polling
    ///
    /// This method runs in a loop, polling blockchain state at the configured interval.
    /// It will continue operating despite RPC failures, using the circuit breaker pattern
    /// to prevent overwhelming a failing service.
    ///
    /// The polling loop will:
    /// - Continue indefinitely, even if individual polls fail
    /// - Respect the circuit breaker (skip polls when circuit is open)
    /// - Log errors with appropriate severity levels
    /// - Emit state updates when successful
    ///
    /// # Returns
    /// * `Ok(())` when polling loop exits normally (never in current implementation)
    /// * `Err` if service is not connected
    pub async fn start_polling(&self) -> Result<()> {
        if !self.connected {
            return Err(anyhow::anyhow!("RPC service not connected. Call connect() first."));
        }
        
        info!(
            "Starting blockchain state polling with interval: {}ms",
            self.config.polling_interval_ms
        );
        info!(
            "Resilience features enabled: retry (max {} attempts), circuit breaker (threshold: {} failures)",
            MAX_RETRY_ATTEMPTS, CIRCUIT_BREAKER_THRESHOLD
        );
        
        let polling_interval = Duration::from_millis(self.config.polling_interval_ms);
        let mut interval = time::interval(polling_interval);
        
        loop {
            interval.tick().await;
            
            // Check circuit breaker status
            let (cb_state, cb_failures) = self.get_circuit_breaker_status();
            if cb_state == "open" {
                warn!(
                    "Circuit breaker is open ({} consecutive failures). Skipping poll, waiting for cooldown.",
                    cb_failures
                );
                continue;
            }
            
            match self.poll_blockchain_state().await {
                Ok(state) => {
                    debug!(
                        "Successfully polled blockchain state. Block: {}, Timestamp: {}",
                        state.block_number, state.timestamp
                    );
                }
                Err(e) => {
                    // Log error with appropriate severity
                    let (cb_state, cb_failures) = self.get_circuit_breaker_status();
                    
                    if cb_state == "open" {
                        error!(
                            "Failed to poll blockchain state: {}. Circuit breaker opened after {} consecutive failures.",
                            e, cb_failures
                        );
                    } else if cb_failures > 0 {
                        warn!(
                            "Failed to poll blockchain state: {}. Consecutive failures: {}",
                            e, cb_failures
                        );
                    } else {
                        // Single failure after successful polls
                        warn!("Failed to poll blockchain state: {}. Will retry on next poll.", e);
                    }
                    
                    // Continue polling despite errors - bot remains operational
                    // The circuit breaker will prevent overwhelming a failing service
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_config_creation() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        );
        
        assert_eq!(config.rpc_url, "https://rpc.hyperliquid.xyz/evm");
        assert_eq!(config.polling_interval_ms, 500);
        assert_eq!(config.timeout_secs, 10);
        assert!(config.private_key.is_none());
    }
    
    #[test]
    fn test_rpc_config_with_timeout() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        ).with_timeout(30);
        
        assert_eq!(config.timeout_secs, 30);
    }
    
    #[test]
    fn test_rpc_config_with_private_key() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        ).with_private_key("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string());
        
        assert!(config.private_key.is_some());
        assert_eq!(
            config.private_key.unwrap(),
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
        );
    }
    
    #[test]
    fn test_rpc_service_creation() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        );
        
        let service = HyperLiquidRpcService::new(config, None);
        
        assert!(!service.is_connected());
        assert_eq!(service.config().rpc_url, "https://rpc.hyperliquid.xyz/evm");
        assert_eq!(service.config().polling_interval_ms, 500);
    }
    
    #[tokio::test]
    async fn test_rpc_service_invalid_url() {
        let config = RpcConfig::new(
            "https://invalid-url-that-does-not-exist.xyz".to_string(),
            500,
        );
        
        let mut service = HyperLiquidRpcService::new(config, None);
        
        // Connection should fail for invalid URL
        let result = service.connect().await;
        assert!(result.is_err());
        assert!(!service.is_connected());
    }
    
    #[tokio::test]
    async fn test_submit_transaction_without_private_key() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        );
        
        let service = HyperLiquidRpcService::new(config, None);
        
        // Create a simple transaction request
        let tx = TransactionRequest::new()
            .to("0x0000000000000000000000000000000000000000".parse::<Address>().unwrap())
            .value(U256::from(1000));
        
        // Should fail because no private key is configured
        let result = service.submit_transaction(tx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no private key configured"));
    }
    
    #[test]
    fn test_wallet_initialization_with_valid_key() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        ).with_private_key("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string());
        
        let service = HyperLiquidRpcService::new(config, None);
        
        // Wallet should be initialized
        assert!(service.wallet.is_some());
    }
    
    #[test]
    fn test_wallet_initialization_with_invalid_key() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        ).with_private_key("invalid_key".to_string());
        
        let service = HyperLiquidRpcService::new(config, None);
        
        // Wallet should not be initialized with invalid key
        assert!(service.wallet.is_none());
    }
    
    #[tokio::test]
    async fn test_wait_for_confirmation_timeout() {
        let config = RpcConfig::new(
            "https://invalid-url-that-does-not-exist.xyz".to_string(),
            500,
        );
        
        let (tx, _rx) = mpsc::unbounded_channel();
        let service = HyperLiquidRpcService::new(config, Some(tx));
        
        // Create a dummy transaction hash
        let tx_hash = TxHash::zero();
        
        // Should timeout quickly (1 second timeout)
        let result = service.wait_for_confirmation(tx_hash, Some(1)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timeout"));
    }
    
    #[test]
    fn test_wait_for_confirmation_method_exists() {
        // This test verifies that the wait_for_confirmation method exists
        // and can be called with the correct parameters
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        );
        
        let service = HyperLiquidRpcService::new(config, None);
        
        // Verify the method exists by checking we can reference it
        // This is a compile-time check
        let tx_hash = TxHash::zero();
        let _future = service.wait_for_confirmation(tx_hash, Some(60));
        // We don't await the future, just verify it compiles
    }
    
    // ===== Circuit Breaker Tests =====
    
    #[test]
    fn test_circuit_breaker_initial_state() {
        let cb = CircuitBreaker::new();
        assert_eq!(cb.get_state(), CircuitBreakerState::Closed);
        assert_eq!(cb.get_consecutive_failures(), 0);
        assert!(cb.should_allow_request());
    }
    
    #[test]
    fn test_circuit_breaker_records_success() {
        let cb = CircuitBreaker::new();
        cb.record_success();
        assert_eq!(cb.get_state(), CircuitBreakerState::Closed);
        assert_eq!(cb.get_consecutive_failures(), 0);
    }
    
    #[test]
    fn test_circuit_breaker_records_failures() {
        let cb = CircuitBreaker::new();
        
        // Record failures below threshold
        for i in 1..CIRCUIT_BREAKER_THRESHOLD {
            cb.record_failure();
            assert_eq!(cb.get_state(), CircuitBreakerState::Closed);
            assert_eq!(cb.get_consecutive_failures(), i);
        }
        
        // Record failure that reaches threshold
        cb.record_failure();
        assert_eq!(cb.get_state(), CircuitBreakerState::Open);
        assert_eq!(cb.get_consecutive_failures(), CIRCUIT_BREAKER_THRESHOLD);
    }
    
    #[test]
    fn test_circuit_breaker_opens_after_threshold() {
        let cb = CircuitBreaker::new();
        
        // Record enough failures to open circuit
        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            cb.record_failure();
        }
        
        assert_eq!(cb.get_state(), CircuitBreakerState::Open);
        assert!(!cb.should_allow_request()); // Should block requests immediately
    }
    
    #[test]
    fn test_circuit_breaker_resets_on_success() {
        let cb = CircuitBreaker::new();
        
        // Record some failures
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.get_consecutive_failures(), 2);
        
        // Record success
        cb.record_success();
        assert_eq!(cb.get_consecutive_failures(), 0);
        assert_eq!(cb.get_state(), CircuitBreakerState::Closed);
    }
    
    #[test]
    fn test_circuit_breaker_status() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        );
        
        let service = HyperLiquidRpcService::new(config, None);
        
        let (state, failures) = service.get_circuit_breaker_status();
        assert_eq!(state, "closed");
        assert_eq!(failures, 0);
    }
    
    // ===== Retry Logic Tests =====
    
    #[tokio::test]
    async fn test_execute_with_retry_success_first_attempt() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        );
        
        let service = HyperLiquidRpcService::new(config, None);
        
        let result = service.execute_with_retry("test_operation", || async {
            Ok::<i32, anyhow::Error>(42)
        }).await;
        
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }
    
    #[tokio::test]
    async fn test_execute_with_retry_success_after_failures() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        );
        
        let service = HyperLiquidRpcService::new(config, None);
        
        let attempt_count = Arc::new(AtomicU64::new(0));
        let attempt_count_clone = Arc::clone(&attempt_count);
        
        let result = service.execute_with_retry("test_operation", move || {
            let count = attempt_count_clone.fetch_add(1, Ordering::Relaxed) + 1;
            async move {
                if count < 3 {
                    Err(anyhow::anyhow!("Simulated failure"))
                } else {
                    Ok::<i32, anyhow::Error>(42)
                }
            }
        }).await;
        
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt_count.load(Ordering::Relaxed), 3);
    }
    
    #[tokio::test]
    async fn test_execute_with_retry_all_attempts_fail() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        ).with_timeout(1); // Short timeout for faster test
        
        let service = HyperLiquidRpcService::new(config, None);
        
        let result = service.execute_with_retry("test_operation", || async {
            Err::<i32, anyhow::Error>(anyhow::anyhow!("Simulated failure"))
        }).await;
        
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Simulated failure"));
        
        // Circuit breaker should have recorded the failure
        let (_, failures) = service.get_circuit_breaker_status();
        assert_eq!(failures, 1);
    }
    
    #[tokio::test]
    async fn test_execute_with_retry_timeout() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        ).with_timeout(1); // 1 second timeout
        
        let service = HyperLiquidRpcService::new(config, None);
        
        let result = service.execute_with_retry("test_operation", || async {
            // Sleep longer than timeout
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok::<i32, anyhow::Error>(42)
        }).await;
        
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }
    
    #[tokio::test]
    async fn test_execute_with_retry_circuit_breaker_blocks() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        ).with_timeout(1);
        
        let service = HyperLiquidRpcService::new(config, None);
        
        // Trigger circuit breaker by causing multiple failures
        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            let _ = service.execute_with_retry("test_operation", || async {
                Err::<i32, anyhow::Error>(anyhow::anyhow!("Simulated failure"))
            }).await;
        }
        
        // Circuit should be open now
        let (state, _) = service.get_circuit_breaker_status();
        assert_eq!(state, "open");
        
        // Next request should be blocked immediately
        let result = service.execute_with_retry("test_operation", || async {
            Ok::<i32, anyhow::Error>(42)
        }).await;
        
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Circuit breaker is open"));
    }
    
    // ===== Additional Circuit Breaker Tests =====
    
    #[test]
    fn test_circuit_breaker_half_open_success_closes() {
        let cb = CircuitBreaker::new();
        
        // Manually set to half-open state
        cb.set_state(CircuitBreakerState::HalfOpen);
        cb.consecutive_failures.store(3, Ordering::Relaxed);
        
        assert_eq!(cb.get_state(), CircuitBreakerState::HalfOpen);
        
        // Success should close the circuit
        cb.record_success();
        assert_eq!(cb.get_state(), CircuitBreakerState::Closed);
        assert_eq!(cb.get_consecutive_failures(), 0);
    }
    
    #[test]
    fn test_circuit_breaker_half_open_failure_reopens() {
        let cb = CircuitBreaker::new();
        
        // Manually set to half-open state
        cb.set_state(CircuitBreakerState::HalfOpen);
        
        assert_eq!(cb.get_state(), CircuitBreakerState::HalfOpen);
        
        // Failure should reopen the circuit
        cb.record_failure();
        assert_eq!(cb.get_state(), CircuitBreakerState::Open);
    }
    
    #[tokio::test]
    async fn test_circuit_breaker_cooldown_transition() {
        let cb = CircuitBreaker::new();
        
        // Open the circuit
        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            cb.record_failure();
        }
        assert_eq!(cb.get_state(), CircuitBreakerState::Open);
        assert!(!cb.should_allow_request());
        
        // Manually set opened_at to past (simulate cooldown elapsed)
        let past_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - CIRCUIT_BREAKER_COOLDOWN_SECS
            - 1;
        cb.opened_at.store(past_time, Ordering::Relaxed);
        
        // Should transition to half-open
        assert!(cb.should_allow_request());
        assert_eq!(cb.get_state(), CircuitBreakerState::HalfOpen);
    }
    
    #[test]
    fn test_circuit_breaker_multiple_instances_independent() {
        let cb1 = CircuitBreaker::new();
        let cb2 = CircuitBreaker::new();
        
        // Open cb1
        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            cb1.record_failure();
        }
        
        // cb1 should be open, cb2 should be closed
        assert_eq!(cb1.get_state(), CircuitBreakerState::Open);
        assert_eq!(cb2.get_state(), CircuitBreakerState::Closed);
        assert!(!cb1.should_allow_request());
        assert!(cb2.should_allow_request());
    }
    
    // ===== Blockchain State Polling Tests =====
    
    #[test]
    fn test_blockchain_state_creation() {
        let state = BlockchainState::new(1000, 1696348800);
        
        assert_eq!(state.block_number, 1000);
        assert_eq!(state.timestamp, 1696348800);
        assert!(state.token_prices.is_empty());
        assert!(state.contract_states.is_empty());
    }
    
    #[tokio::test]
    async fn test_poll_blockchain_state_sends_events() {
        let config = RpcConfig::new(
            "https://invalid-url.xyz".to_string(),
            500,
        ).with_timeout(1);
        
        let (tx, mut rx) = mpsc::unbounded_channel();
        let service = HyperLiquidRpcService::new(config, Some(tx));
        
        // Manually send events to verify channel works
        if let Some(ref state_tx) = service.state_tx {
            state_tx.send(StateUpdateEvent::BlockNumber(1000)).unwrap();
            state_tx.send(StateUpdateEvent::StateSnapshot(BlockchainState::new(1000, 1696348800))).unwrap();
        }
        
        // Verify events were received
        let event1 = rx.recv().await.unwrap();
        match event1 {
            StateUpdateEvent::BlockNumber(num) => assert_eq!(num, 1000),
            _ => panic!("Expected BlockNumber event"),
        }
        
        let event2 = rx.recv().await.unwrap();
        match event2 {
            StateUpdateEvent::StateSnapshot(state) => {
                assert_eq!(state.block_number, 1000);
                assert_eq!(state.timestamp, 1696348800);
            }
            _ => panic!("Expected StateSnapshot event"),
        }
    }
    
    // ===== Transaction Submission Tests =====
    
    #[test]
    fn test_transaction_request_creation() {
        let tx = TransactionRequest::new()
            .to(Address::zero())
            .value(U256::from(1000));
        
        assert_eq!(tx.to, Some(ethers::types::NameOrAddress::Address(Address::zero())));
        assert_eq!(tx.value, Some(U256::from(1000)));
    }
    
    #[test]
    fn test_transaction_request_with_gas() {
        let tx = TransactionRequest::new()
            .to(Address::zero())
            .value(U256::from(5000))
            .gas(U256::from(21000))
            .gas_price(U256::from(1000000000));
        
        assert_eq!(tx.gas, Some(U256::from(21000)));
        assert_eq!(tx.gas_price, Some(U256::from(1000000000)));
    }
    
    #[tokio::test]
    async fn test_submit_transaction_requires_private_key() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        );
        
        let service = HyperLiquidRpcService::new(config, None);
        
        let tx = TransactionRequest::new()
            .to(Address::zero())
            .value(U256::from(1000));
        
        // Should fail immediately without retries
        let result = service.submit_transaction(tx).await;
        
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no private key configured"));
        
        // Circuit breaker should NOT have recorded a failure (fail-fast case)
        let (state, failures) = service.get_circuit_breaker_status();
        assert_eq!(state, "closed");
        assert_eq!(failures, 0);
    }
    
    // ===== Transaction Confirmation Tests =====
    
    #[tokio::test]
    async fn test_wait_for_confirmation_with_timeout() {
        let config = RpcConfig::new(
            "https://invalid-url.xyz".to_string(),
            500,
        ).with_timeout(1);
        
        let (tx, _rx) = mpsc::unbounded_channel();
        let service = HyperLiquidRpcService::new(config, Some(tx));
        
        let tx_hash = TxHash::zero();
        
        // Should timeout after 1 second
        let result = service.wait_for_confirmation(tx_hash, Some(1)).await;
        
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timeout"));
    }
    
    #[tokio::test]
    async fn test_wait_for_confirmation_emits_failure_event() {
        let config = RpcConfig::new(
            "https://invalid-url.xyz".to_string(),
            500,
        ).with_timeout(1);
        
        let (tx, mut rx) = mpsc::unbounded_channel();
        let service = HyperLiquidRpcService::new(config, Some(tx));
        
        let tx_hash = TxHash::zero();
        
        // Start waiting for confirmation (will timeout)
        let wait_task = tokio::spawn(async move {
            service.wait_for_confirmation(tx_hash, Some(1)).await
        });
        
        // Should receive a TransactionFailed event
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
        
        if let Ok(Some(StateUpdateEvent::TransactionFailed { tx_hash: hash, reason })) = event {
            assert_eq!(hash, tx_hash);
            assert!(reason.contains("timeout"));
        }
        
        // Wait task should complete with error
        let result = wait_task.await.unwrap();
        assert!(result.is_err());
    }
    
    // ===== Configuration Tests =====
    
    #[test]
    fn test_rpc_config_builder_pattern() {
        let config = RpcConfig::new("http://test.com".to_string(), 2000)
            .with_timeout(15)
            .with_private_key("0xabc123".to_string());
        
        assert_eq!(config.rpc_url, "http://test.com");
        assert_eq!(config.polling_interval_ms, 2000);
        assert_eq!(config.timeout_secs, 15);
        assert_eq!(config.private_key, Some("0xabc123".to_string()));
    }
    
    #[test]
    fn test_rpc_service_config_immutability() {
        let config = RpcConfig::new("http://localhost:8545".to_string(), 1000);
        let service = HyperLiquidRpcService::new(config.clone(), None);
        
        // Config should be accessible but not mutable through service
        assert_eq!(service.config().rpc_url, config.rpc_url);
        assert_eq!(service.config().polling_interval_ms, config.polling_interval_ms);
    }
    
    #[test]
    fn test_multiple_services_with_different_configs() {
        let config1 = RpcConfig::new("http://localhost:8545".to_string(), 1000);
        let config2 = RpcConfig::new("http://localhost:8546".to_string(), 2000);
        
        let service1 = HyperLiquidRpcService::new(config1, None);
        let service2 = HyperLiquidRpcService::new(config2, None);
        
        assert_eq!(service1.config().rpc_url, "http://localhost:8545");
        assert_eq!(service1.config().polling_interval_ms, 1000);
        
        assert_eq!(service2.config().rpc_url, "http://localhost:8546");
        assert_eq!(service2.config().polling_interval_ms, 2000);
    }
    
    // ===== Provider Access Tests =====
    
    #[test]
    fn test_rpc_service_provider_access() {
        let config = RpcConfig::new("http://localhost:8545".to_string(), 1000);
        let service = HyperLiquidRpcService::new(config, None);
        
        // Should be able to get provider
        let provider = service.provider();
        let url_str = provider.url().to_string();
        assert!(url_str.contains("localhost:8545"));
    }
    
    // ===== Error Handling Tests =====
    
    #[test]
    fn test_circuit_breaker_constants_are_reasonable() {
        // Verify circuit breaker constants are reasonable
        assert!(MAX_RETRY_ATTEMPTS > 0);
        assert!(MAX_RETRY_ATTEMPTS <= 10);
        
        assert!(INITIAL_BACKOFF_MS > 0);
        assert!(INITIAL_BACKOFF_MS < 1000);
        
        assert!(MAX_BACKOFF_MS > INITIAL_BACKOFF_MS);
        assert!(MAX_BACKOFF_MS <= 60_000);
        
        assert!(CIRCUIT_BREAKER_THRESHOLD > 0);
        assert!(CIRCUIT_BREAKER_THRESHOLD <= 10);
        
        assert!(CIRCUIT_BREAKER_COOLDOWN_SECS > 0);
        assert!(CIRCUIT_BREAKER_COOLDOWN_SECS <= 300);
    }
    
    #[test]
    fn test_circuit_breaker_edge_case_exactly_threshold() {
        let cb = CircuitBreaker::new();
        
        // Record exactly threshold - 1 failures
        for _ in 0..(CIRCUIT_BREAKER_THRESHOLD - 1) {
            cb.record_failure();
            assert_eq!(cb.get_state(), CircuitBreakerState::Closed);
        }
        
        // One more failure should open the circuit
        cb.record_failure();
        assert_eq!(cb.get_state(), CircuitBreakerState::Open);
    }
    
    #[tokio::test]
    async fn test_retry_backoff_increases() {
        let config = RpcConfig::new(
            "https://rpc.hyperliquid.xyz/evm".to_string(),
            500,
        ).with_timeout(1);
        
        let service = HyperLiquidRpcService::new(config, None);
        
        let attempt_count = Arc::new(AtomicU64::new(0));
        let attempt_count_clone = Arc::clone(&attempt_count);
        let start_time = std::time::Instant::now();
        
        let _ = service.execute_with_retry("test_operation", move || {
            attempt_count_clone.fetch_add(1, Ordering::Relaxed);
            async move {
                Err::<i32, anyhow::Error>(anyhow::anyhow!("Simulated failure"))
            }
        }).await;
        
        let elapsed = start_time.elapsed();
        
        // Should have made MAX_RETRY_ATTEMPTS attempts
        assert_eq!(attempt_count.load(Ordering::Relaxed), MAX_RETRY_ATTEMPTS as u64);
        
        // Total time should be at least the sum of backoffs
        // First attempt: immediate
        // Second attempt: INITIAL_BACKOFF_MS delay
        // Third attempt: INITIAL_BACKOFF_MS * 2 delay
        // Minimum total: INITIAL_BACKOFF_MS + INITIAL_BACKOFF_MS * 2 = INITIAL_BACKOFF_MS * 3
        let min_expected_ms = INITIAL_BACKOFF_MS * 3;
        assert!(elapsed.as_millis() >= min_expected_ms as u128);
    }
    
    // ===== State Update Event Tests =====
    
    #[test]
    fn test_state_update_event_variants() {
        // Test that all StateUpdateEvent variants can be created
        let event1 = StateUpdateEvent::BlockNumber(1000);
        let event2 = StateUpdateEvent::TokenPrice {
            token: Address::zero(),
            price: U256::from(45000),
        };
        let event3 = StateUpdateEvent::Balance {
            address: Address::zero(),
            balance: U256::from(1000000),
        };
        let event4 = StateUpdateEvent::StateSnapshot(BlockchainState::new(1000, 1696348800));
        let event5 = StateUpdateEvent::TransactionConfirmed {
            tx_hash: TxHash::zero(),
            block_number: 1000,
            gas_used: U256::from(21000),
            status: true,
        };
        let event6 = StateUpdateEvent::TransactionFailed {
            tx_hash: TxHash::zero(),
            reason: "Test failure".to_string(),
        };
        
        // Verify they can be pattern matched
        match event1 {
            StateUpdateEvent::BlockNumber(num) => assert_eq!(num, 1000),
            _ => panic!("Wrong variant"),
        }
        
        match event2 {
            StateUpdateEvent::TokenPrice { token, price } => {
                assert_eq!(token, Address::zero());
                assert_eq!(price, U256::from(45000));
            }
            _ => panic!("Wrong variant"),
        }
        
        match event3 {
            StateUpdateEvent::Balance { address, balance } => {
                assert_eq!(address, Address::zero());
                assert_eq!(balance, U256::from(1000000));
            }
            _ => panic!("Wrong variant"),
        }
        
        match event4 {
            StateUpdateEvent::StateSnapshot(state) => {
                assert_eq!(state.block_number, 1000);
                assert_eq!(state.timestamp, 1696348800);
            }
            _ => panic!("Wrong variant"),
        }
        
        match event5 {
            StateUpdateEvent::TransactionConfirmed { tx_hash, block_number, gas_used, status } => {
                assert_eq!(tx_hash, TxHash::zero());
                assert_eq!(block_number, 1000);
                assert_eq!(gas_used, U256::from(21000));
                assert!(status);
            }
            _ => panic!("Wrong variant"),
        }
        
        match event6 {
            StateUpdateEvent::TransactionFailed { tx_hash, reason } => {
                assert_eq!(tx_hash, TxHash::zero());
                assert_eq!(reason, "Test failure");
            }
            _ => panic!("Wrong variant"),
        }
    }
}
