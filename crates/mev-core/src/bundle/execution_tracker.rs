//! Bundle execution tracking and monitoring system

use super::types::Bundle;
use super::{ExecutionStatus, SubmissionResult};
use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::interval;

/// Execution tracking configuration
#[derive(Debug, Clone)]
pub struct TrackerConfig {
    /// RPC endpoint for blockchain queries
    pub rpc_endpoint: String,
    /// Block confirmation count required
    pub confirmation_blocks: u64,
    /// Maximum tracking duration before timeout
    pub max_tracking_duration: Duration,
    /// Polling interval for block updates
    pub polling_interval: Duration,
    /// Enable detailed transaction analysis
    pub detailed_analysis: bool,
}

impl Default for TrackerConfig {
    fn default() -> Self {
        Self {
            rpc_endpoint: "http://localhost:8545".to_string(),
            confirmation_blocks: 3,
            max_tracking_duration: Duration::from_secs(300), // 5 minutes
            polling_interval: Duration::from_secs(2),
            detailed_analysis: true,
        }
    }
}

/// Bundle execution tracker
pub struct ExecutionTracker {
    /// HTTP client for RPC calls
    client: Client,
    /// Tracker configuration
    config: TrackerConfig,
    /// Tracked bundles
    tracked_bundles: Arc<RwLock<HashMap<String, TrackedBundle>>>,
    /// Latest block number
    latest_block: Arc<RwLock<u64>>,
}

/// Tracked bundle information
#[derive(Debug, Clone)]
struct TrackedBundle {
    bundle: Bundle,
    submission_result: SubmissionResult,
    tracking_start: Instant,
    status: ExecutionStatus,
    inclusion_attempts: u32,
    last_check_block: u64,
    transaction_receipts: HashMap<String, TransactionReceipt>,
}

/// Transaction receipt information
#[derive(Debug, Clone)]
struct TransactionReceipt {
    transaction_hash: String,
    block_number: u64,
    block_hash: String,
    transaction_index: u64,
    gas_used: u64,
    status: bool, // true = success, false = failed
    logs: Vec<LogEntry>,
}

/// Log entry from transaction receipt
#[derive(Debug, Clone)]
struct LogEntry {
    address: String,
    topics: Vec<String>,
    data: String,
}

/// Bundle inclusion metrics
#[derive(Debug, Clone)]
pub struct InclusionMetrics {
    pub bundle_id: String,
    pub submission_to_inclusion_latency: Duration,
    pub target_block: u64,
    pub actual_block: u64,
    pub block_position: Vec<u64>, // Position of each transaction in block
    pub gas_used: u64,
    pub gas_price_paid: u64,
    pub mev_extracted: Option<u64>,
}

/// Bundle ordering verification result
#[derive(Debug, Clone)]
pub struct OrderingVerification {
    pub is_correct_order: bool,
    pub expected_order: Vec<String>,
    pub actual_order: Vec<String>,
    pub position_differences: Vec<i32>,
}

impl ExecutionTracker {
    /// Create a new execution tracker
    pub fn new(config: TrackerConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");
        
        Self {
            client,
            config,
            tracked_bundles: Arc::new(RwLock::new(HashMap::new())),
            latest_block: Arc::new(RwLock::new(0)),
        }
    }
    
    /// Start tracking a submitted bundle
    pub async fn track_bundle(
        &self,
        bundle: Bundle,
        submission_result: SubmissionResult,
    ) -> Result<()> {
        let tracked = TrackedBundle {
            bundle: bundle.clone(),
            submission_result: submission_result.clone(),
            tracking_start: Instant::now(),
            status: ExecutionStatus::Pending,
            inclusion_attempts: 0,
            last_check_block: 0,
            transaction_receipts: HashMap::new(),
        };
        
        let mut bundles = self.tracked_bundles.write().await;
        bundles.insert(bundle.id.clone(), tracked);
        
        tracing::info!("Started tracking bundle: {}", bundle.id);
        Ok(())
    }
    
