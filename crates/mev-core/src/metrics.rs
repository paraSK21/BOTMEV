//! Metrics collection and Prometheus integration for MEV bot

use anyhow::Result;
use axum::{extract::State, http::StatusCode, response::Response, routing::get, Router};
use prometheus::{
    register_histogram_vec, register_int_counter_vec, register_int_gauge_vec,
    HistogramVec, IntCounterVec, IntGaugeVec, TextEncoder,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;
use tracing::info;

/// Latency statistics for performance tracking
#[derive(Debug, Clone)]
pub struct LatencyStats {
    pub median_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub mean_ms: f64,
    pub count: u64,
}

impl Default for LatencyStats {
    fn default() -> Self {
        Self {
            median_ms: 0.0,
            p95_ms: 0.0,
            p99_ms: 0.0,
            mean_ms: 0.0,
            count: 0,
        }
    }
}

/// Comprehensive metrics for the MEV bot system
#[derive(Debug, Clone)]
pub struct SystemMetrics {
    pub mempool: MempoolMetrics,
    pub simulation: SimulationMetrics,
    pub execution: ExecutionMetrics,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Mempool-specific metrics
#[derive(Debug, Clone)]
pub struct MempoolMetrics {
    pub transactions_per_second: f64,
    pub detection_latency_ms: LatencyStats,
    pub buffer_utilization: f64,
    pub dropped_transactions: u64,
    pub interesting_transactions: u64,
}

impl Default for MempoolMetrics {
    fn default() -> Self {
        Self {
            transactions_per_second: 0.0,
            detection_latency_ms: LatencyStats::default(),
            buffer_utilization: 0.0,
            dropped_transactions: 0,
            interesting_transactions: 0,
        }
    }
}

/// Simulation engine metrics
#[derive(Debug, Clone)]
pub struct SimulationMetrics {
    pub simulations_per_second: f64,
    pub decision_latency_ms: LatencyStats,
    pub successful_simulations: u64,
    pub failed_simulations: u64,
    pub profit_estimates_usd: f64,
}

impl Default for SimulationMetrics {
    fn default() -> Self {
        Self {
            simulations_per_second: 0.0,
            decision_latency_ms: LatencyStats::default(),
            successful_simulations: 0,
            failed_simulations: 0,
            profit_estimates_usd: 0.0,
        }
    }
}

/// Bundle execution metrics
#[derive(Debug, Clone)]
pub struct ExecutionMetrics {
    pub bundles_submitted: u64,
    pub bundles_included: u64,
    pub inclusion_rate: f64,
    pub execution_latency_ms: LatencyStats,
    pub realized_profit_usd: f64,
}

impl Default for ExecutionMetrics {
    fn default() -> Self {
        Self {
            bundles_submitted: 0,
            bundles_included: 0,
            inclusion_rate: 0.0,
            execution_latency_ms: LatencyStats::default(),
            realized_profit_usd: 0.0,
        }
    }
}

/// Prometheus metrics collector for the MEV bot
pub struct PrometheusMetrics {
    // Histograms for latency tracking
    pub detection_latency: HistogramVec,
    pub decision_latency: HistogramVec,
    pub execution_latency: HistogramVec,
    pub roundtrip_latency: HistogramVec,
    pub queue_time: HistogramVec,
    
    // Counters for events
    pub transactions_processed: IntCounterVec,
    pub simulations_total: IntCounterVec,
    pub bundles_submitted: IntCounterVec,
    pub bundles_included: IntCounterVec,
    pub transactions_dropped: IntCounterVec,
    
    // Gauges for current state
    pub buffer_utilization: IntGaugeVec,
    pub active_connections: IntGaugeVec,
    pub profit_estimates: IntGaugeVec,
    pub queue_length: IntGaugeVec,
}

impl PrometheusMetrics {
    pub fn new() -> Result<Self> {
        let detection_latency = register_histogram_vec!(
            "mev_detection_latency_seconds",
            "Time from transaction broadcast to local detection",
            &["source", "target_type"],
            vec![0.001, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0]
        )?;

        let decision_latency = register_histogram_vec!(
            "mev_decision_latency_seconds", 
            "Time from detection to bundle decision",
            &["strategy", "outcome"],
            vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.2, 0.5, 1.0]
        )?;

        let execution_latency = register_histogram_vec!(
            "mev_execution_latency_seconds",
            "Time from bundle creation to submission",
            &["submission_type", "result"],
            vec![0.001, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0]
        )?;

        let roundtrip_latency = register_histogram_vec!(
            "mev_roundtrip_latency_seconds",
            "Time from transaction broadcast to local detection (t_broadcast vs t_local_seen)",
            &["source", "network"],
            vec![0.001, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0]
        )?;

        let queue_time = register_histogram_vec!(
            "mev_queue_time_seconds",
            "Time transactions spend in processing queues",
            &["queue_type", "priority"],
            vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0]
        )?;

        let transactions_processed = register_int_counter_vec!(
            "mev_transactions_processed_total",
            "Total number of transactions processed",
            &["source", "target_type", "interesting"]
        )?;

        let simulations_total = register_int_counter_vec!(
            "mev_simulations_total",
            "Total number of simulations performed",
            &["strategy", "result"]
        )?;

        let bundles_submitted = register_int_counter_vec!(
            "mev_bundles_submitted_total",
            "Total number of bundles submitted",
            &["strategy", "submission_type"]
        )?;

        let bundles_included = register_int_counter_vec!(
            "mev_bundles_included_total",
            "Total number of bundles included in blocks",
            &["strategy", "block_position"]
        )?;

        let transactions_dropped = register_int_counter_vec!(
            "mev_transactions_dropped_total",
            "Total number of transactions dropped due to backpressure",
            &["reason", "queue_type"]
        )?;

        let buffer_utilization = register_int_gauge_vec!(
            "mev_buffer_utilization_percent",
            "Current buffer utilization percentage",
            &["buffer_type"]
        )?;

        let active_connections = register_int_gauge_vec!(
            "mev_active_connections",
            "Number of active network connections",
            &["connection_type", "endpoint"]
        )?;

        let profit_estimates = register_int_gauge_vec!(
            "mev_profit_estimates_wei",
            "Current profit estimates in wei",
            &["strategy", "token_pair"]
        )?;

        let queue_length = register_int_gauge_vec!(
            "mev_queue_length",
            "Current number of items in processing queues",
            &["queue_type"]
        )?;

        Ok(Self {
            detection_latency,
            decision_latency,
            execution_latency,
            roundtrip_latency,
            queue_time,
            transactions_processed,
            simulations_total,
            bundles_submitted,
            bundles_included,
            transactions_dropped,
            buffer_utilization,
            active_connections,
            profit_estimates,
            queue_length,
        })
    }

    /// Record detection latency for a transaction
    pub fn record_detection_latency(&self, duration: Duration, source: &str, target_type: &str) {
        self.detection_latency
            .with_label_values(&[source, target_type])
            .observe(duration.as_secs_f64());
    }

    /// Record decision latency for a strategy
    pub fn record_decision_latency(&self, duration: Duration, strategy: &str, outcome: &str) {
        self.decision_latency
            .with_label_values(&[strategy, outcome])
            .observe(duration.as_secs_f64());
    }

    /// Record execution latency
    pub fn record_execution_latency(&self, duration: Duration, submission_type: &str, result: &str) {
        self.execution_latency
            .with_label_values(&[submission_type, result])
            .observe(duration.as_secs_f64());
    }

    /// Increment transaction processed counter
    pub fn inc_transactions_processed(&self, source: &str, target_type: &str, interesting: bool) {
        let interesting_str = if interesting { "true" } else { "false" };
        self.transactions_processed
            .with_label_values(&[source, target_type, interesting_str])
            .inc();
    }

    /// Increment simulation counter
    pub fn inc_simulations(&self, strategy: &str, result: &str) {
        self.simulations_total
            .with_label_values(&[strategy, result])
            .inc();
    }

    /// Update buffer utilization gauge
    pub fn set_buffer_utilization(&self, buffer_type: &str, utilization_percent: i64) {
        self.buffer_utilization
            .with_label_values(&[buffer_type])
            .set(utilization_percent);
    }

    /// Update active connections gauge
    pub fn set_active_connections(&self, connection_type: &str, endpoint: &str, count: i64) {
        self.active_connections
            .with_label_values(&[connection_type, endpoint])
            .set(count);
    }

    /// Record roundtrip latency (t_broadcast vs t_local_seen)
    pub fn record_roundtrip_latency(&self, duration: Duration, source: &str, network: &str) {
        self.roundtrip_latency
            .with_label_values(&[source, network])
            .observe(duration.as_secs_f64());
    }

    /// Record queue time for transactions
    pub fn record_queue_time(&self, duration: Duration, queue_type: &str, priority: &str) {
        self.queue_time
            .with_label_values(&[queue_type, priority])
            .observe(duration.as_secs_f64());
    }

    /// Increment dropped transactions counter
    pub fn inc_transactions_dropped(&self, reason: &str, queue_type: &str) {
        self.transactions_dropped
            .with_label_values(&[reason, queue_type])
            .inc();
    }

    /// Update queue length gauge
    pub fn set_queue_length(&self, queue_type: &str, length: i64) {
        self.queue_length
            .with_label_values(&[queue_type])
            .set(length);
    }
}

