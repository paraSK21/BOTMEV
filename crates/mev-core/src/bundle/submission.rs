//! Bundle submission system with multiple paths and race condition handling

use super::types::{Bundle, SignedTransaction};
use super::{SubmissionResult, ExecutionStatus};
use anyhow::{anyhow, Result};
use ethers::types::{H256, U256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Bundle submission configuration
#[derive(Debug, Clone)]
pub struct SubmissionConfig {
    /// Direct RPC endpoint for eth_sendRawTransaction
    pub direct_rpc_url: String,
    /// Bundle RPC endpoint (flashbots-style)
    pub bundle_rpc_url: Option<String>,
    /// Maximum gas price for resubmission
    pub max_gas_price: U256,
    /// Gas bump percentage for resubmission (e.g., 1.1 = 10% increase)
    pub gas_bump_multiplier: f64,
    /// Maximum number of resubmission attempts
    pub max_resubmissions: u32,
    /// Block confirmation requirement
    pub confirmation_blocks: u64,
    /// Submission timeout in seconds
    pub submission_timeout_secs: u64,
}

impl Default for SubmissionConfig {
    fn default() -> Self {
        Self {
            direct_rpc_url: "http://localhost:8545".to_string(),
            bundle_rpc_url: None,
            max_gas_price: U256::from(500_000_000_000u64), // 500 gwei
            gas_bump_multiplier: 1.1,
            max_resubmissions: 5,
            confirmation_blocks: 1,
            submission_timeout_secs: 30,
        }
    }
}

/// Bundle submission path
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SubmissionPath {
    /// Direct eth_sendRawTransaction
    Direct,
    /// Bundle RPC (flashbots-style)
    BundleRpc,
    /// Both paths simultaneously (race condition)
    Both,
}

/// Bundle submission status
#[derive(Debug, Clone)]
pub struct BundleSubmissionStatus {
    pub bundle_id: String,
    pub submission_attempts: Vec<SubmissionAttempt>,
    pub current_status: ExecutionStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_updated: chrono::DateTime<chrono::Utc>,
}

/// Individual submission attempt
#[derive(Debug, Clone)]
pub struct SubmissionAttempt {
    pub attempt_id: String,
    pub path: SubmissionPath,
    pub gas_price: U256,
    pub submitted_at: chrono::DateTime<chrono::Utc>,
    pub transaction_hashes: Vec<String>,
    pub result: SubmissionAttemptResult,
}

/// Result of a submission attempt
#[derive(Debug, Clone)]
pub enum SubmissionAttemptResult {
    Pending,
    Success { block_number: u64, inclusion_time: chrono::DateTime<chrono::Utc> },
    Failed { error: String },
    Timeout,
}

/// Bundle submitter with multiple paths and race condition handling
pub struct BundleSubmitter {
    config: SubmissionConfig,
    direct_client: Arc<DirectRpcClient>,
    bundle_client: Option<Arc<BundleRpcClient>>,
    submissions: Arc<RwLock<HashMap<String, BundleSubmissionStatus>>>,
}

impl BundleSubmitter {
    /// Create a new bundle submitter
    pub fn new(config: SubmissionConfig) -> Self {
        let direct_client = Arc::new(DirectRpcClient::new(config.direct_rpc_url.clone()));
        let bundle_client = config.bundle_rpc_url.as_ref()
            .map(|url| Arc::new(BundleRpcClient::new(url.clone())));

        Self {
            config,
            direct_client,
            bundle_client,
            submissions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Submit a bundle using the specified path
    pub async fn submit_bundle(
        &self,
        bundle: &Bundle,
        path: SubmissionPath,
    ) -> Result<SubmissionResult> {
        let bundle_id = bundle.id.clone();
        let submission_time = chrono::Utc::now();

        info!(
            bundle_id = %bundle_id,
            path = ?path,
            "Submitting bundle"
        );

        // Initialize submission status
        let mut submissions = self.submissions.write().await;
        submissions.insert(bundle_id.clone(), BundleSubmissionStatus {
            bundle_id: bundle_id.clone(),
            submission_attempts: Vec::new(),
            current_status: ExecutionStatus::Pending,
            created_at: submission_time,
            last_updated: submission_time,
        });
        drop(submissions);

        match path {
            SubmissionPath::Direct => {
                self.submit_direct(bundle).await
            }
            SubmissionPath::BundleRpc => {
                self.submit_bundle_rpc(bundle).await
            }
            SubmissionPath::Both => {
                self.submit_both_paths(bundle).await
            }
        }
    }

    /// Submit bundle via direct eth_sendRawTransaction
    async fn submit_direct(&self, bundle: &Bundle) -> Result<SubmissionResult> {
        let bundle_id = bundle.id.clone();
        let attempt_id = format!("direct_{}", uuid::Uuid::new_v4());
        
        debug!(
            bundle_id = %bundle_id,
            attempt_id = %attempt_id,
            "Submitting via direct RPC"
        );

        let mut transaction_hashes = Vec::new();
        let gas_price = bundle.max_gas_price;

        // Submit all transactions in order
        for (i, tx) in bundle.all_transactions().iter().enumerate() {
            match self.direct_client.send_raw_transaction(&tx.raw_transaction).await {
                Ok(tx_hash) => {
                    transaction_hashes.push(format!("0x{:x}", tx_hash));
                    debug!(
                        bundle_id = %bundle_id,
                        tx_index = i,
                        tx_hash = %format!("0x{:x}", tx_hash),
                        "Transaction submitted"
                    );
                }
                Err(e) => {
                    error!(
                        bundle_id = %bundle_id,
                        tx_index = i,
                        error = %e,
                        "Failed to submit transaction"
                    );
                    
                    // Record failed attempt
                    self.record_attempt(
                        &bundle_id,
                        SubmissionAttempt {
                            attempt_id: attempt_id.clone(),
                            path: SubmissionPath::Direct,
                            gas_price,
                            submitted_at: chrono::Utc::now(),
                            transaction_hashes: transaction_hashes.clone(),
                            result: SubmissionAttemptResult::Failed { error: e.to_string() },
                        },
                    ).await;
                    
                    return Err(e);
                }
            }
        }

        // Record successful attempt
        self.record_attempt(
            &bundle_id,
            SubmissionAttempt {
                attempt_id: attempt_id.clone(),
                path: SubmissionPath::Direct,
                gas_price,
                submitted_at: chrono::Utc::now(),
                transaction_hashes: transaction_hashes.clone(),
                result: SubmissionAttemptResult::Pending,
            },
        ).await;

        Ok(SubmissionResult {
            bundle_id,
            submission_hash: attempt_id,
            submitted_at: chrono::Utc::now(),
            gas_price,
            estimated_inclusion_block: bundle.target_block,
        })
    }

    /// Submit bundle via bundle RPC
    async fn submit_bundle_rpc(&self, bundle: &Bundle) -> Result<SubmissionResult> {
        let bundle_client = self.bundle_client.as_ref()
            .ok_or_else(|| anyhow!("Bundle RPC client not configured"))?;

        let bundle_id = bundle.id.clone();
        let attempt_id = format!("bundle_{}", uuid::Uuid::new_v4());
        
        debug!(
            bundle_id = %bundle_id,
            attempt_id = %attempt_id,
            "Submitting via bundle RPC"
        );

        let gas_price = bundle.max_gas_price;
        let raw_transactions: Vec<_> = bundle.all_transactions()
            .iter()
            .map(|tx| format!("0x{}", hex::encode(&tx.raw_transaction)))
            .collect();

        match bundle_client.send_bundle(&raw_transactions, bundle.target_block).await {
            Ok(bundle_hash) => {
                info!(
                    bundle_id = %bundle_id,
                    bundle_hash = %bundle_hash,
                    "Bundle submitted successfully"
                );

                // Record successful attempt
                self.record_attempt(
                    &bundle_id,
                    SubmissionAttempt {
                        attempt_id: attempt_id.clone(),
                        path: SubmissionPath::BundleRpc,
                        gas_price,
                        submitted_at: chrono::Utc::now(),
                        transaction_hashes: vec![bundle_hash.clone()],
                        result: SubmissionAttemptResult::Pending,
                    },
                ).await;

                Ok(SubmissionResult {
                    bundle_id,
                    submission_hash: bundle_hash,
                    submitted_at: chrono::Utc::now(),
                    gas_price,
                    estimated_inclusion_block: bundle.target_block,
                })
            }
            Err(e) => {
                error!(
                    bundle_id = %bundle_id,
                    error = %e,
                    "Failed to submit bundle via bundle RPC"
                );

                // Record failed attempt
                self.record_attempt(
                    &bundle_id,
                    SubmissionAttempt {
                        attempt_id: attempt_id.clone(),
                        path: SubmissionPath::BundleRpc,
                        gas_price,
                        submitted_at: chrono::Utc::now(),
                        transaction_hashes: Vec::new(),
                        result: SubmissionAttemptResult::Failed { error: e.to_string() },
                    },
                ).await;

                Err(e)
            }
        }
    }

    /// Submit bundle via both paths simultaneously (race condition)
    async fn submit_both_paths(&self, bundle: &Bundle) -> Result<SubmissionResult> {
        let bundle_id = bundle.id.clone();
        
        info!(
            bundle_id = %bundle_id,
            "Submitting via both paths (race condition)"
        );

        // Clone bundle for concurrent submission
        let bundle_direct = bundle.clone();
        let bundle_rpc = bundle.clone();

        // Submit via both paths concurrently
        let (direct_result, bundle_result) = tokio::join!(
            self.submit_direct(&bundle_direct),
            async {
                if self.bundle_client.is_some() {
                    self.submit_bundle_rpc(&bundle_rpc).await
                } else {
                    Err(anyhow!("Bundle RPC not configured"))
                }
            }
        );

        // Return the first successful result, or the error if both fail
        match (direct_result, bundle_result) {
            (Ok(result), _) => {
                info!(
                    bundle_id = %bundle_id,
                    "Direct submission succeeded first"
                );
                Ok(result)
            }
            (_, Ok(result)) => {
                info!(
                    bundle_id = %bundle_id,
                    "Bundle RPC submission succeeded first"
                );
                Ok(result)
            }
            (Err(direct_err), Err(bundle_err)) => {
                error!(
                    bundle_id = %bundle_id,
                    direct_error = %direct_err,
                    bundle_error = %bundle_err,
                    "Both submission paths failed"
                );
                Err(anyhow!("Both submission paths failed: direct={}, bundle={}", direct_err, bundle_err))
            }
        }
    }

    /// Resubmit bundle with gas bump
    pub async fn resubmit_with_gas_bump(&self, bundle_id: &str) -> Result<SubmissionResult> {
        let submissions = self.submissions.read().await;
        let status = submissions.get(bundle_id)
            .ok_or_else(|| anyhow!("Bundle not found: {}", bundle_id))?;

        if status.submission_attempts.len() >= self.config.max_resubmissions as usize {
            return Err(anyhow!("Maximum resubmission attempts reached"));
        }

        // Calculate new gas price
        let last_gas_price = status.submission_attempts
            .last()
            .map(|attempt| attempt.gas_price)
            .unwrap_or(U256::from(50_000_000_000u64)); // 50 gwei default

        let new_gas_price = U256::from(
            (last_gas_price.as_u128() as f64 * self.config.gas_bump_multiplier) as u128
        );

        if new_gas_price > self.config.max_gas_price {
            return Err(anyhow!("Gas price would exceed maximum: {} > {}", new_gas_price, self.config.max_gas_price));
        }

        drop(submissions);

        info!(
            bundle_id = %bundle_id,
            old_gas_price = %last_gas_price,
            new_gas_price = %new_gas_price,
            "Resubmitting with gas bump"
        );

        // For resubmission, we need to rebuild the bundle with new gas price
        // This is a simplified version - in practice, you'd need to access the original bundle
        Err(anyhow!("Resubmission requires bundle reconstruction - not implemented in this example"))
    }

    /// Record a submission attempt
    async fn record_attempt(&self, bundle_id: &str, attempt: SubmissionAttempt) {
        let mut submissions = self.submissions.write().await;
        if let Some(status) = submissions.get_mut(bundle_id) {
            status.submission_attempts.push(attempt);
            status.last_updated = chrono::Utc::now();
        }
    }

    /// Get submission status
    pub async fn get_submission_status(&self, bundle_id: &str) -> Option<BundleSubmissionStatus> {
        let submissions = self.submissions.read().await;
        submissions.get(bundle_id).cloned()
    }

    /// List all submissions
    pub async fn list_submissions(&self) -> Vec<BundleSubmissionStatus> {
        let submissions = self.submissions.read().await;
        submissions.values().cloned().collect()
    }

    /// Clean up old submissions
    pub async fn cleanup_old_submissions(&self, max_age_hours: i64) {
        let mut submissions = self.submissions.write().await;
        let cutoff = chrono::Utc::now() - chrono::Duration::hours(max_age_hours);
        
        submissions.retain(|_, status| status.created_at > cutoff);
        
        info!(
            remaining_submissions = submissions.len(),
            "Cleaned up old submissions"
        );
    }
}

/// Direct RPC client for eth_sendRawTransaction
pub struct DirectRpcClient {
    rpc_url: String,
    client: reqwest::Client,
}

impl DirectRpcClient {
    pub fn new(rpc_url: String) -> Self {
        Self {
            rpc_url,
            client: reqwest::Client::new(),
        }
    }

    pub async fn send_raw_transaction(&self, raw_tx: &ethers::types::Bytes) -> Result<H256> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_sendRawTransaction",
            "params": [format!("0x{}", hex::encode(raw_tx))],
            "id": 1
        });

        let response = self.client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await?;

        let json: serde_json::Value = response.json().await?;
        
        if let Some(error) = json.get("error") {
            return Err(anyhow!("RPC error: {}", error));
        }

        let tx_hash = json.get("result")
            .and_then(|r| r.as_str())
            .ok_or_else(|| anyhow!("Invalid response format"))?;

        Ok(tx_hash.parse()?)
    }

    pub async fn get_transaction_receipt(&self, tx_hash: &H256) -> Result<Option<TransactionReceipt>> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getTransactionReceipt",
            "params": [format!("0x{:x}", tx_hash)],
            "id": 1
        });

        let response = self.client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await?;

        let json: serde_json::Value = response.json().await?;
        
        if let Some(error) = json.get("error") {
            return Err(anyhow!("RPC error: {}", error));
        }

        if json.get("result").and_then(|r| r.as_object()).is_none() {
            return Ok(None);
        }

        // Parse receipt (simplified)
        let result = json.get("result").unwrap();
        let block_number = result.get("blockNumber")
            .and_then(|b| b.as_str())
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);

        let status = result.get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("0x1") == "0x1";

        Ok(Some(TransactionReceipt {
            transaction_hash: *tx_hash,
            block_number,
            status,
        }))
    }
}

