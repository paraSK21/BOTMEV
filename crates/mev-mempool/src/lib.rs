//! MEV Mempool - Mempool monitoring and transaction analysis

pub mod abi_decoder;
pub mod backpressure;
pub mod filter_config;
pub mod filters;
pub mod optimized_abi_decoder;
pub mod parser;
pub mod ring_buffer;
pub mod websocket;

pub use abi_decoder::*;
pub use backpressure::*;
pub use filter_config::*;
pub use filters::*;
pub use optimized_abi_decoder::*;
pub use parser::*;
pub use ring_buffer::*;
pub use websocket::*;

use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use mev_core::{
    LatencyHistogram, MempoolMetrics, ParsedTransaction, PerformanceTimer, PrometheusMetrics,
    Transaction,
};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use tracing::{error, info, warn};

/// Mempool monitoring mode
#[derive(Debug, Clone, PartialEq)]
pub enum MempoolMode {
    Auto,       // Try WebSocket, fallback to polling
    WebSocket,  // WebSocket only
    Polling,    // HTTP polling only
}

/// Mempool service for monitoring pending transactions
#[derive(Clone)]
pub struct MempoolService {
    pub endpoint: String,
    pub rpc_url: String,
    pub mode: MempoolMode,
    pub polling_interval_ms: u64,
    pub websocket_timeout_secs: u64,
    parser: Arc<TransactionParser>,
    ring_buffer: Arc<TransactionRingBuffer>,
    metrics: Arc<PrometheusMetrics>,
    latency_histogram: Arc<LatencyHistogram>,
    seen_txs: Arc<Mutex<HashSet<String>>>,
}

