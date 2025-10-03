use anyhow::Result;
use mev_core::metrics::MetricsServer;
use mev_mempool::MempoolService;
use std::env;
use tokio::signal;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize structured logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    info!("Starting MEV Mempool Service with Metrics");

    // Get WebSocket endpoint from environment or use default
    let ws_endpoint = env::var("WS_ENDPOINT")
        .unwrap_or_else(|_| "ws://localhost:8546".to_string());

    // Get metrics port from environment or use default
    let metrics_port: u16 = env::var("METRICS_PORT")
        .unwrap_or_else(|_| "9090".to_string())
        .parse()
        .unwrap_or(9090);

    info!("Connecting to WebSocket endpoint: {}", ws_endpoint);
    info!("Starting metrics server on port: {}", metrics_port);

    // Create mempool service
    let mempool_service = MempoolService::new(ws_endpoint)?;
    let metrics = mempool_service.get_metrics();
    
    // Start metrics server
    let metrics_server = MetricsServer::new(metrics, metrics_port);
    
    // Run both services concurrently with graceful shutdown
    tokio::select! {
        result = mempool_service.start() => {
            if let Err(e) = result {
                error!("Mempool service error: {}", e);
            }
        }
        result = metrics_server.start() => {
            if let Err(e) = result {
                error!("Metrics server error: {}", e);
            }
        }
        _ = signal::ctrl_c() => {
            info!("Received shutdown signal, stopping services");
        }
    }

    info!("MEV Mempool Service stopped");
    Ok(())
}