    /// Get current execution status of a bundle
    pub async fn get_execution_status(&self, bundle_id: &str) -> Result<ExecutionStatus> {
        let bundles = self.tracked_bundles.read().await;
        let tracked = bundles.get(bundle_id)
            .ok_or_else(|| anyhow!("Bundle not found: {}", bundle_id))?;
        
        Ok(tracked.status.clone())
    }
    
    /// Start the monitoring loop
    pub async fn start_monitoring(&self) -> Result<()> {
        let tracker = Arc::new(self);
        let mut block_interval = interval(self.config.polling_interval);
        
        loop {
            block_interval.tick().await;
            
            // Update latest block
            if let Ok(latest) = tracker.get_latest_block_number().await {
                let mut latest_block = tracker.latest_block.write().await;
                *latest_block = latest;
            }
            
            // Check all tracked bundles
            tracker.check_all_bundles().await;
            
            // Cleanup expired bundles
            tracker.cleanup_expired_bundles().await;
        }
    }
    
    /// Check all tracked bundles for inclusion
    async fn check_all_bundles(&self) {
        let bundle_ids: Vec<String> = {
            let bundles = self.tracked_bundles.read().await;
            bundles.keys().cloned().collect()
        };
        
        for bundle_id in bundle_ids {
            if let Err(e) = self.check_bundle_inclusion(&bundle_id).await {
                tracing::error!("Error checking bundle {}: {}", bundle_id, e);
            }
        }
    }
    
    /// Check if a specific bundle has been included
    async fn check_bundle_inclusion(&self, bundle_id: &str) -> Result<()> {
        let latest_block = *self.latest_block.read().await;
        
        let mut bundles = self.tracked_bundles.write().await;
        let tracked = bundles.get_mut(bundle_id)
            .ok_or_else(|| anyhow!("Bundle not found: {}", bundle_id))?;
        
        // Skip if already processed or expired
        match tracked.status {
            ExecutionStatus::Included { .. } | ExecutionStatus::Failed { .. } | ExecutionStatus::Expired => {
                return Ok(());
            }
            ExecutionStatus::Pending => {}
        }
        
        // Check for timeout
        if tracked.tracking_start.elapsed() > self.config.max_tracking_duration {
            tracked.status = ExecutionStatus::Expired;
            tracing::warn!("Bundle {} tracking expired", bundle_id);
            return Ok(());
        }
        
        // Check blocks since last check
        let start_block = std::cmp::max(tracked.last_check_block, tracked.bundle.target_block);
        let end_block = latest_block;
        
        for block_num in start_block..=end_block {
            if let Ok(inclusion_result) = self.check_block_for_bundle(block_num, tracked).await {
                if inclusion_result.is_some() {
                    let (block_number, tx_hashes) = inclusion_result.unwrap();
                    tracked.status = ExecutionStatus::Included {
                        block_number,
                        transaction_hashes: tx_hashes,
                    };
                    
                    tracing::info!(
                        "Bundle {} included in block {}",
                        bundle_id,
                        block_number
                    );
                    
                    // Generate inclusion metrics
                    if let Ok(metrics) = self.generate_inclusion_metrics(tracked).await {
                        tracing::info!("Bundle metrics: {:?}", metrics);
                    }
                    
                    return Ok(());
                }
            }
        }
        
        tracked.last_check_block = end_block;
        tracked.inclusion_attempts += 1;
        
        Ok(())
    }
    
    /// Check a specific block for bundle inclusion
    async fn check_block_for_bundle(
        &self,
        block_number: u64,
        tracked: &TrackedBundle,
    ) -> Result<Option<(u64, Vec<String>)>> {
        let block = self.get_block_by_number(block_number).await?;
        
        // Get all transaction hashes from the bundle
        let bundle_tx_hashes: Vec<String> = tracked.bundle
            .all_transactions()
            .iter()
            .map(|tx| tx.hash_hex())
            .collect();
        
        // Check if all bundle transactions are in this block
        let mut found_hashes = Vec::new();
        
        for tx_hash in &bundle_tx_hashes {
            if block.transactions.iter().any(|tx| tx.hash == *tx_hash) {
                found_hashes.push(tx_hash.clone());
            }
        }
        
        // Bundle is included if all transactions are found
        if found_hashes.len() == bundle_tx_hashes.len() {
            // Verify transaction ordering
            if self.config.detailed_analysis {
                let ordering = self.verify_transaction_ordering(&block, &bundle_tx_hashes).await?;
                if !ordering.is_correct_order {
                    tracing::warn!(
                        "Bundle {} has incorrect transaction ordering in block {}",
                        tracked.bundle.id,
                        block_number
                    );
                }
            }
            
            Ok(Some((block_number, found_hashes)))
        } else if !found_hashes.is_empty() {
            // Partial inclusion - this is a problem
            tracing::error!(
                "Partial bundle inclusion detected for bundle {} in block {}: {}/{} transactions",
                tracked.bundle.id,
                block_number,
                found_hashes.len(),
                bundle_tx_hashes.len()
            );
            Ok(None)
        } else {
            Ok(None)
        }
    }
    
