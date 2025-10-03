use crossbeam::queue::ArrayQueue;
use mev_core::ParsedTransaction;
use std::sync::Arc;
use tracing::{debug, warn};

/// Lock-free ring buffer for parsed transactions
pub struct TransactionRingBuffer {
    queue: Arc<ArrayQueue<ParsedTransaction>>,
    capacity: usize,
    dropped_count: std::sync::atomic::AtomicU64,
}

impl TransactionRingBuffer {
    /// Create a new ring buffer with specified capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: Arc::new(ArrayQueue::new(capacity)),
            capacity,
            dropped_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Push a parsed transaction to the buffer
    /// Returns true if successful, false if buffer is full
    pub fn push(&self, transaction: ParsedTransaction) -> bool {
        match self.queue.push(transaction) {
            Ok(()) => {
                debug!("Transaction pushed to ring buffer");
                true
            }
            Err(transaction) => {
                // Buffer is full, drop the transaction
                self.dropped_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                warn!(
                    "Ring buffer full, dropping transaction: {}. Total dropped: {}",
                    transaction.transaction.hash,
                    self.dropped_count.load(std::sync::atomic::Ordering::Relaxed)
                );
                false
            }
        }
    }

    /// Pop a parsed transaction from the buffer
    pub fn pop(&self) -> Option<ParsedTransaction> {
        self.queue.pop()
    }

    /// Try to pop a transaction without blocking
    pub fn try_pop(&self) -> Option<ParsedTransaction> {
        self.queue.pop()
    }

    /// Get current buffer length (approximate)
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Get buffer capacity
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get number of dropped transactions
    pub fn dropped_count(&self) -> u64 {
        self.dropped_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Get buffer utilization as percentage
    pub fn utilization(&self) -> f64 {
        (self.len() as f64 / self.capacity as f64) * 100.0
    }

    /// Get a clone of the queue for sharing across threads
    pub fn clone_queue(&self) -> Arc<ArrayQueue<ParsedTransaction>> {
        Arc::clone(&self.queue)
    }
}

impl Clone for TransactionRingBuffer {
    fn clone(&self) -> Self {
        Self {
            queue: Arc::clone(&self.queue),
            capacity: self.capacity,
            dropped_count: std::sync::atomic::AtomicU64::new(
                self.dropped_count.load(std::sync::atomic::Ordering::Relaxed)
            ),
        }
    }
}

/// Consumer for the ring buffer that can be used in different threads
pub struct TransactionConsumer {
    queue: Arc<ArrayQueue<ParsedTransaction>>,
}

impl TransactionConsumer {
    pub fn new(buffer: &TransactionRingBuffer) -> Self {
        Self {
            queue: Arc::clone(&buffer.queue),
        }
    }

    /// Consume transactions from the buffer
    pub fn consume(&self) -> Option<ParsedTransaction> {
        self.queue.pop()
    }

    /// Batch consume multiple transactions
    pub fn consume_batch(&self, max_count: usize) -> Vec<ParsedTransaction> {
        let mut transactions = Vec::with_capacity(max_count);
        
        for _ in 0..max_count {
            if let Some(tx) = self.queue.pop() {
                transactions.push(tx);
            } else {
                break;
            }
        }
        
        transactions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mev_core::{Transaction, TargetType};
    use chrono::Utc;

    fn create_test_transaction() -> ParsedTransaction {
        ParsedTransaction {
            transaction: Transaction {
                hash: "0x123".to_string(),
                from: "0x456".to_string(),
                to: Some("0x789".to_string()),
                value: "1000".to_string(),
                gas_price: "20000000000".to_string(),
                gas_limit: "21000".to_string(),
                nonce: 1,
                input: "0x".to_string(),
                timestamp: Utc::now(),
            },
            decoded_input: None,
            target_type: TargetType::Unknown,
            processing_time_ms: 5,
        }
    }

    #[test]
    fn test_ring_buffer_basic_operations() {
        let buffer = TransactionRingBuffer::new(10);
        
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
        assert_eq!(buffer.capacity(), 10);
        
        let tx = create_test_transaction();
        assert!(buffer.push(tx));
        
        assert_eq!(buffer.len(), 1);
        assert!(!buffer.is_empty());
        
        let popped = buffer.pop();
        assert!(popped.is_some());
        assert_eq!(popped.unwrap().transaction.hash, "0x123");
        
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
    }

    #[test]
    fn test_ring_buffer_overflow() {
        let buffer = TransactionRingBuffer::new(2);
        
        // Fill the buffer
        assert!(buffer.push(create_test_transaction()));
        assert!(buffer.push(create_test_transaction()));
        
        // This should fail and increment dropped count
        assert!(!buffer.push(create_test_transaction()));
        assert_eq!(buffer.dropped_count(), 1);
    }

    #[test]
    fn test_consumer() {
        let buffer = TransactionRingBuffer::new(10);
        let consumer = TransactionConsumer::new(&buffer);
        
        buffer.push(create_test_transaction());
        buffer.push(create_test_transaction());
        
        let batch = consumer.consume_batch(5);
        assert_eq!(batch.len(), 2);
    }
}
