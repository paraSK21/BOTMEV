//! gRPC simulation server for MEV bot

use anyhow::Result;
use mev_core::{
    ForkSimulator, SimulationConfig, PrometheusMetrics, start_grpc_server
};
use std::sync::Arc;
use tracing::{info, error};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("Starting MEV Simulation gRPC Server");

    // Create configuration
    let config = SimulationConfig {
        rpc_url: std::env::var("RPC_URL").unwrap_or_else(|_| "http://localhost:8545".to_string()),
        timeout_ms: 1000,
        max_concurrent_simulations: 100,
        enable_state_override: true,
        gas_limit_multiplier: 1.2,
        base_fee_multiplier: 1.1,
        http_api_port: 8080, // Not used for gRPC
        enable_deterministic_testing: true,
    };

    let grpc_port = std::env::var("GRPC_PORT")
        .unwrap_or_else(|_| "50051".to_string())
        .parse::<u16>()
        .unwrap_or(50051);

    // Create metrics
    let metrics = Arc::new(PrometheusMetrics::new()?);

    // Create simulator
    let simulator = match ForkSimulator::new(config.clone(), metrics).await {
        Ok(sim) => Arc::new(sim),
        Err(e) => {
            error!("Failed to create simulator: {}", e);
            error!("Make sure you have a valid RPC endpoint configured");
            return Err(e);
        }
    };

    // Start gRPC server
    info!("Starting gRPC server on port {}", grpc_port);
    start_grpc_server(simulator, grpc_port).await?;

    Ok(())
}