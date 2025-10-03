//! Production monitoring and metrics system

pub mod metrics;
pub mod logging;
pub mod dashboards;
pub mod health;

pub use metrics::*;
pub use logging::*;
pub use dashboards::*;
pub use health::*;

use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use prometheus::TextEncoder;
use serde_json::json;
use std::sync::Arc;
use tokio::net::TcpListener;

/// Production monitoring server
pub struct MonitoringServer {
    metrics: Arc<ProductionMetrics>,
    health_checker: Arc<HealthChecker>,
    port: u16,
}

impl MonitoringServer {
    /// Create a new monitoring server
    pub fn new(metrics: Arc<ProductionMetrics>, health_checker: Arc<HealthChecker>, port: u16) -> Self {
        Self {
            metrics,
            health_checker,
            port,
        }
    }
    
    /// Start the monitoring server
    pub async fn start(&self) -> Result<()> {
        let app = Router::new()
            .route("/metrics", get(metrics_handler))
            .route("/health", get(health_handler))
            .route("/ready", get(readiness_handler))
            .route("/live", get(liveness_handler))
            .with_state(AppState {
                metrics: self.metrics.clone(),
                health_checker: self.health_checker.clone(),
            });
        
        let listener = TcpListener::bind(format!("0.0.0.0:{}", self.port)).await?;
        tracing::info!("Monitoring server listening on port {}", self.port);
        
        axum::serve(listener, app).await?;
        Ok(())
    }
}

#[derive(Clone)]
struct AppState {
    metrics: Arc<ProductionMetrics>,
    health_checker: Arc<HealthChecker>,
}

async fn metrics_handler(State(state): State<AppState>) -> Response {
    let encoder = TextEncoder::new();
    let metric_families = state.metrics.registry.gather();
    
    match encoder.encode_to_string(&metric_families) {
        Ok(output) => output.into_response(),
        Err(e) => {
            tracing::error!("Failed to encode metrics: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn health_handler(State(state): State<AppState>) -> Response {
    let health_status = state.health_checker.get_health_status().await;
    
    let status_code = if health_status.is_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    
    (status_code, Json(health_status)).into_response()
}

async fn readiness_handler(State(state): State<AppState>) -> Response {
    let is_ready = state.health_checker.is_ready().await;
    
    if is_ready {
        Json(json!({"status": "ready"})).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"status": "not ready"}))).into_response()
    }
}

async fn liveness_handler() -> Response {
    Json(json!({"status": "alive"})).into_response()
}