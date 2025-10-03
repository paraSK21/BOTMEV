//! Simple SQLite persistence layer for MEV bot state

use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json;
use std::path::Path;
use tracing::{debug, info};

use crate::{BlockHeader, PendingTransactionState, ReorgEvent};

/// SQLite persistence manager
pub struct PersistenceManager {
    connection: Connection,
}

impl PersistenceManager {
    /// Create new persistence manager
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let connection = Connection::open(db_path)?;
        let manager = Self { connection };
        
        manager.initialize_schema()?;
        info!("Persistence manager initialized");
        
        Ok(manager)
    }

    /// Initialize database schema
    fn initialize_schema(&self) -> Result<()> {
        // Block headers table
        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS block_headers (
                number INTEGER PRIMARY KEY,
                hash TEXT NOT NULL,
                parent_hash TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                gas_limit INTEGER NOT NULL,
                gas_used INTEGER NOT NULL,
                difficulty TEXT NOT NULL,
                nonce TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Pending transactions table
        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS pending_transactions (
                hash TEXT PRIMARY KEY,
                from_address TEXT NOT NULL,
                to_address TEXT,
                nonce INTEGER NOT NULL,
                gas_price INTEGER NOT NULL,
                gas_limit INTEGER NOT NULL,
                value TEXT NOT NULL,
                data TEXT NOT NULL,
                first_seen DATETIME NOT NULL,
                last_seen DATETIME NOT NULL,
                seen_count INTEGER NOT NULL,
                block_number INTEGER,
                block_hash TEXT,
                status TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Reorg events table
        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS reorg_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                old_chain TEXT NOT NULL,
                new_chain TEXT NOT NULL,
                common_ancestor TEXT NOT NULL,
                depth INTEGER NOT NULL,
                detected_at DATETIME NOT NULL,
                affected_transactions TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Nonce tracker table
        self.connection.execute(
            "CREATE TABLE IF NOT EXISTS nonce_tracker (
                address TEXT PRIMARY KEY,
                highest_nonce INTEGER NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )?;

        // Create indexes for better performance
        self.connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_pending_tx_from ON pending_transactions(from_address)",
            [],
        )?;

        self.connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_pending_tx_status ON pending_transactions(status)",
            [],
        )?;

        self.connection.execute(
            "CREATE INDEX IF NOT EXISTS idx_block_headers_number ON block_headers(number)",
            [],
        )?;

        debug!("Database schema initialized");
        Ok(())
    }

    /// Save block header
    pub fn save_block_header(&self, header: &BlockHeader) -> Result<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO block_headers 
             (number, hash, parent_hash, timestamp, gas_limit, gas_used, difficulty, nonce)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                header.number,
                header.hash,
                header.parent_hash,
                header.timestamp,
                header.gas_limit,
                header.gas_used,
                header.difficulty,
                header.nonce,
            ],
        )?;

        debug!(block_number = header.number, "Saved block header");
        Ok(())
    }

    /// Load recent block headers
    pub fn load_recent_block_headers(&self, limit: usize) -> Result<Vec<BlockHeader>> {
        let mut stmt = self.connection.prepare(
            "SELECT number, hash, parent_hash, timestamp, gas_limit, gas_used, difficulty, nonce
             FROM block_headers 
             ORDER BY number DESC 
             LIMIT ?1"
        )?;

        let header_iter = stmt.query_map(params![limit], |row| {
            Ok(BlockHeader {
                number: row.get(0)?,
                hash: row.get(1)?,
                parent_hash: row.get(2)?,
                timestamp: row.get(3)?,
                gas_limit: row.get(4)?,
                gas_used: row.get(5)?,
                difficulty: row.get(6)?,
                nonce: row.get(7)?,
            })
        })?;

        let mut headers = Vec::new();
        for header in header_iter {
            headers.push(header?);
        }

        debug!(count = headers.len(), "Loaded block headers");
        Ok(headers)
    }

    /// Save pending transaction
    pub fn save_pending_transaction(&self, tx: &PendingTransactionState) -> Result<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO pending_transactions 
             (hash, from_address, to_address, nonce, gas_price, gas_limit, value, data,
              first_seen, last_seen, seen_count, block_number, block_hash, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                tx.hash,
                tx.from,
                tx.to,
                tx.nonce,
                tx.gas_price,
                tx.gas_limit,
                tx.value,
                tx.data,
                tx.first_seen.to_rfc3339(),
                tx.last_seen.to_rfc3339(),
                tx.seen_count,
                tx.block_number,
                tx.block_hash,
                format!("{:?}", tx.status),
            ],
        )?;

        debug!(tx_hash = %tx.hash, "Saved pending transaction");
        Ok(())
    }

    /// Load pending transactions
    pub fn load_pending_transactions(&self, limit: usize) -> Result<Vec<PendingTransactionState>> {
        let mut stmt = self.connection.prepare(
            "SELECT hash, from_address, to_address, nonce, gas_price, gas_limit, value, data,
                    first_seen, last_seen, seen_count, block_number, block_hash, status
             FROM pending_transactions 
             WHERE status = 'Pending'
             ORDER BY first_seen DESC 
             LIMIT ?1"
        )?;

        let tx_iter = stmt.query_map(params![limit], |row| {
            let status_str: String = row.get(13)?;
            let status = match status_str.as_str() {
                "Pending" => crate::TransactionStatus::Pending,
                "Included" => crate::TransactionStatus::Included,
                "Dropped" => crate::TransactionStatus::Dropped,
                "Replaced" => crate::TransactionStatus::Replaced,
                _ => crate::TransactionStatus::Pending,
            };

            Ok(PendingTransactionState {
                hash: row.get(0)?,
                from: row.get(1)?,
                to: row.get(2)?,
                nonce: row.get(3)?,
                gas_price: row.get(4)?,
                gas_limit: row.get(5)?,
                value: row.get(6)?,
                data: row.get(7)?,
                first_seen: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(8)?)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                last_seen: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(9)?)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                seen_count: row.get(10)?,
                block_number: row.get(11)?,
                block_hash: row.get(12)?,
                status,
            })
        })?;

        let mut transactions = Vec::new();
        for tx in tx_iter {
            transactions.push(tx?);
        }

        debug!(count = transactions.len(), "Loaded pending transactions");
        Ok(transactions)
    }

    /// Save reorg event
    pub fn save_reorg_event(&self, event: &ReorgEvent) -> Result<()> {
        let old_chain_json = serde_json::to_string(&event.old_chain)?;
        let new_chain_json = serde_json::to_string(&event.new_chain)?;
        let common_ancestor_json = serde_json::to_string(&event.common_ancestor)?;
        let affected_txs_json = serde_json::to_string(&event.affected_transactions)?;

        self.connection.execute(
            "INSERT INTO reorg_events 
             (old_chain, new_chain, common_ancestor, depth, detected_at, affected_transactions)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                old_chain_json,
                new_chain_json,
                common_ancestor_json,
                event.depth,
                event.detected_at.to_rfc3339(),
                affected_txs_json,
            ],
        )?;

        debug!(depth = event.depth, "Saved reorg event");
        Ok(())
    }

    /// Load recent reorg events
    pub fn load_reorg_events(&self, limit: usize) -> Result<Vec<ReorgEvent>> {
        let mut stmt = self.connection.prepare(
            "SELECT old_chain, new_chain, common_ancestor, depth, detected_at, affected_transactions
             FROM reorg_events 
             ORDER BY detected_at DESC 
             LIMIT ?1"
        )?;

        let event_iter = stmt.query_map(params![limit], |row| {
            let old_chain_json: String = row.get(0)?;
            let new_chain_json: String = row.get(1)?;
            let common_ancestor_json: String = row.get(2)?;
            let affected_txs_json: String = row.get(5)?;

            let old_chain: Vec<BlockHeader> = serde_json::from_str(&old_chain_json)
                .map_err(|_e| rusqlite::Error::InvalidColumnType(0, "old_chain".to_string(), rusqlite::types::Type::Text))?;
            let new_chain: Vec<BlockHeader> = serde_json::from_str(&new_chain_json)
                .map_err(|_e| rusqlite::Error::InvalidColumnType(1, "new_chain".to_string(), rusqlite::types::Type::Text))?;
            let common_ancestor: BlockHeader = serde_json::from_str(&common_ancestor_json)
                .map_err(|_e| rusqlite::Error::InvalidColumnType(2, "common_ancestor".to_string(), rusqlite::types::Type::Text))?;
            let affected_transactions: Vec<String> = serde_json::from_str(&affected_txs_json)
                .map_err(|_e| rusqlite::Error::InvalidColumnType(5, "affected_transactions".to_string(), rusqlite::types::Type::Text))?;

            Ok(ReorgEvent {
                old_chain,
                new_chain,
                common_ancestor,
                depth: row.get(3)?,
                detected_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                    .unwrap()
                    .with_timezone(&chrono::Utc),
                affected_transactions,
            })
        })?;

        let mut events = Vec::new();
        for event in event_iter {
            events.push(event?);
        }

        debug!(count = events.len(), "Loaded reorg events");
        Ok(events)
    }

    /// Save nonce tracker state
    pub fn save_nonce_tracker(&self, address: &str, nonce: u64) -> Result<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO nonce_tracker (address, highest_nonce)
             VALUES (?1, ?2)",
            params![address, nonce],
        )?;

        debug!(address = %address, nonce = nonce, "Saved nonce tracker");
        Ok(())
    }

    /// Load nonce tracker state
    pub fn load_nonce_tracker(&self) -> Result<std::collections::HashMap<String, u64>> {
        let mut stmt = self.connection.prepare(
            "SELECT address, highest_nonce FROM nonce_tracker"
        )?;

        let nonce_iter = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })?;

        let mut nonce_map = std::collections::HashMap::new();
        for nonce_entry in nonce_iter {
            let (address, nonce) = nonce_entry?;
            nonce_map.insert(address, nonce);
        }

        debug!(count = nonce_map.len(), "Loaded nonce tracker");
        Ok(nonce_map)
    }

    /// Clean up old data
    pub fn cleanup_old_data(&self, days_to_keep: u32) -> Result<()> {
        let cutoff_date = chrono::Utc::now() - chrono::Duration::days(days_to_keep as i64);
        let cutoff_str = cutoff_date.to_rfc3339();

        // Clean up old pending transactions
        let deleted_txs = self.connection.execute(
            "DELETE FROM pending_transactions WHERE created_at < ?1",
            params![cutoff_str],
        )?;

        // Clean up old reorg events
        let deleted_reorgs = self.connection.execute(
            "DELETE FROM reorg_events WHERE created_at < ?1",
            params![cutoff_str],
        )?;

        // Keep only recent block headers (last 10000 blocks)
        let deleted_blocks = self.connection.execute(
            "DELETE FROM block_headers WHERE number < (
                SELECT MAX(number) - 10000 FROM block_headers
            )",
            [],
        )?;

        info!(
            deleted_txs = deleted_txs,
            deleted_reorgs = deleted_reorgs,
            deleted_blocks = deleted_blocks,
            "Cleaned up old data"
        );

        Ok(())
    }

    /// Get database statistics
    pub fn get_stats(&self) -> Result<PersistenceStats> {
        let block_count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM block_headers",
            [],
            |row| row.get(0),
        )?;

        let pending_tx_count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM pending_transactions WHERE status = 'Pending'",
            [],
            |row| row.get(0),
        )?;

        let reorg_count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM reorg_events",
            [],
            |row| row.get(0),
        )?;

        let nonce_tracker_count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM nonce_tracker",
            [],
            |row| row.get(0),
        )?;

        Ok(PersistenceStats {
            block_headers: block_count as usize,
            pending_transactions: pending_tx_count as usize,
            reorg_events: reorg_count as usize,
            tracked_addresses: nonce_tracker_count as usize,
        })
    }
}

