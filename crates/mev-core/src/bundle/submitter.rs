//! Bundle submission system with multiple paths and race condition handling

use super::types::Bundle;
use super::SubmissionResult;
use anyhow::{anyhow, Result};
use ethers::types::U256;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::{sleep, Instant};

/// Bundle submission configuration
#[derive(Debug, Clone)]
pub struct SubmissionConfig {
    /// RPC endpoint for direct eth_sendRawTransaction
    pub rpc_endpoint: String,
    /// Bundle RPC endpoint (flashbots-style)
    pub bundle_rpc_endpoint: Option<String>,
    /// Maximum number of resubmission attempts
    pub max_resubmissions: u32,
    /// Gas bump percentage for resubmissions (e.g., 1.1 = 10% increase)
    pub gas_bump_multiplier: f64,
    /// Timeout for submission attempts
    pub submission_timeout: Duration,
    /// Interval between resubmission attempts
    pub resubmission_interval: Duration,
}

impl Default for SubmissionConfig {
    fn default() -> Self {
        Self {
            rpc_endpoint: "http://localhost:8545".to_string(),
            bundle_rpc_endpoint: None,
            max_resubmissions: 3,
            gas_bump_multiplier: 1.1,
            submission_timeout: Duration::from_secs(30),
            resubmission_interval: Duration::from_secs(5),
        }
    }
}

/// Bundle submitter with multiple submission paths
pub struct BundleSubmitter {
    /// HTTP client for RPC calls
    client: Client,
    /// Submission configuration
    config: SubmissionConfig,
    /// Active submissions tracking
    active_submissions: Arc<RwLock<HashMap<String, SubmissionTracker>>>,
}

/// Tracks the state of an active bundle submission
#[derive(Debug, Clone)]
struct SubmissionTracker {
    #[allow(dead_code)]
    bundle_id: String,
    submission_attempts: u32,
    last_submission_time: Instant,
    current_gas_price: U256,
    original_bundle: Bundle,
    #[allow(dead_code)]
    submission_hashes: Vec<String>,
}

/// Submission path enumeration
#[derive(Debug, Clone)]
pub enum SubmissionPath {
    /// Direct eth_sendRawTransaction for each transaction
    Direct,
    /// Bundle RPC (flashbots-style) for atomic submission
    BundleRpc,
    /// Race condition: submit via both paths simultaneously
    Race,
}

/// Submission statistics
#[derive(Debug, Clone)]
pub struct SubmissionStats {
    pub total_submissions: u64,
    pub successful_submissions: u64,
    pub failed_submissions: u64,
    pub average_inclusion_time: Duration,
    pub gas_bump_count: u64,
}

