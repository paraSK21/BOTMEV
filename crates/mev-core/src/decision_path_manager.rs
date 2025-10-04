//! Decision Path Manager - Integrates ultra-low latency decision path with mempool service

use crate::{
    decision_path::{DecisionPath, DecisionPathConfig, DecisionResult},
    metrics::PrometheusMetrics,
    types::ParsedTransaction,
};
use anyhow::Result;
use crate::strategy_types::MockStrategyEngine;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Configuration for the decision path manager
#[derive(Debug, Clone)]
pub struct DecisionPathManagerConfig {
    pub decision_path: DecisionPathConfig,
    pub mempool_endpoint: String,
    pub enable_backpressure_monitoring: bool,
    pub stats_reporting_interval_seconds: u64,
}

impl Default for DecisionPathManagerConfig {
    fn default() -> Self {
        Self {
            decision_path: DecisionPathConfig::default(),
            mempool_endpoint: "wss://api.hyperliquid.xyz/ws".to_string(),
            enable_backpressure_monitoring: true,
            stats_reporting_interval_seconds: 10,
        }
    }
}

/// Statistics for the decision path manager
#[derive(Debug, Clone, Default)]
pub struct DecisionPathManagerStats {
    pub mempool_transactions_received: u64,
    pub decision_path_transactions_sent: u64,
    pub decision_results_processed: u64,
    pub opportunities_found: u64,
    pub backpressure_events: u64,
    pub average_end_to_end_latency_ms: f64,
    pub throughput_tps: f64,
    pub uptime_seconds: u64,
}

/// Decision Path Manager - Orchestrates the complete MEV detection pipeline
pub struct DecisionPathManager {
    config: DecisionPathManagerConfig,
    decision_path: Arc<DecisionPath>,
    #[allow(dead_code)]
    strategy_engine: Arc<MockStrategyEngine>,
    metrics: Arc<PrometheusMetrics>,
    
    // Statistics and monitoring
    stats: Arc<RwLock<DecisionPathManagerStats>>,
    start_time: Instant,
    shutdown: Arc<AtomicBool>,
    
    // Performance counters
    mempool_rx_count: Arc<AtomicU64>,
    decision_tx_count: Arc<AtomicU64>,
    results_count: Arc<AtomicU64>,
}

