//! Simulation API server for testing and development

use anyhow::Result;
use mev_core::{
    ForkSimulator, SimulationConfig, SimulationService, PrometheusMetrics, TestDataGenerator
};
use std::sync::Arc;
use tracing::{info, error};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("Starting MEV Simulation API Server");

    // Create configuration
    let config = SimulationConfig {
        rpc_url: std::env::var("RPC_URL").unwrap_or_else(|_| "http://localhost:8545".to_string()),
        timeout_ms: 1000,
        max_concurrent_simulations: 100,
        enable_state_override: true,
        gas_limit_multiplier: 1.2,
        base_fee_multiplier: 1.1,
        http_api_port: 8080,
        enable_deterministic_testing: true,
    };

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

    // Create service
    let service = SimulationService::new(simulator);

    // Test with deterministic data if enabled
    if config.enable_deterministic_testing {
        info!("Running deterministic tests...");
        run_deterministic_tests(&service).await?;
    }

    // Start HTTP server
    info!("Starting HTTP API server on port {}", config.http_api_port);
    service.start_server(config.http_api_port).await?;

    Ok(())
}

async fn run_deterministic_tests(service: &SimulationService) -> Result<()> {
    let generator = TestDataGenerator::new(12345);
    let test_bundles = generator.generate_test_bundles();

    info!("Running {} deterministic test bundles", test_bundles.len());

    for bundle in test_bundles {
        info!("Testing bundle: {}", bundle.id);
        
        match service.get_simulator().simulate_bundle(bundle.clone()).await {
            Ok(result) => {
                info!(
                    "Bundle {} completed: success={}, gas_used={}, simulation_time={}ms",
                    result.bundle_id,
                    result.success,
                    result.gas_used,
                    result.simulation_time_ms
                );
                
                if !result.success {
                    info!("Bundle failed as expected: {:?}", result.revert_reason);
                }
            }
            Err(e) => {
                error!("Bundle {} failed with error: {}", bundle.id, e);
            }
        }
    }

    info!("Deterministic tests completed");
    Ok(())
}