impl BundleSubmitter {
    /// Create a new bundle submitter
    pub fn new(config: SubmissionConfig) -> Self {
        let client = Client::builder()
            .timeout(config.submission_timeout)
            .build()
            .expect("Failed to create HTTP client");
        
        Self {
            client,
            config,
            active_submissions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Submit a bundle using the specified path
    pub async fn submit_bundle(
        &self,
        bundle: &Bundle,
        path: SubmissionPath,
    ) -> Result<SubmissionResult> {
        let submission_id = format!("sub_{}", uuid::Uuid::new_v4());
        
        match path {
            SubmissionPath::Direct => self.submit_direct(bundle, &submission_id).await,
            SubmissionPath::BundleRpc => self.submit_bundle_rpc(bundle, &submission_id).await,
            SubmissionPath::Race => self.submit_race(bundle, &submission_id).await,
        }
    }
    
    /// Submit bundle via direct eth_sendRawTransaction
    async fn submit_direct(&self, bundle: &Bundle, submission_id: &str) -> Result<SubmissionResult> {
        let mut submission_hashes = Vec::new();
        
        // Submit all transactions in order
        for tx in bundle.all_transactions() {
            let tx_hash = self.send_raw_transaction(&tx.raw_transaction).await?;
            submission_hashes.push(tx_hash);
        }
        
        // Track the submission
        let tracker = SubmissionTracker {
            bundle_id: bundle.id.clone(),
            submission_attempts: 1,
            last_submission_time: Instant::now(),
            current_gas_price: bundle.max_gas_price,
            original_bundle: bundle.clone(),
            submission_hashes: submission_hashes.clone(),
        };
        
        let mut active = self.active_submissions.write().await;
        active.insert(submission_id.to_string(), tracker);
        
        Ok(SubmissionResult {
            bundle_id: bundle.id.clone(),
            submission_hash: submission_hashes.join(","),
            submitted_at: chrono::Utc::now(),
            gas_price: bundle.max_gas_price,
            estimated_inclusion_block: bundle.target_block,
        })
    }
    
    /// Submit bundle via bundle RPC (flashbots-style)
    async fn submit_bundle_rpc(&self, bundle: &Bundle, submission_id: &str) -> Result<SubmissionResult> {
        let bundle_endpoint = self.config.bundle_rpc_endpoint
            .as_ref()
            .ok_or_else(|| anyhow!("Bundle RPC endpoint not configured"))?;
        
        // Prepare bundle for submission
        let bundle_data = self.prepare_bundle_data(bundle)?;
        
        let response = self.client
            .post(bundle_endpoint)
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "eth_sendBundle",
                "params": [bundle_data],
                "id": 1
            }))
            .send()
            .await?;
        
        let result: Value = response.json().await?;
        
        if let Some(error) = result.get("error") {
            return Err(anyhow!("Bundle RPC error: {}", error));
        }
        
        let bundle_hash = result["result"]
            .as_str()
            .ok_or_else(|| anyhow!("Invalid bundle RPC response"))?
            .to_string();
        
        // Track the submission
        let tracker = SubmissionTracker {
            bundle_id: bundle.id.clone(),
            submission_attempts: 1,
            last_submission_time: Instant::now(),
            current_gas_price: bundle.max_gas_price,
            original_bundle: bundle.clone(),
            submission_hashes: vec![bundle_hash.clone()],
        };
        
        let mut active = self.active_submissions.write().await;
        active.insert(submission_id.to_string(), tracker);
        