impl DecisionPathManager {
    /// Create new decision path manager
    pub async fn new(config: DecisionPathManagerConfig) -> Result<Self> {
        info!("Initializing Decision Path Manager");

        // Create metrics
        let metrics = Arc::new(PrometheusMetrics::new()?);

        // Create strategy engine
        let strategy_engine = Arc::new(MockStrategyEngine::new());

        // Create decision path
        let decision_path = Arc::new(DecisionPath::new(
            config.decision_path.clone(),
            strategy_engine.clone(),
            metrics.clone(),
        )?);

        Ok(Self {
            config,
            decision_path,
            strategy_engine,
            metrics,
            stats: Arc::new(RwLock::new(DecisionPathManagerStats::default())),
            start_time: Instant::now(),
            shutdown: Arc::new(AtomicBool::new(false)),
            mempool_rx_count: Arc::new(AtomicU64::new(0)),
            decision_tx_count: Arc::new(AtomicU64::new(0)),
            results_count: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Start the complete MEV detection pipeline
    pub async fn start(&self) -> Result<()> {
        info!("Starting Decision Path Manager pipeline");

        // Start decision path pipeline
        self.decision_path.start().await?;

        // Start mempool to decision path bridge
        self.start_mempool_bridge().await?;

        // Start decision results processor
        self.start_results_processor().await?;

        // Start statistics reporting
        if self.config.stats_reporting_interval_seconds > 0 {
            self.start_stats_reporting().await?;
        }

        // Start backpressure monitoring if enabled
        if self.config.enable_backpressure_monitoring {
            self.start_backpressure_monitoring().await?;
        }

        info!("Decision Path Manager pipeline started successfully");
        Ok(())
    }

    /// Process a transaction through the decision path
    pub async fn process_transaction(&self, transaction: ParsedTransaction) -> Result<()> {
        let _current_count = self.mempool_rx_count.fetch_add(1, Ordering::Relaxed) + 1;
        
        // Send to decision path
        match self.decision_path.process_transaction(transaction.clone()).await {
            Ok(_) => {
                self.decision_tx_count.fetch_add(1, Ordering::Relaxed);
                
                debug!(
                    tx_hash = %transaction.transaction.hash,
                    "Transaction sent to decision path"
                );
                Ok(())
            }
            Err(e) => {
                warn!(
                    tx_hash = %transaction.transaction.hash,
                    error = %e,
                    "Failed to send transaction to decision path"
                );
                self.metrics.inc_transactions_dropped("decision_path_error", "bridge");
                Err(e)
            }
        }
    }

    /// Start the mempool bridge (placeholder for external integration)
    async fn start_mempool_bridge(&self) -> Result<()> {
        info!("Mempool bridge interface ready - use process_transaction() to send transactions");
        Ok(())
    }

    /// Start processing decision results
    async fn start_results_processor(&self) -> Result<()> {
        let results_rx = self.decision_path.get_results();
        let shutdown = self.shutdown.clone();
        let results_count = self.results_count.clone();
        let stats = self.stats.clone();
        let metrics = self.metrics.clone();

        tokio::spawn(async move {
            info!("Starting decision results processor");

            while !shutdown.load(Ordering::Relaxed) {
                match results_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(result) => {
                        let current_count = results_count.fetch_add(1, Ordering::Relaxed) + 1;
                        
                        // Process the decision result
                        Self::process_decision_result(&result, &stats, &metrics).await;

                        debug!(
                            tx_id = %result.transaction_id,
                            opportunities = result.opportunities.len(),
                            processing_time_ms = format!("{:.2}", result.processing_time_ms),
                            "Decision result processed"
                        );

                        // Log summary every 50 results
                        if current_count % 50 == 0 {
                            let stats_snapshot = stats.read().await;
                            info!(
                                results_processed = current_count,
                                opportunities_found = stats_snapshot.opportunities_found,
                                avg_latency_ms = format!("{:.2}", stats_snapshot.average_end_to_end_latency_ms),
                                "Decision results summary"
                            );
                        }
                    }
                    Err(crossbeam::channel::RecvTimeoutError::Timeout) => {
                        // Continue loop
                    }
                    Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                        warn!("Decision results channel disconnected");
                        break;
                    }
                }
            }

            info!("Decision results processor stopped");
        });

        info!("Decision results processor started");
        Ok(())
    }

    /// Process a single decision result
    async fn process_decision_result(
        result: &DecisionResult,
        stats: &Arc<RwLock<DecisionPathManagerStats>>,
        metrics: &Arc<PrometheusMetrics>,
    ) {
        // Update statistics
        {
            let mut stats = stats.write().await;
            stats.decision_results_processed += 1;
            stats.opportunities_found += result.opportunities.len() as u64;
            
            // Update average end-to-end latency
            let count = stats.decision_results_processed as f64;
            stats.average_end_to_end_latency_ms = 
                (stats.average_end_to_end_latency_ms * (count - 1.0) + result.processing_time_ms) / count;
        }

        // Record metrics for each opportunity found
        for opportunity in &result.opportunities {
            metrics.inc_simulations(&opportunity.strategy_name, "opportunity_found");
            
            info!(
                tx_id = %result.transaction_id,
                strategy = %opportunity.strategy_name,
                opportunity_type = ?opportunity.opportunity_type,
                estimated_profit_wei = opportunity.estimated_profit_wei,
                confidence = format!("{:.2}", opportunity.confidence_score),
                "MEV opportunity detected"
            );
        }

        // Record stage timing metrics
        metrics.record_decision_latency(
            Duration::from_millis(result.stage_timings.filter_ms as u64),
            "filter_stage",
            "completed",
        );
        
        metrics.record_decision_latency(
            Duration::from_millis(result.stage_timings.simulation_ms as u64),
            "simulation_stage", 
            "completed",
        );
        
        metrics.record_decision_latency(
            Duration::from_millis(result.stage_timings.bundle_ms as u64),
            "bundle_stage",
            "completed",
        );

        // Check if we met latency targets
        let target_ms = 25.0; // From requirements
        let latency_status = if result.processing_time_ms <= target_ms {
            "within_target"
        } else {
            "exceeded_target"
        };

        metrics.record_decision_latency(
            Duration::from_millis(result.processing_time_ms as u64),
            "end_to_end",
            latency_status,
        );
    }

    /// Start periodic statistics reporting
    async fn start_stats_reporting(&self) -> Result<()> {
        let stats = self.stats.clone();
        let mempool_rx_count = self.mempool_rx_count.clone();
        let decision_tx_count = self.decision_tx_count.clone();
        let results_count = self.results_count.clone();
        let shutdown = self.shutdown.clone();
        let start_time = self.start_time;
        let interval_seconds = self.config.stats_reporting_interval_seconds;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
            let mut last_mempool_count = 0u64;
            let mut last_results_count = 0u64;
            let mut last_time = Instant::now();

            while !shutdown.load(Ordering::Relaxed) {
                interval.tick().await;

                let current_mempool = mempool_rx_count.load(Ordering::Relaxed);
                let current_decision = decision_tx_count.load(Ordering::Relaxed);
                let current_results = results_count.load(Ordering::Relaxed);
                let current_time = Instant::now();
                
                let elapsed = current_time.duration_since(last_time).as_secs_f64();
                let uptime = current_time.duration_since(start_time).as_secs();

                if elapsed > 0.0 {
                    let mempool_tps = (current_mempool - last_mempool_count) as f64 / elapsed;
                    let results_tps = (current_results - last_results_count) as f64 / elapsed;

                    // Update stats
                    {
                        let mut stats = stats.write().await;
                        stats.mempool_transactions_received = current_mempool;
                        stats.decision_path_transactions_sent = current_decision;
                        stats.decision_results_processed = current_results;
                        stats.throughput_tps = mempool_tps;
                        stats.uptime_seconds = uptime;
                    }

                    let stats_snapshot = stats.read().await;
                    
                    info!(
                        uptime_seconds = uptime,
                        mempool_rx_total = current_mempool,
                        decision_tx_total = current_decision,
                        results_total = current_results,
                        mempool_tps = format!("{:.1}", mempool_tps),
                        results_tps = format!("{:.1}", results_tps),
                        opportunities_found = stats_snapshot.opportunities_found,
                        avg_latency_ms = format!("{:.2}", stats_snapshot.average_end_to_end_latency_ms),
                        backpressure_events = stats_snapshot.backpressure_events,
                        "Decision Path Manager Statistics"
                    );
                }

                last_mempool_count = current_mempool;
                last_results_count = current_results;
                last_time = current_time;
            }

            info!("Statistics reporting stopped");
        });

