//! In-memory state management and blockchain reorganization detection

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use tracing::{debug, error, info, warn};

/// Block header information for reorg detection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlockHeader {
    pub number: u64,
    pub hash: String,
    pub parent_hash: String,
    pub timestamp: u64,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub difficulty: String,
    pub nonce: String,
}

impl BlockHeader {
    pub fn new(number: u64, hash: String, parent_hash: String) -> Self {
        Self {
            number,
            hash,
            parent_hash,
            timestamp: Utc::now().timestamp() as u64,
            gas_limit: 30_000_000,
            gas_used: 0,
            difficulty: "0x0".to_string(),
            nonce: "0x0".to_string(),
        }
    }
}

/// Pending transaction state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingTransactionState {
    pub hash: String,
    pub from: String,
    pub to: Option<String>,
    pub nonce: u64,
    pub gas_price: u64,
    pub gas_limit: u64,
    pub value: String,
    pub data: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub seen_count: u32,
    pub block_number: Option<u64>,
    pub block_hash: Option<String>,
    pub status: TransactionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TransactionStatus {
    Pending,
    Included,
    Dropped,
    Replaced,
}

/// Blockchain reorganization event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorgEvent {
    pub old_chain: Vec<BlockHeader>,
    pub new_chain: Vec<BlockHeader>,
    pub common_ancestor: BlockHeader,
    pub depth: u64,
    pub detected_at: DateTime<Utc>,
    pub affected_transactions: Vec<String>,
}

/// Configuration for state manager
#[derive(Debug, Clone)]
pub struct StateManagerConfig {
    pub max_pending_transactions: usize,
    pub max_block_history: usize,
    pub transaction_ttl_seconds: u64,
    pub reorg_detection_depth: u64,
    pub checkpoint_interval_seconds: u64,
    pub enable_persistence: bool,
    pub database_path: String,
}

impl Default for StateManagerConfig {
    fn default() -> Self {
        Self {
            max_pending_transactions: 100_000,
            max_block_history: 1000,
            transaction_ttl_seconds: 300, // 5 minutes
            reorg_detection_depth: 64,
            checkpoint_interval_seconds: 60,
            enable_persistence: true,
            database_path: "mev_state.db".to_string(),
        }
    }
}

/// In-memory blockchain state manager
pub struct StateManager {
    config: StateManagerConfig,
    
    // Block chain state
    block_headers: Arc<RwLock<VecDeque<BlockHeader>>>,
    current_head: Arc<RwLock<Option<BlockHeader>>>,
    
    // Transaction state
    pending_transactions: Arc<RwLock<HashMap<String, PendingTransactionState>>>,
    nonce_tracker: Arc<RwLock<HashMap<String, u64>>>, // address -> highest nonce
    
    // Reorg detection
    reorg_events: Arc<RwLock<Vec<ReorgEvent>>>,
    
    // Persistence
    last_checkpoint: Arc<RwLock<Instant>>,
}

impl StateManager {
    pub fn new(config: StateManagerConfig) -> Self {
        Self {
            config,
            block_headers: Arc::new(RwLock::new(VecDeque::new())),
            current_head: Arc::new(RwLock::new(None)),
            pending_transactions: Arc::new(RwLock::new(HashMap::new())),
            nonce_tracker: Arc::new(RwLock::new(HashMap::new())),
            reorg_events: Arc::new(RwLock::new(Vec::new())),
            last_checkpoint: Arc::new(RwLock::new(Instant::now())),
        }
    }

    /// Initialize state manager with current blockchain state
    pub async fn initialize(&self, current_block: BlockHeader) -> Result<()> {
        info!(
            block_number = current_block.number,
            block_hash = %current_block.hash,
            "Initializing state manager"
        );

        // Set current head
        {
            let mut head = self.current_head.write().unwrap();
            *head = Some(current_block.clone());
        }

        // Add to block history
        {
            let mut headers = self.block_headers.write().unwrap();
            headers.push_back(current_block);
        }

        // Load from persistence if enabled
        if self.config.enable_persistence {
            if let Err(e) = self.load_from_persistence().await {
                warn!("Failed to load from persistence: {}", e);
            }
        }

        info!("State manager initialized successfully");
        Ok(())
    }