        Ok(SubmissionResult {
            bundle_id: bundle.id.clone(),
            submission_hash: bundle_hash,
            submitted_at: chrono::Utc::now(),
            gas_price: bundle.max_gas_price,
            estimated_inclusion_block: bundle.target_block,
        })
    }
    
    /// Submit bundle via race condition (both paths simultaneously)
    async fn submit_race(&self, bundle: &Bundle, submission_id: &str) -> Result<SubmissionResult> {
        let direct_id = format!("{}_direct", submission_id);
        let bundle_id = format!("{}_bundle", submission_id);
        
        let direct_future = self.submit_direct(bundle, &direct_id);
        let bundle_rpc_future = self.submit_bundle_rpc(bundle, &bundle_id);
        
        // Race both submissions
        tokio::select! {
            direct_result = direct_future => {
                tracing::info!("Direct submission won the race for bundle {}", bundle.id);
                direct_result
            }
            bundle_result = bundle_rpc_future => {
                tracing::info!("Bundle RPC submission won the race for bundle {}", bundle.id);
                bundle_result
            }
        }
    }
    
    /// Resubmit a bundle with gas bump
    pub async fn resubmit_with_gas_bump(&self, submission_id: &str) -> Result<SubmissionResult> {
        let mut active = self.active_submissions.write().await;
        let tracker = active.get_mut(submission_id)
            .ok_or_else(|| anyhow!("Submission not found: {}", submission_id))?;
        
        if tracker.submission_attempts >= self.config.max_resubmissions {
            return Err(anyhow!("Maximum resubmission attempts reached"));
        }
        
        // Calculate new gas price with bump
        let new_gas_price = U256::from(
            (tracker.current_gas_price.as_u128() as f64 * self.config.gas_bump_multiplier) as u128
        );
        
        // Create new bundle with bumped gas price
        let mut new_bundle = tracker.original_bundle.clone();
        new_bundle.max_gas_price = new_gas_price;
        
        // Update all transaction gas prices
        for tx in &mut new_bundle.pre_transactions {
            tx.max_fee_per_gas = new_gas_price;
        }
        new_bundle.target_transaction.max_fee_per_gas = new_gas_price;
        for tx in &mut new_bundle.post_transactions {
            tx.max_fee_per_gas = new_gas_price;
        }
        
        // Update tracker
        tracker.submission_attempts += 1;
        tracker.last_submission_time = Instant::now();
        tracker.current_gas_price = new_gas_price;
        
        drop(active); // Release lock before recursive call
        
        // Resubmit with direct path (faster for gas bumps)
        self.submit_direct(&new_bundle, submission_id).await
    }
    
    /// Send raw transaction via RPC
    async fn send_raw_transaction(&self, raw_tx: &ethers::types::Bytes) -> Result<String> {
        let response = self.client
            .post(&self.config.rpc_endpoint)
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "eth_sendRawTransaction",
                "params": [format!("0x{}", hex::encode(raw_tx))],
                "id": 1
            }))
            .send()
            .await?;
        
        let result: Value = response.json().await?;
        
        if let Some(error) = result.get("error") {
            return Err(anyhow!("RPC error: {}", error));
        }
        
        result["result"]
            .as_str()
            .ok_or_else(|| anyhow!("Invalid RPC response"))
            .map(|s| s.to_string())
    }
    
    /// Prepare bundle data for bundle RPC submission
    fn prepare_bundle_data(&self, bundle: &Bundle) -> Result<Value> {
        let mut transactions = Vec::new();
        
        // Add all transactions in order
        for tx in bundle.all_transactions() {
            transactions.push(format!("0x{}", hex::encode(&tx.raw_transaction)));
        }
        
        Ok(json!({
            "txs": transactions,
            "blockNumber": format!("0x{:x}", bundle.target_block),
            "minTimestamp": 0,
            "maxTimestamp": bundle.created_at.timestamp() + 30, // 30 second window
        }))
    }
    
    /// Get submission statistics
    pub async fn get_stats(&self) -> SubmissionStats {
        let active = self.active_submissions.read().await;
        
        let total_submissions = active.len() as u64;
        let gas_bump_count = active.values()
            .map(|t| t.submission_attempts.saturating_sub(1) as u64)
            .sum();
        
        // In a real implementation, these would be tracked over time
        SubmissionStats {
            total_submissions,
            successful_submissions: 0, // Would be tracked separately
            failed_submissions: 0,     // Would be tracked separately
            average_inclusion_time: Duration::from_secs(12), // Placeholder
            gas_bump_count,
        }
    }
    
    /// Clean up old submissions
    pub async fn cleanup_old_submissions(&self, max_age: Duration) {
        let mut active = self.active_submissions.write().await;
        let now = Instant::now();
        
        active.retain(|_, tracker| {
            now.duration_since(tracker.last_submission_time) < max_age
        });
    }
    
    /// Get active submission count
    pub async fn get_active_submission_count(&self) -> usize {
        let active = self.active_submissions.read().await;
        active.len()
    }
}

/// Fast resubmission logic with automatic gas bumping
pub struct FastResubmitter {
    submitter: Arc<BundleSubmitter>,
    resubmission_interval: Duration,
    max_gas_price: U256,
}

impl FastResubmitter {
    /// Create a new fast resubmitter
    pub fn new(submitter: Arc<BundleSubmitter>, max_gas_price: U256) -> Self {
        Self {
            submitter,
            resubmission_interval: Duration::from_secs(3),
            max_gas_price,
        }
    }
    
