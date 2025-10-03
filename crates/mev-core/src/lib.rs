//! MEV Core - Core utilities and types for MEV bot

pub mod abi_decoder;
pub mod bundle;
pub mod cpu_pinning;
pub mod decision_path;
pub mod decision_path_manager;
pub mod grpc_service;
pub mod latency_monitor;
pub mod metrics;
pub mod monitoring;
pub mod persistence;
pub mod rpc_client;
pub mod simulation;
pub mod simulation_benchmark;
pub mod simulation_optimizer;
pub mod state_manager;
pub mod strategy_types;
pub mod types;
pub mod utils;

pub use abi_decoder::*;
pub use bundle::{
    BundleManager, Bundle, SignedTransaction, TransactionTemplate, BundleParams, 
    NonceStrategy, BundleValidation, SubmissionResult, ExecutionStatus,
    KeyManager, TransactionSigner, BundleBuilder, BundleSubmitter, SubmissionConfig,
    SubmissionPath, FastResubmitter, ExecutionTracker, TrackerConfig, InclusionMetrics,
    OrderingVerification, VictimGenerator, VictimGeneratorConfig, VictimTransaction,
    VictimDeployment, TestScenario, TestResults, ArbitrageBundleBuilder, BackrunBundleBuilder,
    SandwichBundleBuilder
};
pub use cpu_pinning::*;
pub use decision_path::{DecisionPath, DecisionPathConfig, DecisionResult, DecisionPathStats};
pub use decision_path_manager::*;
pub use grpc_service::*;
pub use latency_monitor::{LatencyMonitor, LatencyMonitorConfig, LatencyReport, LatencyCategory, LatencyMeasurement};
pub use metrics::{PrometheusMetrics, PerformanceTimer, LatencyHistogram, SystemMetrics, MempoolMetrics, SimulationMetrics, ExecutionMetrics};
pub use monitoring::{ProductionMetrics, MonitoringServer, HealthChecker, HealthStatus, CheckResult};
pub use persistence::*;
pub use rpc_client::*;
pub use simulation::*;
pub use simulation_benchmark::*;
pub use simulation_optimizer::*;
pub use state_manager::*;
pub use strategy_types::*;
pub use types::*;
pub use utils::*;

/// Core MEV bot functionality
pub struct MevCore {
    pub name: String,
}

impl MevCore {
    pub fn new(name: String) -> Self {
        Self { name }
    }

    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
}