/// Metrics server for exposing Prometheus metrics
pub struct MetricsServer {
    metrics: Arc<PrometheusMetrics>,
    port: u16,
}

impl MetricsServer {
    pub fn new(metrics: Arc<PrometheusMetrics>, port: u16) -> Self {
        Self { metrics, port }
    }

    /// Start the metrics server
    pub async fn start(&self) -> Result<()> {
        let app = Router::new()
            .route("/metrics", get(metrics_handler))
            .route("/health", get(health_handler))
            .layer(CorsLayer::permissive())
            .with_state(self.metrics.clone());

        let addr = format!("0.0.0.0:{}", self.port);
        let listener = TcpListener::bind(&addr).await?;
        
        info!("Metrics server starting on {}", addr);
        
        axum::serve(listener, app).await?;
        
        Ok(())
    }
}

/// Handler for /metrics endpoint
async fn metrics_handler(State(_metrics): State<Arc<PrometheusMetrics>>) -> Result<Response<String>, StatusCode> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    
    match encoder.encode_to_string(&metric_families) {
        Ok(output) => {
            let response = Response::builder()
                .header("content-type", "text/plain; version=0.0.4")
                .body(output)
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(response)
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Handler for /health endpoint
async fn health_handler() -> &'static str {
    "OK"
}

/// Performance timer for measuring operation latency
pub struct PerformanceTimer {
    start: Instant,
    label: String,
}

impl PerformanceTimer {
    pub fn new(label: String) -> Self {
        Self {
            start: Instant::now(),
            label,
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn elapsed_ms(&self) -> f64 {
        self.elapsed().as_secs_f64() * 1000.0
    }
}

impl Drop for PerformanceTimer {
    fn drop(&mut self) {
        let elapsed = self.elapsed_ms();
        tracing::debug!(
            timer = %self.label,
            elapsed_ms = elapsed,
            "Performance timer completed"
        );
    }
}

/// Latency histogram for calculating percentiles
#[derive(Debug)]
pub struct LatencyHistogram {
    samples: Arc<Mutex<Vec<f64>>>,
    max_samples: usize,
}

impl LatencyHistogram {
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: Arc::new(Mutex::new(Vec::with_capacity(max_samples))),
            max_samples,
        }
    }

    pub fn record(&self, latency_ms: f64) {
        if let Ok(mut samples) = self.samples.lock() {
            if samples.len() >= self.max_samples {
                samples.remove(0); // Remove oldest sample
            }
            samples.push(latency_ms);
        }
    }

    pub fn get_stats(&self) -> LatencyStats {
        if let Ok(mut samples) = self.samples.lock() {
            if samples.is_empty() {
                return LatencyStats::default();
            }

            samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let count = samples.len();
            
            let median_ms = if count % 2 == 0 {
                (samples[count / 2 - 1] + samples[count / 2]) / 2.0
            } else {
                samples[count / 2]
            };

            let p95_idx = ((count as f64) * 0.95) as usize;
            let p95_ms = samples[p95_idx.min(count - 1)];

            let p99_idx = ((count as f64) * 0.99) as usize;
            let p99_ms = samples[p99_idx.min(count - 1)];

            let mean_ms = samples.iter().sum::<f64>() / count as f64;

            LatencyStats {
                median_ms,
                p95_ms,
                p99_ms,
                mean_ms,
                count: count as u64,
            }
        } else {
            LatencyStats::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_latency_histogram() {
        let histogram = LatencyHistogram::new(100);
        
        // Record some sample latencies
        histogram.record(10.0);
        histogram.record(20.0);
        histogram.record(15.0);
        histogram.record(25.0);
        histogram.record(30.0);

        let stats = histogram.get_stats();
        assert_eq!(stats.count, 5);
        assert_eq!(stats.median_ms, 20.0);
        assert!(stats.mean_ms > 0.0);
    }

    #[test]
    fn test_performance_timer() {
        let timer = PerformanceTimer::new("test".to_string());
        std::thread::sleep(Duration::from_millis(1));
        assert!(timer.elapsed_ms() >= 1.0);
    }

    #[tokio::test]
    async fn test_prometheus_metrics_creation() {
        let metrics = PrometheusMetrics::new();
        assert!(metrics.is_ok());
    }
}