    /// Start automatic resubmission for a bundle
    pub async fn start_resubmission(&self, submission_id: String) -> Result<()> {
        let submitter = self.submitter.clone();
        let interval = self.resubmission_interval;
        let max_gas = self.max_gas_price;
        
        tokio::spawn(async move {
            let mut attempts = 0;
            
            loop {
                sleep(interval).await;
                attempts += 1;
                
                // Check if submission still exists
                let active_count = submitter.get_active_submission_count().await;
                if active_count == 0 {
                    break;
                }
                
                // Attempt resubmission with gas bump
                match submitter.resubmit_with_gas_bump(&submission_id).await {
                    Ok(result) => {
                        tracing::info!(
                            "Resubmitted bundle {} with gas price {}",
                            result.bundle_id,
                            result.gas_price
                        );
                        
                        // Stop if gas price exceeds maximum
                        if result.gas_price > max_gas {
                            tracing::warn!("Gas price exceeded maximum, stopping resubmission");
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!("Resubmission failed: {}", e);
                        break;
                    }
                }
                
                // Limit total attempts
                if attempts >= 10 {
                    tracing::warn!("Maximum resubmission attempts reached");
                    break;
                }
            }
        });
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::types::*;
    use ethers::types::{Address, Bytes, H256};
    
    fn create_test_bundle() -> Bundle {
        Bundle {
            id: "test_bundle".to_string(),
            pre_transactions: vec![],
            target_transaction: SignedTransaction {
                hash: H256::random(),
                from: Address::random(),
                to: Some(Address::random()),
                value: U256::from(1000000000000000000u64),
                gas_limit: U256::from(21000),
                max_fee_per_gas: U256::from(50000000000u64),
                max_priority_fee_per_gas: U256::from(2000000000u64),
                nonce: U256::from(1),
                data: Bytes::default(),
                chain_id: 998,
                raw_transaction: Bytes::from(vec![0x01, 0x02, 0x03]), // Dummy data
                transaction_type: 2,
            },
            post_transactions: vec![],
            target_block: 1000,
            max_gas_price: U256::from(100000000000u64),
            estimated_profit: U256::from(50000000000000000u64),
            created_at: chrono::Utc::now(),
            metadata: std::collections::HashMap::new(),
        }
    }
    
    #[tokio::test]
    async fn test_submission_config() {
        let config = SubmissionConfig::default();
        assert_eq!(config.max_resubmissions, 3);
        assert_eq!(config.gas_bump_multiplier, 1.1);
    }
    
    #[tokio::test]
    async fn test_bundle_submitter_creation() {
        let config = SubmissionConfig::default();
        let submitter = BundleSubmitter::new(config);
        
        let stats = submitter.get_stats().await;
        assert_eq!(stats.total_submissions, 0);
    }
    
    #[tokio::test]
    async fn test_prepare_bundle_data() {
        let config = SubmissionConfig::default();
        let submitter = BundleSubmitter::new(config);
        let bundle = create_test_bundle();
        
        let bundle_data = submitter.prepare_bundle_data(&bundle).unwrap();
        
        assert!(bundle_data["txs"].is_array());
        assert!(bundle_data["blockNumber"].is_string());
    }
    
    #[tokio::test]
    async fn test_gas_bump_calculation() {
        let config = SubmissionConfig {
            gas_bump_multiplier: 1.2,
            ..Default::default()
        };
        
        let original_gas = U256::from(100_000_000_000u64); // 100 gwei
        let bumped_gas = U256::from(
            (original_gas.as_u128() as f64 * config.gas_bump_multiplier) as u128
        );
        
        assert_eq!(bumped_gas, U256::from(120_000_000_000u64)); // 120 gwei
    }
    
    #[tokio::test]
    async fn test_submission_cleanup() {
        let config = SubmissionConfig::default();
        let submitter = BundleSubmitter::new(config);
        
        // Initially no active submissions
        assert_eq!(submitter.get_active_submission_count().await, 0);
        
        // Cleanup should not panic
        submitter.cleanup_old_submissions(Duration::from_secs(60)).await;
    }
}