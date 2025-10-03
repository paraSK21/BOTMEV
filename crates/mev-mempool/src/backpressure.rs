//! Backpressure handling for MEV bot mempool processing

use anyhow::Result;
use mev_core::{ParsedTransaction, PrometheusMetrics};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Drop policy for when buffer is full
#[derive(Debug, Clone, PartialEq)]
pub enum DropPolicy {
    Oldest,
    Newest,
    Random,
}

impl From<&str> for DropPolicy {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "oldest" => DropPolicy::Oldest,
            "newest" => DropPolicy::Newest,
            "random" => DropPolicy::Random,
            _ => DropPolicy::Oldest,
        }
    }
}

/// Backpressure configuration
#[derive(Debug, Clone)]
pub struct BackpressureConfig {
    pub enabled: bool,
    pub drop_policy: DropPolicy,
    pub high_watermark: f64,
    pub low_watermark: f64,
    pub buffer_size: usize,
}

impl Default for BackpressureConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            drop_policy: DropPolicy::Oldest,
            high_watermark: 0.8,
            low_watermark: 0.6,
            buffer_size: 10000,
        }
    }
}

/// Backpressure-aware transaction buffer
pub struct BackpressureBuffer {
    sender: mpsc::Sender<ParsedTransaction>,
    receiver: Arc<tokio::sync::Mutex<mpsc::Receiver<ParsedTransaction>>>,
    config: BackpressureConfig,
    metrics: Arc<PrometheusMetrics>,
    
    // Statistics
    total_received: AtomicU64,
    total_dropped: AtomicU64,
    total_processed: AtomicU64,
    backpressure_active: AtomicBool,
}

impl BackpressureBuffer {
    pub fn new(config: BackpressureConfig, metrics: Arc<PrometheusMetrics>) -> Self {
        let (sender, receiver) = mpsc::channel(config.buffer_size);
        
        Self {
            sender,
            receiver: Arc::new(tokio::sync::Mutex::new(receiver)),
            config,
            metrics,
            total_received: AtomicU64::new(0),
            total_dropped: AtomicU64::new(0),
            total_processed: AtomicU64::new(0),
            backpressure_active: AtomicBool::new(false),
        }
    }