    /// Verify that transactions appear in the correct order within a block
    async fn verify_transaction_ordering(
        &self,
        block: &BlockInfo,
        expected_tx_hashes: &[String],
    ) -> Result<OrderingVerification> {
        let mut actual_positions = Vec::new();
        let mut actual_order = Vec::new();
        
        // Find positions of our transactions in the block
        for expected_hash in expected_tx_hashes {
            if let Some(pos) = block.transactions.iter().position(|tx| tx.hash == *expected_hash) {
                actual_positions.push(pos);
                actual_order.push(expected_hash.clone());
            }
        }
        
        // Check if positions are in ascending order
        let is_correct_order = actual_positions.windows(2).all(|w| w[0] < w[1]);
        
        // Calculate position differences
        let position_differences: Vec<i32> = actual_positions
            .iter()
            .enumerate()
            .map(|(expected_idx, &actual_pos)| actual_pos as i32 - expected_idx as i32)
            .collect();
        
        Ok(OrderingVerification {
            is_correct_order,
            expected_order: expected_tx_hashes.to_vec(),
            actual_order,
            position_differences,
        })
    }
    
    /// Generate detailed inclusion metrics
    async fn generate_inclusion_metrics(&self, tracked: &TrackedBundle) -> Result<InclusionMetrics> {
        let inclusion_latency = tracked.tracking_start.elapsed();
        
        // Get transaction receipts for gas analysis
        let mut total_gas_used = 0u64;
        let mut block_positions = Vec::new();
        
        for tx in tracked.bundle.all_transactions() {
            if let Ok(receipt) = self.get_transaction_receipt(&tx.hash_hex()).await {
                total_gas_used += receipt.gas_used;
                block_positions.push(receipt.transaction_index);
            }
        }
        
        // Calculate actual block number from status
        let actual_block = match &tracked.status {
            ExecutionStatus::Included { block_number, .. } => *block_number,
            _ => 0,
        };
        
        Ok(InclusionMetrics {
            bundle_id: tracked.bundle.id.clone(),
            submission_to_inclusion_latency: inclusion_latency,
            target_block: tracked.bundle.target_block,
            actual_block,
            block_position: block_positions,
            gas_used: total_gas_used,
            gas_price_paid: tracked.bundle.max_gas_price.as_u64(),
            mev_extracted: None, // Would require additional analysis
        })
    }
    