impl MempoolService {
    pub fn new(endpoint: String, rpc_url: String) -> Result<Self> {
        let metrics = Arc::new(PrometheusMetrics::new()?);
        let latency_histogram = Arc::new(LatencyHistogram::new(1000)); // Keep last 1000 samples
        
        Ok(Self {
            endpoint,
            rpc_url,
            mode: MempoolMode::Auto,
            polling_interval_ms: 300,
            websocket_timeout_secs: 3,
            parser: Arc::new(TransactionParser::new()),
            ring_buffer: Arc::new(TransactionRingBuffer::new(10000)), // 10k transaction buffer
            metrics,
            latency_histogram,
            seen_txs: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    pub fn with_mode(mut self, mode: MempoolMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_polling_interval(mut self, interval_ms: u64) -> Self {
        self.polling_interval_ms = interval_ms;
        self
    }

    pub fn with_websocket_timeout(mut self, timeout_secs: u64) -> Self {
        self.websocket_timeout_secs = timeout_secs;
        self
    }

    pub fn get_metrics(&self) -> Arc<PrometheusMetrics> {
        self.metrics.clone()
    }

    /// Start the mempool monitoring service
    pub async fn start(&self) -> Result<()> {
        info!("Starting mempool service with mode: {:?}", self.mode);

        match self.mode {
            MempoolMode::Auto => {
                info!("Auto mode: Attempting WebSocket first...");
                match self.try_websocket_mode().await {
                    Ok(_) => {
                        warn!("WebSocket mode stopped, restarting...");
                    }
                    Err(e) => {
                        warn!("WebSocket mode failed: {}", e);
                        info!("Falling back to HTTP polling mode");
                        self.start_polling_mode().await?;
                    }
                }
            }
            MempoolMode::WebSocket => {
                info!("WebSocket-only mode");
                self.start_websocket_mode().await?;
            }
            MempoolMode::Polling => {
                info!("Polling-only mode");
                self.start_polling_mode().await?;
            }
        }

        Ok(())
    }

    /// Try WebSocket mode (for auto mode)
    async fn try_websocket_mode(&self) -> Result<()> {
        let ws_client = WebSocketRpcClient::new(self.endpoint.clone());
        
        // Single attempt for auto mode
        self.run_mempool_monitor(&ws_client).await
    }

    /// Start WebSocket mode with retry loop
    async fn start_websocket_mode(&self) -> Result<()> {
        let ws_client = WebSocketRpcClient::new(self.endpoint.clone());
        
        loop {
            match self.run_mempool_monitor(&ws_client).await {
                Ok(_) => {
                    warn!("Mempool monitor stopped unexpectedly, restarting...");
                }
                Err(e) => {
                    error!("Mempool monitor error: {}, restarting in 5 seconds...", e);
                    sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    /// Start HTTP polling mode
    async fn start_polling_mode(&self) -> Result<()> {
        info!("Starting HTTP polling mode on {}", self.rpc_url);
        info!("Polling interval: {}ms", self.polling_interval_ms);

        let client = reqwest::Client::new();
        let mut request_id = 1u64;

        loop {
            match self.poll_pending_transactions(&client, &mut request_id).await {
                Ok(tx_count) => {
                    if tx_count > 0 {
                        info!("Polled {} pending transactions", tx_count);
                    }
                }
                Err(e) => {
                    error!("Polling error: {}", e);
                }
            }

            sleep(Duration::from_millis(self.polling_interval_ms)).await;
        }
    }

    /// Poll for pending transactions via HTTP RPC
    async fn poll_pending_transactions(
        &self,
        client: &reqwest::Client,
        request_id: &mut u64,
    ) -> Result<usize> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": *request_id,
            "method": "eth_getBlockByNumber",
            "params": ["pending", true]
        });

        *request_id += 1;

        let response = client
            .post(&self.rpc_url)
            .json(&request)
            .send()
            .await
            .map_err(|e| anyhow!("HTTP request failed: {}", e))?;

        let response_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| anyhow!("Failed to parse response: {}", e))?;

        if let Some(error) = response_json.get("error") {
            return Err(anyhow!("RPC error: {}", error));
        }

        let block = response_json
            .get("result")
            .ok_or_else(|| anyhow!("No result in response"))?;

        let transactions = block
            .get("transactions")
            .and_then(|t| t.as_array())
            .ok_or_else(|| anyhow!("No transactions array in block"))?;

        let mut new_tx_count = 0;
        let mut seen_txs = self.seen_txs.lock().await;

        for tx in transactions {
            if let Some(hash) = tx.get("hash").and_then(|h| h.as_str()) {
                // Check if we've already seen this transaction
                if seen_txs.insert(hash.to_string()) {
                    // New transaction
                    self.process_transaction(tx.clone()).await;
                    new_tx_count += 1;
                }
            }
        }

        // Cleanup: Keep only last 10k transaction hashes
        if seen_txs.len() > 10000 {
            let to_remove: Vec<String> = seen_txs.iter().take(seen_txs.len() - 10000).cloned().collect();
            for hash in to_remove {
                seen_txs.remove(&hash);
            }
        }

        Ok(new_tx_count)
    }

    /// Run the main mempool monitoring loop
    async fn run_mempool_monitor(&self, ws_client: &WebSocketRpcClient) -> Result<()> {
        // Connect to WebSocket
        let mut ws_stream = ws_client.subscribe_pending_transactions().await?;
        
        // Send subscription request with timeout
        let subscription_id = ws_client
            .send_subscribe_request(&mut ws_stream, self.websocket_timeout_secs)
            .await?;
        info!("Subscribed to pending transactions with ID: {}", subscription_id);

        let mut processed_count = 0u64;
        let mut last_stats_time = std::time::Instant::now();

        // Process incoming messages
        while let Some(message) = ws_stream.next().await {
            match message {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    if let Ok(Some(tx_data)) = ws_client.parse_transaction_notification(&text) {
                        self.process_transaction(tx_data).await;
                        processed_count += 1;

                        // Print stats every 100 transactions
                        if processed_count % 100 == 0 {
                            let elapsed = last_stats_time.elapsed();
                            let tps = 100.0 / elapsed.as_secs_f64();
                            
                            info!(
                                "Processed {} transactions, TPS: {:.2}, Buffer: {}/{} ({:.1}%), Dropped: {}",
                                processed_count,
                                tps,
                                self.ring_buffer.len(),
                                self.ring_buffer.capacity(),
                                self.ring_buffer.utilization(),
                                self.ring_buffer.dropped_count()
                            );
                            
                            last_stats_time = std::time::Instant::now();
                        }
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                    warn!("WebSocket connection closed");
                    break;
                }
                Ok(tokio_tungstenite::tungstenite::Message::Ping(data)) => {
                    // Respond to ping with pong
                    if let Err(e) = ws_stream.send(tokio_tungstenite::tungstenite::Message::Pong(data)).await {
                        error!("Failed to send pong: {}", e);
                    }
                }
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                }
                _ => {
                    // Ignore other message types
                }
            }
        }

        Err(anyhow!("WebSocket stream ended"))
    }

    /// Process a single transaction
    async fn process_transaction(&self, tx_data: serde_json::Value) {
        let processing_timer = PerformanceTimer::new("transaction_processing".to_string());
        
        match self.parser.parse_transaction(&tx_data) {
            Ok(parsed_tx) => {
                let processing_time_ms = processing_timer.elapsed_ms();
                
                // Record metrics
                let target_type = format!("{:?}", parsed_tx.target_type);
                
                let is_interesting = self.parser.is_interesting_transaction(&parsed_tx);
                
                // Record detection latency
                self.latency_histogram.record(processing_time_ms);
                self.metrics.record_detection_latency(
                    processing_timer.elapsed(),
                    "websocket",
                    &target_type,
                );
                
                // Increment transaction counter
                self.metrics.inc_transactions_processed(
                    "websocket",
                    &target_type,
                    is_interesting,
                );

                // Log interesting transactions
                if is_interesting {
                    self.log_parsed_transaction(&parsed_tx);
                }

                // Push to ring buffer for downstream processing
                if !self.ring_buffer.push(parsed_tx) {
                    warn!("Ring buffer full, transaction dropped");
                }
                
                // Update buffer utilization metrics
                let utilization = (self.ring_buffer.utilization() * 100.0) as i64;
                self.metrics.set_buffer_utilization("mempool", utilization);
            }
            Err(e) => {
                error!("Failed to parse transaction: {}", e);
            }
        }
    }

    /// Log parsed transaction with structured JSON
    fn log_parsed_transaction(&self, parsed_tx: &ParsedTransaction) {
        let tx = &parsed_tx.transaction;
        
        info!(
            target: "mempool_transaction",
            tx_hash = %tx.hash,
            from = %tx.from,
            to = ?tx.to,
            value = %tx.value,
            gas_price = %tx.gas_price,
            gas_limit = %tx.gas_limit,
            nonce = tx.nonce,
            target_type = ?parsed_tx.target_type,
            function_name = ?parsed_tx.decoded_input.as_ref().map(|d| &d.function_name),
            function_signature = ?parsed_tx.decoded_input.as_ref().map(|d| &d.function_signature),
            processing_time_ms = parsed_tx.processing_time_ms,
            timestamp = %tx.timestamp,
            "Parsed interesting transaction"
        );
    }

    /// Get a consumer for the ring buffer
    pub fn get_consumer(&self) -> TransactionConsumer {
        TransactionConsumer::new(&self.ring_buffer)
    }

    /// Get current buffer statistics
    pub fn get_buffer_stats(&self) -> (usize, usize, f64, u64) {
        (
            self.ring_buffer.len(),
            self.ring_buffer.capacity(),
            self.ring_buffer.utilization(),
            self.ring_buffer.dropped_count(),
        )
    }

    /// Get current mempool metrics
    pub fn get_mempool_metrics(&self) -> MempoolMetrics {
        let latency_stats = self.latency_histogram.get_stats();
        let (_len, _capacity, utilization, dropped) = self.get_buffer_stats();
        
        MempoolMetrics {
            transactions_per_second: 0.0, // This would be calculated over time
            detection_latency_ms: latency_stats,
            buffer_utilization: utilization,
            dropped_transactions: dropped,
            interesting_transactions: 0, // This would be tracked separately
        }
    }

    pub async fn get_pending_transactions(&self) -> Result<Vec<Transaction>> {
        // This method is now deprecated in favor of the streaming approach
        warn!("get_pending_transactions is deprecated, use streaming via start() instead");
        Ok(vec![])
    }
}