    /// Try to push a transaction to the buffer
    pub async fn push(&self, transaction: ParsedTransaction) -> Result<bool> {
        self.total_received.fetch_add(1, Ordering::Relaxed);
        
        if !self.config.enabled {
            // Backpressure disabled, try to send directly
            match self.sender.try_send(transaction) {
                Ok(_) => {
                    self.total_processed.fetch_add(1, Ordering::Relaxed);
                    return Ok(true);
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    self.total_dropped.fetch_add(1, Ordering::Relaxed);
                    warn!("Buffer full, dropping transaction (backpressure disabled)");
                    return Ok(false);
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err(anyhow::anyhow!("Buffer channel closed"));
                }
            }
        }

        // Check current buffer utilization
        let current_size = self.sender.capacity() - self.sender.max_capacity();
        let utilization = current_size as f64 / self.config.buffer_size as f64;
        
        // Update metrics
        self.metrics.set_buffer_utilization("mempool", (utilization * 100.0) as i64);

        // Handle backpressure
        if utilization >= self.config.high_watermark {
            if !self.backpressure_active.load(Ordering::Relaxed) {
                info!(
                    utilization = utilization,
                    high_watermark = self.config.high_watermark,
                    "Activating backpressure"
                );
                self.backpressure_active.store(true, Ordering::Relaxed);
            }

            // Apply drop policy
            match self.config.drop_policy {
                DropPolicy::Oldest => {
                    // Try to make space by dropping the oldest (this is automatic with MPSC)
                    match self.sender.try_send(transaction) {
                        Ok(_) => {
                            self.total_processed.fetch_add(1, Ordering::Relaxed);
                            Ok(true)
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            self.total_dropped.fetch_add(1, Ordering::Relaxed);
                            debug!("Dropping transaction due to backpressure (oldest policy)");
                            Ok(false)
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            Err(anyhow::anyhow!("Buffer channel closed"))
                        }
                    }
                }
                DropPolicy::Newest => {
                    // Drop the new transaction
                    self.total_dropped.fetch_add(1, Ordering::Relaxed);
                    debug!("Dropping new transaction due to backpressure (newest policy)");
                    Ok(false)
                }
                DropPolicy::Random => {
                    // Randomly decide whether to drop
                    if rand::random::<f64>() < 0.5 {
                        self.total_dropped.fetch_add(1, Ordering::Relaxed);
                        debug!("Randomly dropping transaction due to backpressure");
                        Ok(false)
                    } else {
                        match self.sender.try_send(transaction) {
                            Ok(_) => {
                                self.total_processed.fetch_add(1, Ordering::Relaxed);
                                Ok(true)
                            }
                            Err(mpsc::error::TrySendError::Full(_)) => {
                                self.total_dropped.fetch_add(1, Ordering::Relaxed);
                                Ok(false)
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => {
                                Err(anyhow::anyhow!("Buffer channel closed"))
                            }
                        }
                    }
                }
            }
        } else {
            // Normal operation
            if self.backpressure_active.load(Ordering::Relaxed) && utilization <= self.config.low_watermark {
                info!(
                    utilization = utilization,
                    low_watermark = self.config.low_watermark,
                    "Deactivating backpressure"
                );
                self.backpressure_active.store(false, Ordering::Relaxed);
            }

            match self.sender.try_send(transaction) {
                Ok(_) => {
                    self.total_processed.fetch_add(1, Ordering::Relaxed);
                    Ok(true)
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    self.total_dropped.fetch_add(1, Ordering::Relaxed);
                    warn!("Buffer unexpectedly full during normal operation");
                    Ok(false)
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    Err(anyhow::anyhow!("Buffer channel closed"))
                }
            }
        }
    }

    /// Receive a batch of transactions
    pub async fn receive_batch(&self, max_batch_size: usize) -> Vec<ParsedTransaction> {
        let mut batch = Vec::with_capacity(max_batch_size);
        let mut receiver = self.receiver.lock().await;

        // Try to receive up to max_batch_size transactions
        for _ in 0..max_batch_size {
            match receiver.try_recv() {
                Ok(transaction) => {
                    batch.push(transaction);
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    break; // No more transactions available
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    warn!("Buffer channel disconnected");
                    break;
                }
            }
        }

        batch
    }

    /// Wait for at least one transaction
    pub async fn receive_one(&self) -> Option<ParsedTransaction> {
        let mut receiver = self.receiver.lock().await;
        receiver.recv().await
    }

    /// Get buffer statistics
    pub fn get_stats(&self) -> BackpressureStats {
        let current_size = self.sender.capacity() - self.sender.max_capacity();
        let utilization = current_size as f64 / self.config.buffer_size as f64;

        BackpressureStats {
            total_received: self.total_received.load(Ordering::Relaxed),
            total_dropped: self.total_dropped.load(Ordering::Relaxed),
            total_processed: self.total_processed.load(Ordering::Relaxed),
            current_size,
            buffer_capacity: self.config.buffer_size,
            utilization,
            backpressure_active: self.backpressure_active.load(Ordering::Relaxed),
            drop_rate: {
                let received = self.total_received.load(Ordering::Relaxed);
                let dropped = self.total_dropped.load(Ordering::Relaxed);
                if received > 0 {
                    dropped as f64 / received as f64
                } else {
                    0.0
                }
            },
        }
    }
}

/// Statistics for backpressure buffer
#[derive(Debug, Clone)]
pub struct BackpressureStats {
    pub total_received: u64,
    pub total_dropped: u64,
    pub total_processed: u64,
    pub current_size: usize,
    pub buffer_capacity: usize,
    pub utilization: f64,
    pub backpressure_active: bool,
    pub drop_rate: f64,
}

impl BackpressureStats {
    pub fn print_summary(&self) {
        info!(
            received = self.total_received,
            processed = self.total_processed,
            dropped = self.total_dropped,
            drop_rate = format!("{:.2}%", self.drop_rate * 100.0),
            utilization = format!("{:.1}%", self.utilization * 100.0),
            backpressure_active = self.backpressure_active,
            "Backpressure buffer stats"
        );
    }
}

/// Load testing utility for backpressure system
pub struct BackpressureLoadTester {
    buffer: Arc<BackpressureBuffer>,
    target_tps: u64,
    duration_seconds: u64,
}

impl BackpressureLoadTester {
    pub fn new(buffer: Arc<BackpressureBuffer>, target_tps: u64, duration_seconds: u64) -> Self {
        Self {
            buffer,
            target_tps,
            duration_seconds,
        }
    }