    /// Update with new block header and detect reorgs
    pub async fn update_block_header(&self, new_header: BlockHeader) -> Result<Option<ReorgEvent>> {
        debug!(
            block_number = new_header.number,
            block_hash = %new_header.hash,
            parent_hash = %new_header.parent_hash,
            "Processing new block header"
        );

        let current_head = {
            let head = self.current_head.read().unwrap();
            head.clone()
        };

        let reorg_event = if let Some(current) = current_head {
            // Check for reorg
            if new_header.number <= current.number {
                // Potential reorg - new block number is not greater than current
                self.detect_reorg(&current, &new_header).await?
            } else if new_header.parent_hash != current.hash {
                // Parent hash mismatch - definite reorg
                self.handle_reorg(&current, &new_header).await?
            } else {
                // Normal progression
                None
            }
        } else {
            None
        };

        // Update current head
        {
            let mut head = self.current_head.write().unwrap();
            *head = Some(new_header.clone());
        }

        // Add to block history
        {
            let mut headers = self.block_headers.write().unwrap();
            headers.push_back(new_header.clone());
            
            // Maintain max history size
            while headers.len() > self.config.max_block_history {
                headers.pop_front();
            }
        }

        // Update transaction states based on new block
        self.update_transaction_states(&new_header).await?;

        Ok(reorg_event)
    }

    /// Add or update pending transaction
    pub async fn update_pending_transaction(&self, tx_hash: String, tx_data: PendingTransactionState) -> Result<()> {
        let mut pending = self.pending_transactions.write().unwrap();
        
        // Check if transaction already exists
        if let Some(existing) = pending.get_mut(&tx_hash) {
            existing.last_seen = Utc::now();
            existing.seen_count += 1;
            debug!(
                tx_hash = %tx_hash,
                seen_count = existing.seen_count,
                "Updated existing pending transaction"
            );
        } else {
            // Add new transaction
            pending.insert(tx_hash.clone(), tx_data);
            debug!(
                tx_hash = %tx_hash,
                "Added new pending transaction"
            );
            
            // Maintain max pending transactions
            if pending.len() > self.config.max_pending_transactions {
                self.cleanup_old_transactions(&mut pending);
            }
        }

        // Update nonce tracker
        let tx = pending.get(&tx_hash).unwrap();
        let mut nonce_tracker = self.nonce_tracker.write().unwrap();
        let current_nonce = nonce_tracker.get(&tx.from).copied().unwrap_or(0);
        if tx.nonce > current_nonce {
            nonce_tracker.insert(tx.from.clone(), tx.nonce);
        }

        Ok(())
    }

    /// Get pending transactions for an address
    pub fn get_pending_transactions(&self, address: &str) -> Vec<PendingTransactionState> {
        let pending = self.pending_transactions.read().unwrap();
        pending
            .values()
            .filter(|tx| tx.from == address || tx.to.as_ref() == Some(&address.to_string()))
            .cloned()
            .collect()
    }

    /// Get current nonce for an address
    pub fn get_current_nonce(&self, address: &str) -> u64 {
        let nonce_tracker = self.nonce_tracker.read().unwrap();
        nonce_tracker.get(address).copied().unwrap_or(0)
    }

    /// Get recent reorg events
    pub fn get_reorg_events(&self, limit: usize) -> Vec<ReorgEvent> {
        let events = self.reorg_events.read().unwrap();
        events.iter().rev().take(limit).cloned().collect()
    }

    /// Get state statistics
    pub fn get_stats(&self) -> StateStats {
        let pending = self.pending_transactions.read().unwrap();
        let headers = self.block_headers.read().unwrap();
        let reorgs = self.reorg_events.read().unwrap();
        let nonce_tracker = self.nonce_tracker.read().unwrap();

        let current_head = {
            let head = self.current_head.read().unwrap();
            head.clone()
        };

        StateStats {
            pending_transactions: pending.len(),
            block_headers: headers.len(),
            reorg_events: reorgs.len(),
            tracked_addresses: nonce_tracker.len(),
            current_block_number: current_head.as_ref().map(|h| h.number),
            current_block_hash: current_head.as_ref().map(|h| h.hash.clone()),
            oldest_pending_transaction: pending
                .values()
                .map(|tx| tx.first_seen)
                .min(),
        }
    }

