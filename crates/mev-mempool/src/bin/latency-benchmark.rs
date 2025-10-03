//! Latency benchmarking tool for mempool ingestion
//! 
//! This tool measures roundtrip metrics (t_broadcast vs t_local_seen) and provides
//! baseline measurements for 30-minute HyperEVM mempool capture.

use anyhow::Result;
use chrono::{DateTime, Utc};
use mev_core::{LatencyHistogram, PerformanceTimer, PrometheusMetrics};
use mev_mempool::{MempoolService, WebSocketRpcClient};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    fs::File,
    io::AsyncWriteExt,
    signal,
    time::{interval, sleep},
};
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyBenchmarkResult {
    pub timestamp: DateTime<Utc>,
    pub duration_minutes: f64,
    pub total_transactions: u64,
    pub transactions_per_second: f64,
    pub detection_latency_ms: LatencyStats,
    pub roundtrip_latency_ms: LatencyStats,
    pub buffer_stats: BufferStats,
    pub dropped_transactions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyStats {
    pub median: f64,
    pub p95: f64,
    pub p99: f64,
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferStats {
    pub utilization_percent: f64,
    pub max_utilization_percent: f64,
    pub capacity: usize,
    pub peak_length: usize,
}

/// Micro-benchmarking instrumentation for roundtrip metrics
#[derive(Debug)]
pub struct RoundtripBenchmark {
    start_time: Instant,
    detection_histogram: Arc<LatencyHistogram>,
    roundtrip_histogram: Arc<LatencyHistogram>,
    transaction_count: Arc<AtomicU64>,
    dropped_count: Arc<AtomicU64>,
    max_buffer_utilization: Arc<std::sync::Mutex<f64>>,
}

impl RoundtripBenchmark {
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            detection_histogram: Arc::new(LatencyHistogram::new(10000)), // Keep 10k samples
            roundtrip_histogram: Arc::new(LatencyHistogram::new(10000)),
            transaction_count: Arc::new(AtomicU64::new(0)),
            dropped_count: Arc::new(AtomicU64::new(0)),
            max_buffer_utilization: Arc::new(std::sync::Mutex::new(0.0)),
        }
    }

    pub fn record_detection(&self, latency_ms: f64) {
        self.detection_histogram.record(latency_ms);
        self.transaction_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_roundtrip(&self, t_broadcast: DateTime<Utc>, t_local_seen: DateTime<Utc>) {
        let roundtrip_ms = (t_local_seen - t_broadcast).num_milliseconds() as f64;
        if roundtrip_ms >= 0.0 && roundtrip_ms < 10000.0 {
            // Only record reasonable roundtrip times (< 10 seconds)
            self.roundtrip_histogram.record(roundtrip_ms);
        }
    }

    pub fn record_dropped(&self) {
        self.dropped_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn update_buffer_utilization(&self, utilization: f64) {
        if let Ok(mut max_util) = self.max_buffer_utilization.lock() {
            if utilization > *max_util {
                *max_util = utilization;
            }
        }
    }

    pub fn get_results(&self, buffer_capacity: usize, peak_length: usize) -> LatencyBenchmarkResult {
        let elapsed = self.start_time.elapsed();
        let total_transactions = self.transaction_count.load(Ordering::Relaxed);
        let dropped_transactions = self.dropped_count.load(Ordering::Relaxed);
        let max_utilization = self.max_buffer_utilization.lock().unwrap_or(&0.0).clone();

        let detection_stats = self.detection_histogram.get_stats();
        let roundtrip_stats = self.roundtrip_histogram.get_stats();

        LatencyBenchmarkResult {
            timestamp: Utc::now(),
            duration_minutes: elapsed.as_secs_f64() / 60.0,
            total_transactions,
            transactions_per_second: total_transactions as f64 / elapsed.as_secs_f64(),
            detection_latency_ms: LatencyStats {
                median: detection_stats.median_ms,
                p95: detection_stats.p95_ms,
                p99: detection_stats.p99_ms,
                mean: detection_stats.mean_ms,
                min: 0.0, // Would need to track separately
                max: 0.0, // Would need to track separately
                count: detection_stats.count,
            },
            roundtrip_latency_ms: LatencyStats {
                median: roundtrip_stats.median_ms,
                p95: roundtrip_stats.p95_ms,
                p99: roundtrip_stats.p99_ms,
                mean: roundtrip_stats.mean_ms,
                min: 0.0,
                max: 0.0,
                count: roundtrip_stats.count,
            },
            buffer_stats: BufferStats {
                utilization_percent: 0.0, // Current utilization at end
                max_utilization_percent: max_utilization,
                capacity: buffer_capacity,
                peak_length,
            },
            dropped_transactions,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("info,mev_mempool=debug")
        .with_target(false)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    info!("Starting MEV Bot Latency Benchmark");

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let duration_minutes = if args.len() > 1 {
        args[1].parse::<f64>().unwrap_or(30.0)
    } else {
        30.0
    };

    let endpoint = std::env::var("HYPEREVM_WS_URL")
        .unwrap_or_else(|_| "wss://api.hyperliquid.xyz/ws".to_string());

    info!(
        "Running benchmark for {:.1} minutes on endpoint: {}",
        duration_minutes, endpoint
    );

    // Create benchmark tracker
    let benchmark = Arc::new(RoundtripBenchmark::new());

    // Create mempool service
    let mempool_service = MempoolService::new(endpoint)?;
    let metrics = mempool_service.get_metrics();

    // Start metrics server in background
    let metrics_server = mev_core::MetricsServer::new(metrics.clone(), 9090);
    let metrics_handle = tokio::spawn(async move {
        if let Err(e) = metrics_server.start().await {
            error!("Metrics server error: {}", e);
        }
    });

    // Start mempool monitoring with benchmark instrumentation
    let benchmark_clone = benchmark.clone();
    let mempool_handle = tokio::spawn(async move {
        if let Err(e) = run_benchmark_mempool(mempool_service, benchmark_clone).await {
            error!("Mempool benchmark error: {}", e);
        }
    });

    // Start periodic stats reporting
    let benchmark_clone = benchmark.clone();
    let stats_handle = tokio::spawn(async move {
        let mut stats_interval = interval(Duration::from_secs(30));
        loop {
            stats_interval.tick().await;
            print_benchmark_stats(&benchmark_clone).await;
        }
    });

    // Wait for specified duration or Ctrl+C
    let duration = Duration::from_secs((duration_minutes * 60.0) as u64);
    tokio::select! {
        _ = sleep(duration) => {
            info!("Benchmark duration completed");
        }
        _ = signal::ctrl_c() => {
            info!("Received Ctrl+C, stopping benchmark");
        }
    }

    // Stop background tasks
    mempool_handle.abort();
    stats_handle.abort();
    metrics_handle.abort();

    // Generate final report
    let results = benchmark.get_results(10000, 0); // TODO: Get actual buffer stats
    save_benchmark_results(&results).await?;
    print_final_results(&results);

    Ok(())
}

async fn run_benchmark_mempool(
    mempool_service: MempoolService,
    benchmark: Arc<RoundtripBenchmark>,
) -> Result<()> {
    let ws_client = WebSocketRpcClient::new(mempool_service.endpoint.clone());

    loop {
        match run_benchmark_session(&ws_client, &benchmark).await {
            Ok(_) => {
                warn!("Benchmark session ended, restarting...");
            }
            Err(e) => {
                error!("Benchmark session error: {}, restarting in 5 seconds...", e);
                sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn run_benchmark_session(
    ws_client: &WebSocketRpcClient,
    benchmark: &Arc<RoundtripBenchmark>,
) -> Result<()> {
    use futures_util::{SinkExt, StreamExt};

    let mut ws_stream = ws_client.subscribe_pending_transactions().await?;
    let _subscription_id = ws_client.send_subscribe_request(&mut ws_stream).await?;

    while let Some(message) = ws_stream.next().await {
        match message {
            Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                let processing_timer = PerformanceTimer::new("benchmark_processing".to_string());

                if let Ok(Some(tx_data)) = ws_client.parse_transaction_notification(&text) {
                    let processing_time_ms = processing_timer.elapsed_ms();
                    benchmark.record_detection(processing_time_ms);

                    // Extract timestamp for roundtrip calculation
                    if let Some(timestamp_str) = tx_data.get("timestamp").and_then(|v| v.as_str()) {
                        if let Ok(t_broadcast) = DateTime::parse_from_rfc3339(timestamp_str) {
                            let t_local_seen = Utc::now();
                            benchmark.record_roundtrip(t_broadcast.into(), t_local_seen);
                        }
                    }
                }
            }
            Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                warn!("WebSocket connection closed");
                break;
            }
            Ok(tokio_tungstenite::tungstenite::Message::Ping(data)) => {
                if let Err(e) = ws_stream
                    .send(tokio_tungstenite::tungstenite::Message::Pong(data))
                    .await
                {
                    error!("Failed to send pong: {}", e);
                }
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

async fn print_benchmark_stats(benchmark: &Arc<RoundtripBenchmark>) {
    let elapsed = benchmark.start_time.elapsed();
    let total_transactions = benchmark.transaction_count.load(Ordering::Relaxed);
    let dropped_transactions = benchmark.dropped_count.load(Ordering::Relaxed);
    let tps = total_transactions as f64 / elapsed.as_secs_f64();

    let detection_stats = benchmark.detection_histogram.get_stats();
    let roundtrip_stats = benchmark.roundtrip_histogram.get_stats();

    info!(
        "Benchmark Stats - Elapsed: {:.1}m, TPS: {:.2}, Total: {}, Dropped: {}",
        elapsed.as_secs_f64() / 60.0,
        tps,
        total_transactions,
        dropped_transactions
    );

    info!(
        "Detection Latency - Median: {:.2}ms, P95: {:.2}ms, P99: {:.2}ms, Mean: {:.2}ms",
        detection_stats.median_ms,
        detection_stats.p95_ms,
        detection_stats.p99_ms,
        detection_stats.mean_ms
    );

    if roundtrip_stats.count > 0 {
        info!(
            "Roundtrip Latency - Median: {:.2}ms, P95: {:.2}ms, P99: {:.2}ms, Mean: {:.2}ms",
            roundtrip_stats.median_ms,
            roundtrip_stats.p95_ms,
            roundtrip_stats.p99_ms,
            roundtrip_stats.mean_ms
        );
    }
}

async fn save_benchmark_results(results: &LatencyBenchmarkResult) -> Result<()> {
    let filename = format!(
        "benchmark_results_{}.json",
        results.timestamp.format("%Y%m%d_%H%M%S")
    );

    let json_data = serde_json::to_string_pretty(results)?;
    let mut file = File::create(&filename).await?;
    file.write_all(json_data.as_bytes()).await?;

    info!("Benchmark results saved to: {}", filename);
    Ok(())
}

fn print_final_results(results: &LatencyBenchmarkResult) {
    println!("\n=== MEV Bot Latency Benchmark Results ===");
    println!("Duration: {:.1} minutes", results.duration_minutes);
    println!("Total Transactions: {}", results.total_transactions);
    println!("Transactions/Second: {:.2}", results.transactions_per_second);
    println!("Dropped Transactions: {}", results.dropped_transactions);
    println!();

    println!("Detection Latency (ms):");
    println!("  Median: {:.2}", results.detection_latency_ms.median);
    println!("  P95: {:.2}", results.detection_latency_ms.p95);
    println!("  P99: {:.2}", results.detection_latency_ms.p99);
    println!("  Mean: {:.2}", results.detection_latency_ms.mean);
    println!();

    if results.roundtrip_latency_ms.count > 0 {
        println!("Roundtrip Latency (ms):");
        println!("  Median: {:.2}", results.roundtrip_latency_ms.median);
        println!("  P95: {:.2}", results.roundtrip_latency_ms.p95);
        println!("  P99: {:.2}", results.roundtrip_latency_ms.p99);
        println!("  Mean: {:.2}", results.roundtrip_latency_ms.mean);
        println!();
    }

    println!("Buffer Statistics:");
    println!("  Capacity: {}", results.buffer_stats.capacity);
    println!("  Peak Length: {}", results.buffer_stats.peak_length);
    println!(
        "  Max Utilization: {:.1}%",
        results.buffer_stats.max_utilization_percent
    );

    // Performance assessment
    println!("\n=== Performance Assessment ===");
    if results.detection_latency_ms.median <= 20.0 {
        println!("✅ Detection latency target met (≤20ms median)");
    } else {
        println!("❌ Detection latency target missed (≤20ms median)");
    }

    if results.detection_latency_ms.p95 <= 50.0 {
        println!("✅ Detection latency P95 target met (≤50ms)");
    } else {
        println!("❌ Detection latency P95 target missed (≤50ms)");
    }

    if results.transactions_per_second >= 100.0 {
        println!("✅ Throughput target met (≥100 TPS)");
    } else {
        println!("❌ Throughput target missed (≥100 TPS)");
    }

    let drop_rate = results.dropped_transactions as f64 / results.total_transactions as f64 * 100.0;
    if drop_rate <= 1.0 {
        println!("✅ Drop rate acceptable (≤1%)");
    } else {
        println!("❌ Drop rate too high (≤1%)");
    }
}