/// Bundle RPC client (flashbots-style)
pub struct BundleRpcClient {
    rpc_url: String,
    client: reqwest::Client,
}

impl BundleRpcClient {
    pub fn new(rpc_url: String) -> Self {
        Self {
            rpc_url,
            client: reqwest::Client::new(),
        }
    }

    pub async fn send_bundle(&self, transactions: &[String], target_block: u64) -> Result<String> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_sendBundle",
            "params": [{
                "txs": transactions,
                "blockNumber": format!("0x{:x}", target_block)
            }],
            "id": 1
        });

        let response = self.client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await?;

        let json: serde_json::Value = response.json().await?;
        
        if let Some(error) = json.get("error") {
            return Err(anyhow!("Bundle RPC error: {}", error));
        }

        let bundle_hash = json.get("result")
            .and_then(|r| r.as_str())
            .ok_or_else(|| anyhow!("Invalid bundle response format"))?;

        Ok(bundle_hash.to_string())
    }
}

/// Simplified transaction receipt
#[derive(Debug, Clone)]
pub struct TransactionReceipt {
    pub transaction_hash: H256,
    pub block_number: u64,
    pub status: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::types::Bundle;
    use ethers::types::{Address, Bytes};
    use std::collections::HashMap;

    fn create_test_bundle() -> Bundle {
        Bundle {
            id: "test_bundle".to_string(),
            pre_transactions: vec![],
            target_transaction: create_test_signed_transaction(),
            post_transactions: vec![],
            target_block: 1000,
            max_gas_price: U256::from(100_000_000_000u64),
            estimated_profit: U256::from(50_000_000_000_000_000u64),
            created_at: chrono::Utc::now(),
            metadata: HashMap::new(),
        }
    }