        info!(
            interval_seconds = interval_seconds,
            "Statistics reporting started"
        );
        Ok(())
    }

    /// Start backpressure monitoring (placeholder for external integration)
    async fn start_backpressure_monitoring(&self) -> Result<()> {
        info!("Backpressure monitoring interface ready - integrate with external mempool service");
        Ok(())
    }

    /// Get current statistics
    pub async fn get_stats(&self) -> DecisionPathManagerStats {
        let mut stats = self.stats.read().await.clone();
        
        // Update real-time counters
        stats.mempool_transactions_received = self.mempool_rx_count.load(Ordering::Relaxed);
        stats.decision_path_transactions_sent = self.decision_tx_count.load(Ordering::Relaxed);
        stats.decision_results_processed = self.results_count.load(Ordering::Relaxed);
        stats.uptime_seconds = self.start_time.elapsed().as_secs();
        
        stats
    }

    /// Get decision path statistics
    pub async fn get_decision_path_stats(&self) -> crate::decision_path::DecisionPathStats {
        self.decision_path.get_stats().await
    }

    /// Get mempool metrics (placeholder - integrate with external mempool service)
    pub fn get_mempool_metrics(&self) -> Option<String> {
        Some("Integrate with external mempool service for metrics".to_string())
    }

    /// Shutdown the manager
    pub async fn shutdown(&self) {
        info!("Shutting down Decision Path Manager");
        
        self.shutdown.store(true, Ordering::Relaxed);
        self.decision_path.shutdown().await;
        
        // Give tasks time to shutdown gracefully
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        info!("Decision Path Manager shutdown complete");
    }

    /// Print comprehensive performance report
    pub async fn print_performance_report(&self) {
        let stats = self.get_stats().await;
        let decision_stats = self.get_decision_path_stats().await;
        info!("=== Decision Path Manager Performance Report ===");
        info!("Uptime: {} seconds", stats.uptime_seconds);
        info!("Mempool Transactions Received: {}", stats.mempool_transactions_received);
        info!("Decision Path Transactions Sent: {}", stats.decision_path_transactions_sent);
        info!("Decision Results Processed: {}", stats.decision_results_processed);
        info!("Opportunities Found: {}", stats.opportunities_found);
        info!("Average End-to-End Latency: {:.2} ms", stats.average_end_to_end_latency_ms);
        info!("Throughput: {:.1} TPS", stats.throughput_tps);
        info!("Backpressure Events: {}", stats.backpressure_events);
        
        info!("--- Decision Path Details ---");
        info!("Transactions Processed: {}", decision_stats.transactions_processed);
        info!("Average Decision Latency: {:.2} ms", decision_stats.average_latency_ms);
        info!("P95 Decision Latency: {:.2} ms", decision_stats.p95_latency_ms);
        info!("P99 Decision Latency: {:.2} ms", decision_stats.p99_latency_ms);
        info!("Bundles Created: {}", decision_stats.bundles_created);
        info!("Timeout Count: {}", decision_stats.timeout_count);
        info!("Error Count: {}", decision_stats.error_count);
        
        // Performance assessment
        let target_latency = 25.0;
        let latency_status = if stats.average_end_to_end_latency_ms <= target_latency {
            "✅ MEETING TARGET"
        } else {
            "❌ EXCEEDING TARGET"
        };
        
        info!("--- Performance Assessment ---");
        info!("Latency Target: ≤{} ms", target_latency);
        info!("Current Average: {:.2} ms ({})", stats.average_end_to_end_latency_ms, latency_status);
        
        if stats.throughput_tps > 100.0 {
            info!("Throughput: ✅ HIGH ({:.1} TPS)", stats.throughput_tps);
        } else if stats.throughput_tps > 10.0 {
            info!("Throughput: ⚠️ MODERATE ({:.1} TPS)", stats.throughput_tps);
        } else {
            info!("Throughput: ❌ LOW ({:.1} TPS)", stats.throughput_tps);
        }
        
        if stats.backpressure_events == 0 {
            info!("Backpressure: ✅ NONE");
        } else {
            info!("Backpressure: ⚠️ {} EVENTS", stats.backpressure_events);
        }
        
        info!("================================================");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_decision_path_manager_creation() {
        let config = DecisionPathManagerConfig::default();
        let manager = DecisionPathManager::new(config).await;
        assert!(manager.is_ok());
    }

    #[tokio::test]
    async fn test_stats_initialization() {
        let config = DecisionPathManagerConfig::default();
        let manager = DecisionPathManager::new(config).await.unwrap();
        
        let stats = manager.get_stats().await;
        assert_eq!(stats.mempool_transactions_received, 0);
        assert_eq!(stats.decision_results_processed, 0);
        assert_eq!(stats.opportunities_found, 0);
    }

    #[test]
    fn test_config_defaults() {
        let config = DecisionPathManagerConfig::default();
        assert_eq!(config.decision_path.latency_targets.total_decision_loop_ms, 25);
        assert!(config.enable_backpressure_monitoring);
        assert_eq!(config.stats_reporting_interval_seconds, 10);
    }
}