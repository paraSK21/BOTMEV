//! Standalone metrics server for MEV bot Prometheus metrics
//! 
//! This server exposes metrics on /metrics endpoint and provides health checks.

use anyhow::Result;
use mev_core::{metrics::MetricsServer, PrometheusMetrics};
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .with_thread_ids(true)
        .init();

    info!("Starting MEV Bot Metrics Server");

    // Parse port from environment or use default
    let port = std::env::var("METRICS_PORT")
        .unwrap_or_else(|_| "9090".to_string())
        .parse::<u16>()
        .unwrap_or(9090);

    // Create Prometheus metrics
    let metrics = Arc::new(PrometheusMetrics::new()?);
    
    // Create and start metrics server
    let metrics_server = MetricsServer::new(metrics.clone(), port);
    
    info!("Metrics server will be available at http://0.0.0.0:{}/metrics", port);
    info!("Health check available at http://0.0.0.0:{}/health", port);
    
    // Start server with graceful shutdown
    tokio::select! {
        result = metrics_server.start() => {
            if let Err(e) = result {
                error!("Metrics server error: {}", e);
            }
        }
        _ = signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down metrics server");
        }
    }

    info!("Metrics server stopped");
    Ok(())
}