    /// Get latest block number from RPC
    async fn get_latest_block_number(&self) -> Result<u64> {
        let response = self.client
            .post(&self.config.rpc_endpoint)
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "eth_blockNumber",
                "params": [],
                "id": 1
            }))
            .send()
            .await?;
        
        let result: Value = response.json().await?;
        
        if let Some(error) = result.get("error") {
            return Err(anyhow!("RPC error: {}", error));
        }
        
        let block_hex = result["result"]
            .as_str()
            .ok_or_else(|| anyhow!("Invalid block number response"))?;
        
        let block_number = u64::from_str_radix(block_hex.trim_start_matches("0x"), 16)?;
        Ok(block_number)
    }
    
    /// Get block information by number
    async fn get_block_by_number(&self, block_number: u64) -> Result<BlockInfo> {
        let response = self.client
            .post(&self.config.rpc_endpoint)
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "eth_getBlockByNumber",
                "params": [format!("0x{:x}", block_number), true],
                "id": 1
            }))
            .send()
            .await?;
        
        let result: Value = response.json().await?;
        
        if let Some(error) = result.get("error") {
            return Err(anyhow!("RPC error: {}", error));
        }
        
        let block_data = &result["result"];
        if block_data.is_null() {
            return Err(anyhow!("Block not found: {}", block_number));
        }
        
        let transactions: Vec<TransactionInfo> = block_data["transactions"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|tx| TransactionInfo {
                hash: tx["hash"].as_str().unwrap_or("").to_string(),
                from: tx["from"].as_str().unwrap_or("").to_string(),
                to: tx["to"].as_str().map(|s| s.to_string()),
                gas_price: tx["gasPrice"].as_str().unwrap_or("0x0").to_string(),
            })
            .collect();
        
        Ok(BlockInfo {
            number: block_number,
            hash: block_data["hash"].as_str().unwrap_or("").to_string(),
            timestamp: block_data["timestamp"].as_str().unwrap_or("0x0").to_string(),
            transactions,
        })
    }
    
    /// Get transaction receipt
    async fn get_transaction_receipt(&self, tx_hash: &str) -> Result<TransactionReceipt> {
        let response = self.client
            .post(&self.config.rpc_endpoint)
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "eth_getTransactionReceipt",
                "params": [tx_hash],
                "id": 1
            }))
            .send()
            .await?;
        
        let result: Value = response.json().await?;
        
        if let Some(error) = result.get("error") {
            return Err(anyhow!("RPC error: {}", error));
        }
        
        let receipt_data = &result["result"];
        if receipt_data.is_null() {
            return Err(anyhow!("Transaction receipt not found: {}", tx_hash));
        }
        
        let block_number = u64::from_str_radix(
            receipt_data["blockNumber"].as_str().unwrap_or("0x0").trim_start_matches("0x"),
            16
        )?;
        
        let transaction_index = u64::from_str_radix(
            receipt_data["transactionIndex"].as_str().unwrap_or("0x0").trim_start_matches("0x"),
            16
        )?;
        
        let gas_used = u64::from_str_radix(
            receipt_data["gasUsed"].as_str().unwrap_or("0x0").trim_start_matches("0x"),
            16
        )?;
        
        let status = receipt_data["status"].as_str().unwrap_or("0x0") == "0x1";
        
        Ok(TransactionReceipt {
            transaction_hash: tx_hash.to_string(),
            block_number,
            block_hash: receipt_data["blockHash"].as_str().unwrap_or("").to_string(),
            transaction_index,
            gas_used,
            status,
            logs: vec![], // Simplified for now
        })
    }
    
    /// Clean up expired bundle tracking
    async fn cleanup_expired_bundles(&self) {
        let mut bundles = self.tracked_bundles.write().await;
        let now = Instant::now();
        
        bundles.retain(|bundle_id, tracked| {
            let should_keep = match &tracked.status {
                ExecutionStatus::Pending => {
                    now.duration_since(tracked.tracking_start) < self.config.max_tracking_duration
                }
                _ => false, // Remove completed/failed/expired bundles
            };
            
            if !should_keep {
                tracing::debug!("Cleaning up tracking for bundle: {}", bundle_id);
            }
            
            should_keep
        });
    }
    
    /// Get tracking statistics
    pub async fn get_tracking_stats(&self) -> TrackingStats {
        let bundles = self.tracked_bundles.read().await;
        
        let mut pending = 0;
        let mut included = 0;
        let mut failed = 0;
        let mut expired = 0;
        
        for tracked in bundles.values() {
            match tracked.status {
                ExecutionStatus::Pending => pending += 1,
                ExecutionStatus::Included { .. } => included += 1,
                ExecutionStatus::Failed { .. } => failed += 1,
                ExecutionStatus::Expired => expired += 1,
            }
        }
        
        TrackingStats {
            total_tracked: bundles.len(),
            pending,
            included,
            failed,
            expired,
        }
    }
}

/// Simplified block information
#[derive(Debug, Clone)]
struct BlockInfo {
    number: u64,
    hash: String,
    timestamp: String,
    transactions: Vec<TransactionInfo>,
}