    /// Start background maintenance tasks
    pub async fn start_maintenance_tasks(&self) -> Result<()> {
        let state_manager = self.clone();
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            
            loop {
                interval.tick().await;
                
                // Cleanup old transactions
                if let Err(e) = state_manager.cleanup_expired_transactions().await {
                    error!("Failed to cleanup expired transactions: {}", e);
                }
                
                // Checkpoint if needed
                if let Err(e) = state_manager.checkpoint_if_needed().await {
                    error!("Failed to checkpoint state: {}", e);
                }
            }
        });

        info!("Started state manager maintenance tasks");
        Ok(())
    }

    /// Detect blockchain reorganization
    async fn detect_reorg(&self, current: &BlockHeader, new_header: &BlockHeader) -> Result<Option<ReorgEvent>> {
        info!(
            current_block = current.number,
            current_hash = %current.hash,
            new_block = new_header.number,
            new_hash = %new_header.hash,
            "Potential reorg detected"
        );

        // For now, treat any non-sequential block as a reorg
        // In a real implementation, you'd fetch the full chain to find the common ancestor
        let reorg_event = ReorgEvent {
            old_chain: vec![current.clone()],
            new_chain: vec![new_header.clone()],
            common_ancestor: current.clone(), // Simplified
            depth: 1,
            detected_at: Utc::now(),
            affected_transactions: self.get_affected_transactions(current, new_header).await,
        };

        // Store reorg event
        {
            let mut reorgs = self.reorg_events.write().unwrap();
            reorgs.push(reorg_event.clone());
        }

        warn!(
            depth = reorg_event.depth,
            affected_txs = reorg_event.affected_transactions.len(),
            "Blockchain reorganization detected"
        );

        Ok(Some(reorg_event))
    }

    /// Handle confirmed reorganization
    async fn handle_reorg(&self, current: &BlockHeader, new_header: &BlockHeader) -> Result<Option<ReorgEvent>> {
        warn!(
            old_block = current.number,
            old_hash = %current.hash,
            new_block = new_header.number,
            new_hash = %new_header.hash,
            "Handling blockchain reorganization"
        );

        // Mark affected transactions as potentially dropped
        let affected_txs = self.get_affected_transactions(current, new_header).await;
        self.mark_transactions_as_dropped(&affected_txs).await?;

        let reorg_event = ReorgEvent {
            old_chain: vec![current.clone()],
            new_chain: vec![new_header.clone()],
            common_ancestor: current.clone(), // Simplified
            depth: 1,
            detected_at: Utc::now(),
            affected_transactions: affected_txs,
        };

        // Store reorg event
        {
            let mut reorgs = self.reorg_events.write().unwrap();
            reorgs.push(reorg_event.clone());
        }

        Ok(Some(reorg_event))
    }

    /// Get transactions affected by reorg
    async fn get_affected_transactions(&self, _old_block: &BlockHeader, _new_block: &BlockHeader) -> Vec<String> {
        // In a real implementation, you'd compare transaction lists between blocks
        // For now, return empty list
        vec![]
    }

    /// Mark transactions as dropped due to reorg
    async fn mark_transactions_as_dropped(&self, tx_hashes: &[String]) -> Result<()> {
        let mut pending = self.pending_transactions.write().unwrap();
        
        for tx_hash in tx_hashes {
            if let Some(tx) = pending.get_mut(tx_hash) {
                tx.status = TransactionStatus::Dropped;
                debug!(tx_hash = %tx_hash, "Marked transaction as dropped due to reorg");
            }
        }

        Ok(())
    }

    /// Update transaction states based on new block
    async fn update_transaction_states(&self, _block: &BlockHeader) -> Result<()> {
        // In a real implementation, you'd check which pending transactions
        // were included in the new block and update their status
        Ok(())
    }

    /// Cleanup old transactions
    fn cleanup_old_transactions(&self, pending: &mut HashMap<String, PendingTransactionState>) {
        let cutoff = Utc::now() - chrono::Duration::seconds(self.config.transaction_ttl_seconds as i64);
        
        let old_txs: Vec<String> = pending
            .iter()
            .filter(|(_, tx)| tx.first_seen < cutoff)
            .map(|(hash, _)| hash.clone())
            .collect();

        for tx_hash in old_txs {
            pending.remove(&tx_hash);
            debug!(tx_hash = %tx_hash, "Removed expired transaction");
        }
    }

    /// Cleanup expired transactions
    async fn cleanup_expired_transactions(&self) -> Result<()> {
        let mut pending = self.pending_transactions.write().unwrap();
        let initial_count = pending.len();
        
        self.cleanup_old_transactions(&mut pending);
        
        let removed_count = initial_count - pending.len();
        if removed_count > 0 {
            info!(removed = removed_count, "Cleaned up expired transactions");
        }

        Ok(())
    }

    /// Checkpoint state to persistence if needed
    async fn checkpoint_if_needed(&self) -> Result<()> {
        let should_checkpoint = {
            let last_checkpoint = self.last_checkpoint.read().unwrap();
            last_checkpoint.elapsed() > Duration::from_secs(self.config.checkpoint_interval_seconds)
        };

        if should_checkpoint && self.config.enable_persistence {
            self.save_to_persistence().await?;
            
            let mut last_checkpoint = self.last_checkpoint.write().unwrap();
            *last_checkpoint = Instant::now();
            
            debug!("State checkpointed to persistence");
        }

        Ok(())
    }

    /// Save state to persistence (SQLite)
    async fn save_to_persistence(&self) -> Result<()> {
        // In a real implementation, you'd save to SQLite
        // For now, just log that we would save
        debug!("Would save state to {}", self.config.database_path);
        Ok(())
    }

    /// Load state from persistence
    async fn load_from_persistence(&self) -> Result<()> {
        // In a real implementation, you'd load from SQLite
        // For now, just log that we would load
        debug!("Would load state from {}", self.config.database_path);
        Ok(())
    }
}