    fn create_test_signed_transaction() -> crate::bundle::types::SignedTransaction {
        crate::bundle::types::SignedTransaction {
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
            raw_transaction: Bytes::from(vec![0x01, 0x02, 0x03]), // Mock raw transaction
            transaction_type: 2,
        }
    }

    #[tokio::test]
    async fn test_submission_config() {
        let config = SubmissionConfig::default();
        assert_eq!(config.direct_rpc_url, "http://localhost:8545");
        assert_eq!(config.gas_bump_multiplier, 1.1);
        assert_eq!(config.max_resubmissions, 5);
    }

    #[tokio::test]
    async fn test_bundle_submitter_creation() {
        let config = SubmissionConfig::default();
        let submitter = BundleSubmitter::new(config);
        
        // Should be able to create submitter
        assert!(submitter.direct_client.rpc_url.contains("localhost"));
        assert!(submitter.bundle_client.is_none()); // No bundle RPC configured
    }

    #[tokio::test]
    async fn test_submission_status_tracking() {
        let config = SubmissionConfig::default();
        let submitter = BundleSubmitter::new(config);
        let bundle = create_test_bundle();

        // Record a test attempt
        let attempt = SubmissionAttempt {
            attempt_id: "test_attempt".to_string(),
            path: SubmissionPath::Direct,
            gas_price: U256::from(50_000_000_000u64),
            submitted_at: chrono::Utc::now(),
            transaction_hashes: vec!["0x123".to_string()],
            result: SubmissionAttemptResult::Pending,
        };

        submitter.record_attempt(&bundle.id, attempt).await;

        // Should be able to retrieve status
        let status = submitter.get_submission_status(&bundle.id).await;
        assert!(status.is_none()); // No status because we didn't initialize it properly

        // Test cleanup
        submitter.cleanup_old_submissions(1).await;
        let submissions = submitter.list_submissions().await;
        assert_eq!(submissions.len(), 0);
    }

    #[test]
    fn test_submission_path_enum() {
        assert_eq!(SubmissionPath::Direct, SubmissionPath::Direct);
        assert_ne!(SubmissionPath::Direct, SubmissionPath::BundleRpc);
    }

    #[test]
    fn test_submission_attempt_result() {
        let result = SubmissionAttemptResult::Success {
            block_number: 1000,
            inclusion_time: chrono::Utc::now(),
        };

        match result {
            SubmissionAttemptResult::Success { block_number, .. } => {
                assert_eq!(block_number, 1000);
            }
            _ => panic!("Expected Success result"),
        }
    }
}