/// Persistence layer statistics
#[derive(Debug, Clone)]
pub struct PersistenceStats {
    pub block_headers: usize,
    pub pending_transactions: usize,
    pub reorg_events: usize,
    pub tracked_addresses: usize,
}

impl PersistenceStats {
    pub fn print_summary(&self) {
        info!(
            block_headers = self.block_headers,
            pending_transactions = self.pending_transactions,
            reorg_events = self.reorg_events,
            tracked_addresses = self.tracked_addresses,
            "Persistence layer statistics"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_persistence_manager_creation() {
        let temp_file = NamedTempFile::new().unwrap();
        let manager = PersistenceManager::new(temp_file.path());
        assert!(manager.is_ok());
    }

    #[test]
    fn test_block_header_persistence() {
        let temp_file = NamedTempFile::new().unwrap();
        let manager = PersistenceManager::new(temp_file.path()).unwrap();

        let header = BlockHeader::new(1, "0x123".to_string(), "0x000".to_string());
        
        // Save header
        let result = manager.save_block_header(&header);
        assert!(result.is_ok());

        // Load headers
        let loaded_headers = manager.load_recent_block_headers(10).unwrap();
        assert_eq!(loaded_headers.len(), 1);
        assert_eq!(loaded_headers[0].number, 1);
        assert_eq!(loaded_headers[0].hash, "0x123");
    }
}