    /// Run load test
    pub async fn run_test(&self) -> Result<BackpressureStats> {
        info!(
            target_tps = self.target_tps,
            duration_seconds = self.duration_seconds,
            "Starting backpressure load test"
        );

        let interval = tokio::time::Duration::from_millis(1000 / self.target_tps);
        let end_time = tokio::time::Instant::now() + tokio::time::Duration::from_secs(self.duration_seconds);

        let mut transaction_count = 0u64;

        while tokio::time::Instant::now() < end_time {
            // Create a dummy transaction
            let transaction = self.create_dummy_transaction(transaction_count);
            
            // Try to push it
            if let Err(e) = self.buffer.push(transaction).await {
                warn!("Failed to push transaction during load test: {}", e);
            }

            transaction_count += 1;

            // Print progress every 1000 transactions
            if transaction_count % 1000 == 0 {
                let stats = self.buffer.get_stats();
                info!(
                    transactions_sent = transaction_count,
                    utilization = format!("{:.1}%", stats.utilization * 100.0),
                    drop_rate = format!("{:.2}%", stats.drop_rate * 100.0),
                    "Load test progress"
                );
            }

            tokio::time::sleep(interval).await;
        }

        let final_stats = self.buffer.get_stats();
        info!(
            total_sent = transaction_count,
            total_processed = final_stats.total_processed,
            total_dropped = final_stats.total_dropped,
            final_drop_rate = format!("{:.2}%", final_stats.drop_rate * 100.0),
            "Load test completed"
        );

        Ok(final_stats)
    }

    fn create_dummy_transaction(&self, id: u64) -> ParsedTransaction {
        use mev_core::{DecodedInput, TargetType, Transaction};
        use chrono::Utc;

        ParsedTransaction {
            transaction: Transaction {
                hash: format!("0x{:064x}", id),
                from: format!("0x{:040x}", id % 1000),
                to: Some(format!("0x{:040x}", (id + 1) % 1000)),
                value: "1000000000000000000".to_string(),
                gas_price: "20000000000".to_string(),
                gas_limit: "21000".to_string(),
                nonce: id % 100,
                input: "0x".to_string(),
                timestamp: Utc::now(),
            },
            decoded_input: Some(DecodedInput {
                function_name: "transfer".to_string(),
                function_signature: "0xa9059cbb".to_string(),
                parameters: vec![],
            }),
            target_type: if id % 10 == 0 { TargetType::UniswapV2 } else { TargetType::Unknown },
            processing_time_ms: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mev_core::PrometheusMetrics;

    #[tokio::test]
    async fn test_backpressure_buffer_basic() {
        let metrics = Arc::new(PrometheusMetrics::new().unwrap());
        let config = BackpressureConfig {
            buffer_size: 10,
            ..Default::default()
        };
        
        let buffer = BackpressureBuffer::new(config, metrics);
        
        // Create a dummy transaction
        let transaction = create_test_transaction(1);
        
        // Should be able to push
        let result = buffer.push(transaction).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
        
        // Should be able to receive
        let received = buffer.receive_one().await;
        assert!(received.is_some());
    }

    #[tokio::test]
    async fn test_backpressure_activation() {
        let metrics = Arc::new(PrometheusMetrics::new().unwrap());
        let config = BackpressureConfig {
            buffer_size: 5,
            high_watermark: 0.6, // Activate at 60% (3 items)
            ..Default::default()
        };
        
        let buffer = BackpressureBuffer::new(config, metrics);
        
        // Fill buffer to trigger backpressure
        for i in 0..10 {
            let transaction = create_test_transaction(i);
            let _ = buffer.push(transaction).await;
        }
        
        let stats = buffer.get_stats();
        assert!(stats.total_dropped > 0);
    }

    fn create_test_transaction(id: u64) -> ParsedTransaction {
        use mev_core::{TargetType, Transaction};
        use chrono::Utc;

        ParsedTransaction {
            transaction: Transaction {
                hash: format!("0x{:064x}", id),
                from: format!("0x{:040x}", id),
                to: Some(format!("0x{:040x}", id + 1)),
                value: "0".to_string(),
                gas_price: "20000000000".to_string(),
                gas_limit: "21000".to_string(),
                nonce: id,
                input: "0x".to_string(),
                timestamp: Utc::now(),
            },
            decoded_input: None,
            target_type: TargetType::Unknown,
            processing_time_ms: 1,
        }
    }
}