/// Simplified transaction information
#[derive(Debug, Clone)]
struct TransactionInfo {
    hash: String,
    from: String,
    to: Option<String>,
    gas_price: String,
}

/// Tracking statistics
#[derive(Debug, Clone)]
pub struct TrackingStats {
    pub total_tracked: usize,
    pub pending: usize,
    pub included: usize,
    pub failed: usize,
    pub expired: usize,
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
                value: ethers::types::U256::from(1000000000000000000u64),
                gas_limit: ethers::types::U256::from(21000),
                max_fee_per_gas: ethers::types::U256::from(50000000000u64),
                max_priority_fee_per_gas: ethers::types::U256::from(2000000000u64),
                nonce: ethers::types::U256::from(1),
                data: Bytes::default(),
                chain_id: 998,
                raw_transaction: Bytes::default(),
                transaction_type: 2,
            },
            post_transactions: vec![],
            target_block: 1000,
            max_gas_price: ethers::types::U256::from(100000000000u64),
            estimated_profit: ethers::types::U256::from(50000000000000000u64),
            created_at: chrono::Utc::now(),
            metadata: std::collections::HashMap::new(),
        }
    }
    
    fn create_test_submission_result() -> SubmissionResult {
        SubmissionResult {
            bundle_id: "test_bundle".to_string(),
            submission_hash: "0x1234567890abcdef".to_string(),
            submitted_at: chrono::Utc::now(),
            gas_price: ethers::types::U256::from(50000000000u64),
            estimated_inclusion_block: 1000,
        }
    }
    
    #[tokio::test]
    async fn test_tracker_creation() {
        let config = TrackerConfig::default();
        let tracker = ExecutionTracker::new(config);
        
        let stats = tracker.get_tracking_stats().await;
        assert_eq!(stats.total_tracked, 0);
    }
    
    #[tokio::test]
    async fn test_bundle_tracking() {
        let config = TrackerConfig::default();
        let tracker = ExecutionTracker::new(config);
        
        let bundle = create_test_bundle();
        let submission = create_test_submission_result();
        
        tracker.track_bundle(bundle.clone(), submission).await.unwrap();
        
        let status = tracker.get_execution_status(&bundle.id).await.unwrap();
        assert!(matches!(status, ExecutionStatus::Pending));
        
        let stats = tracker.get_tracking_stats().await;
        assert_eq!(stats.total_tracked, 1);
        assert_eq!(stats.pending, 1);
    }
    
    #[tokio::test]
    async fn test_ordering_verification() {
        let config = TrackerConfig::default();
        let tracker = ExecutionTracker::new(config);
        
        let block = BlockInfo {
            number: 1000,
            hash: "0xblock".to_string(),
            timestamp: "0x123456".to_string(),
            transactions: vec![
                TransactionInfo {
                    hash: "0xaaa".to_string(),
                    from: "0xfrom1".to_string(),
                    to: Some("0xto1".to_string()),
                    gas_price: "0x1234".to_string(),
                },
                TransactionInfo {
                    hash: "0xbbb".to_string(),
                    from: "0xfrom2".to_string(),
                    to: Some("0xto2".to_string()),
                    gas_price: "0x1234".to_string(),
                },
            ],
        };
        
        let expected_order = vec!["0xaaa".to_string(), "0xbbb".to_string()];
        let verification = tracker.verify_transaction_ordering(&block, &expected_order).await.unwrap();
        
        assert!(verification.is_correct_order);
        assert_eq!(verification.expected_order, expected_order);
    }
    
    #[tokio::test]
    async fn test_inclusion_metrics() {
        let bundle = create_test_bundle();
        let submission = create_test_submission_result();
        
        let tracked = TrackedBundle {
            bundle: bundle.clone(),
            submission_result: submission,
            tracking_start: Instant::now(),
            status: ExecutionStatus::Included {
                block_number: 1001,
                transaction_hashes: vec!["0xaaa".to_string()],
            },
            inclusion_attempts: 1,
            last_check_block: 1001,
            transaction_receipts: HashMap::new(),
        };
        
        // Test that metrics structure is correct
        assert_eq!(tracked.bundle.id, "test_bundle");
        assert!(matches!(tracked.status, ExecutionStatus::Included { .. }));
    }
}