impl Clone for StateManager {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            block_headers: self.block_headers.clone(),
            current_head: self.current_head.clone(),
            pending_transactions: self.pending_transactions.clone(),
            nonce_tracker: self.nonce_tracker.clone(),
            reorg_events: self.reorg_events.clone(),
            last_checkpoint: self.last_checkpoint.clone(),
        }
    }
}

/// State manager statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStats {
    pub pending_transactions: usize,
    pub block_headers: usize,
    pub reorg_events: usize,
    pub tracked_addresses: usize,
    pub current_block_number: Option<u64>,
    pub current_block_hash: Option<String>,
    pub oldest_pending_transaction: Option<DateTime<Utc>>,
}

impl StateStats {
    pub fn print_summary(&self) {
        info!(
            pending_txs = self.pending_transactions,
            block_headers = self.block_headers,
            reorg_events = self.reorg_events,
            tracked_addresses = self.tracked_addresses,
            current_block = self.current_block_number,
            "State manager statistics"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_state_manager_initialization() {
        let config = StateManagerConfig::default();
        let state_manager = StateManager::new(config);
        
        let block = BlockHeader::new(1, "0x123".to_string(), "0x000".to_string());
        let result = state_manager.initialize(block).await;
        
        assert!(result.is_ok());
        
        let stats = state_manager.get_stats();
        assert_eq!(stats.current_block_number, Some(1));
    }

    #[tokio::test]
    async fn test_pending_transaction_management() {
        let config = StateManagerConfig::default();
        let state_manager = StateManager::new(config);
        
        let tx = PendingTransactionState {
            hash: "0xabc".to_string(),
            from: "0x123".to_string(),
            to: Some("0x456".to_string()),
            nonce: 1,
            gas_price: 20_000_000_000,
            gas_limit: 21_000,
            value: "1000000000000000000".to_string(),
            data: "0x".to_string(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            seen_count: 1,
            block_number: None,
            block_hash: None,
            status: TransactionStatus::Pending,
        };
        
        let result = state_manager.update_pending_transaction("0xabc".to_string(), tx).await;
        assert!(result.is_ok());
        
        let pending_txs = state_manager.get_pending_transactions("0x123");
        assert_eq!(pending_txs.len(), 1);
        
        let nonce = state_manager.get_current_nonce("0x123");
        assert_eq!(nonce, 1);
    }

    #[tokio::test]
    async fn test_reorg_detection() {
        let config = StateManagerConfig::default();
        let state_manager = StateManager::new(config);
        
        // Initialize with block 1
        let block1 = BlockHeader::new(1, "0x111".to_string(), "0x000".to_string());
        state_manager.initialize(block1.clone()).await.unwrap();
        
        // Simulate reorg with different block 1
        let block1_alt = BlockHeader::new(1, "0x222".to_string(), "0x000".to_string());
        let reorg_result = state_manager.update_block_header(block1_alt).await.unwrap();
        
        assert!(reorg_result.is_some());
        
        let reorg_events = state_manager.get_reorg_events(10);
        assert_eq!(reorg_events.len(), 1);
    }
}