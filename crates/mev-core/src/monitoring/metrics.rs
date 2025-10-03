//! Comprehensive Prometheus metrics for production monitoring

use prometheus::{
    CounterVec, Gauge, GaugeVec, HistogramOpts, HistogramVec, IntCounter,
    IntCounterVec, Opts, Registry,
};
use std::sync::Arc;

/// Production metrics collector
pub struct ProductionMetrics {
    pub registry: Registry,
    
    // Detection and Decision Latency
    pub detection_latency: HistogramVec,
    pub decision_latency: HistogramVec,
    pub simulation_latency: HistogramVec,
    
    // Bundle Metrics
    pub bundles_created: IntCounterVec,
    pub bundles_submitted: IntCounterVec,
    pub bundles_included: IntCounterVec,
    pub bundles_failed: IntCounterVec,
    pub bundle_success_rate: GaugeVec,
    
    // Simulation Metrics
    pub simulations_per_second: Gauge,
    pub simulation_success_rate: Gauge,
    pub simulation_gas_used: HistogramVec,
    
    // PnL and Financial Metrics
    pub simulated_pnl: GaugeVec,
    pub realized_pnl: GaugeVec,
    pub gas_costs: CounterVec,
    pub profit_per_bundle: HistogramVec,
    
    // System Performance
    pub mempool_transactions_per_second: Gauge,
    pub opportunities_detected: IntCounterVec,
    pub cpu_usage: Gauge,
    pub memory_usage: Gauge,
    pub network_latency: HistogramVec,
    
    // Error and Failure Tracking
    pub errors_total: IntCounterVec,
    pub failures_by_type: IntCounterVec,
    pub recovery_attempts: IntCounterVec,
    pub kill_switch_activations: IntCounter,
}

impl ProductionMetrics {
    /// Create a new production metrics instance
    pub fn new() -> Arc<Self> {
        let registry = Registry::new();
        
        // Detection and Decision Latency
        let detection_latency = HistogramVec::new(
            HistogramOpts::new(
                "mev_detection_latency_seconds",
                "Time from mempool ingestion to opportunity detection"
            ).buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0]),
            &["strategy", "target_type"]
        ).unwrap();
        
        let decision_latency = HistogramVec::new(
            HistogramOpts::new(
                "mev_decision_latency_seconds",
                "Time from detection to bundle creation decision"
            ).buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5]),
            &["strategy", "decision"]
        ).unwrap();
        
        let simulation_latency = HistogramVec::new(
            HistogramOpts::new(
                "mev_simulation_latency_seconds",
                "Time to complete bundle simulation"
            ).buckets(vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0]),
            &["strategy", "simulation_type"]
        ).unwrap();
        
        // Bundle Metrics
        let bundles_created = IntCounterVec::new(
            Opts::new("mev_bundles_created_total", "Total bundles created"),
            &["strategy"]
        ).unwrap();
        
        let bundles_submitted = IntCounterVec::new(
            Opts::new("mev_bundles_submitted_total", "Total bundles submitted"),
            &["strategy", "submission_path"]
        ).unwrap();
        
        let bundles_included = IntCounterVec::new(
            Opts::new("mev_bundles_included_total", "Total bundles included in blocks"),
            &["strategy", "block_position"]
        ).unwrap();
        
        let bundles_failed = IntCounterVec::new(
            Opts::new("mev_bundles_failed_total", "Total bundles that failed"),
            &["strategy", "failure_reason"]
        ).unwrap();
        
        let bundle_success_rate = GaugeVec::new(
            Opts::new("mev_bundle_success_rate", "Bundle inclusion success rate"),
            &["strategy", "time_window"]
        ).unwrap();
        
        // Simulation Metrics
        let simulations_per_second = Gauge::new(
            "mev_simulations_per_second",
            "Current simulation throughput"
        ).unwrap();
        
        let simulation_success_rate = Gauge::new(
            "mev_simulation_success_rate",
            "Percentage of successful simulations"
        ).unwrap();
        
        let simulation_gas_used = HistogramVec::new(
            HistogramOpts::new(
                "mev_simulation_gas_used",
                "Gas used in bundle simulations"
            ).buckets(vec![50000.0, 100000.0, 200000.0, 500000.0, 1000000.0, 2000000.0]),
            &["strategy"]
        ).unwrap();
        
        // PnL and Financial Metrics
        let simulated_pnl = GaugeVec::new(
            Opts::new("mev_simulated_pnl_wei", "Simulated profit/loss in wei"),
            &["strategy", "token"]
        ).unwrap();
        
        let realized_pnl = GaugeVec::new(
            Opts::new("mev_realized_pnl_wei", "Realized profit/loss in wei"),
            &["strategy", "token"]
        ).unwrap();
        
        let gas_costs = CounterVec::new(
            Opts::new("mev_gas_costs_wei_total", "Total gas costs in wei"),
            &["strategy", "transaction_type"]
        ).unwrap();
        
        let profit_per_bundle = HistogramVec::new(
            HistogramOpts::new(
                "mev_profit_per_bundle_wei",
                "Profit per bundle in wei"
            ).buckets(vec![
                1000000000000000.0,    // 0.001 ETH
                10000000000000000.0,   // 0.01 ETH
                50000000000000000.0,   // 0.05 ETH
                100000000000000000.0,  // 0.1 ETH
                500000000000000000.0,  // 0.5 ETH
                1000000000000000000.0, // 1 ETH
            ]),
            &["strategy"]
        ).unwrap();
        
        // System Performance
        let mempool_transactions_per_second = Gauge::new(
            "mev_mempool_transactions_per_second",
            "Mempool transaction ingestion rate"
        ).unwrap();
        
        let opportunities_detected = IntCounterVec::new(
            Opts::new("mev_opportunities_detected_total", "Total MEV opportunities detected"),
            &["strategy", "confidence_level"]
        ).unwrap();
        
        let cpu_usage = Gauge::new(
            "mev_cpu_usage_percent",
            "CPU usage percentage"
        ).unwrap();
        
        let memory_usage = Gauge::new(
            "mev_memory_usage_bytes",
            "Memory usage in bytes"
        ).unwrap();
        
        let network_latency = HistogramVec::new(
            HistogramOpts::new(
                "mev_network_latency_seconds",
                "Network latency to various endpoints"
            ).buckets(vec![0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0]),
            &["endpoint", "operation"]
        ).unwrap();
        
        // Error and Failure Tracking
        let errors_total = IntCounterVec::new(
            Opts::new("mev_errors_total", "Total errors by type"),
            &["error_type", "component"]
        ).unwrap();
        
        let failures_by_type = IntCounterVec::new(
            Opts::new("mev_failures_by_type_total", "Failures categorized by type"),
            &["failure_type", "recovery_action"]
        ).unwrap();
        
        let recovery_attempts = IntCounterVec::new(
            Opts::new("mev_recovery_attempts_total", "Recovery attempts by type"),
            &["recovery_type", "success"]
        ).unwrap();
        
        let kill_switch_activations = IntCounter::new(
            "mev_kill_switch_activations_total",
            "Total kill switch activations"
        ).unwrap();
        
        // Register all metrics
        registry.register(Box::new(detection_latency.clone())).unwrap();
        registry.register(Box::new(decision_latency.clone())).unwrap();
        registry.register(Box::new(simulation_latency.clone())).unwrap();
        registry.register(Box::new(bundles_created.clone())).unwrap();
        registry.register(Box::new(bundles_submitted.clone())).unwrap();
        registry.register(Box::new(bundles_included.clone())).unwrap();
        registry.register(Box::new(bundles_failed.clone())).unwrap();
        registry.register(Box::new(bundle_success_rate.clone())).unwrap();
        registry.register(Box::new(simulations_per_second.clone())).unwrap();
        registry.register(Box::new(simulation_success_rate.clone())).unwrap();
        registry.register(Box::new(simulation_gas_used.clone())).unwrap();
        registry.register(Box::new(simulated_pnl.clone())).unwrap();
        registry.register(Box::new(realized_pnl.clone())).unwrap();
        registry.register(Box::new(gas_costs.clone())).unwrap();
        registry.register(Box::new(profit_per_bundle.clone())).unwrap();
        registry.register(Box::new(mempool_transactions_per_second.clone())).unwrap();
        registry.register(Box::new(opportunities_detected.clone())).unwrap();
        registry.register(Box::new(cpu_usage.clone())).unwrap();
        registry.register(Box::new(memory_usage.clone())).unwrap();
        registry.register(Box::new(network_latency.clone())).unwrap();
        registry.register(Box::new(errors_total.clone())).unwrap();
        registry.register(Box::new(failures_by_type.clone())).unwrap();
        registry.register(Box::new(recovery_attempts.clone())).unwrap();
        registry.register(Box::new(kill_switch_activations.clone())).unwrap();
        
        Arc::new(Self {
            registry,
            detection_latency,
            decision_latency,
            simulation_latency,
            bundles_created,
            bundles_submitted,
            bundles_included,
            bundles_failed,
            bundle_success_rate,
            simulations_per_second,
            simulation_success_rate,
            simulation_gas_used,
            simulated_pnl,
            realized_pnl,
            gas_costs,
            profit_per_bundle,
            mempool_transactions_per_second,
            opportunities_detected,
            cpu_usage,
            memory_usage,
            network_latency,
            errors_total,
            failures_by_type,
            recovery_attempts,
            kill_switch_activations,
        })
    }
    
    /// Record detection latency
    pub fn record_detection_latency(&self, strategy: &str, target_type: &str, duration_secs: f64) {
        self.detection_latency
            .with_label_values(&[strategy, target_type])
            .observe(duration_secs);
    }
    
    /// Record bundle creation
    pub fn record_bundle_created(&self, strategy: &str) {
        self.bundles_created
            .with_label_values(&[strategy])
            .inc();
    }
    
    /// Record bundle submission
    pub fn record_bundle_submitted(&self, strategy: &str, submission_path: &str) {
        self.bundles_submitted
            .with_label_values(&[strategy, submission_path])
            .inc();
    }
    
    /// Record bundle inclusion
    pub fn record_bundle_included(&self, strategy: &str, block_position: &str) {
        self.bundles_included
            .with_label_values(&[strategy, block_position])
            .inc();
    }
    
    /// Record bundle failure
    pub fn record_bundle_failure(&self, strategy: &str, failure_reason: &str) {
        self.bundles_failed
            .with_label_values(&[strategy, failure_reason])
            .inc();
    }
    
    /// Update system performance metrics
    pub fn update_system_metrics(&self, cpu_percent: f64, memory_bytes: f64, mempool_tps: f64) {
        self.cpu_usage.set(cpu_percent);
        self.memory_usage.set(memory_bytes);
        self.mempool_transactions_per_second.set(mempool_tps);
    }
    
    /// Record opportunity detection
    pub fn record_opportunity_detected(&self, strategy: &str, confidence_level: &str) {
        self.opportunities_detected
            .with_label_values(&[strategy, confidence_level])
            .inc();
    }
    
    /// Record profit/loss
    pub fn record_pnl(&self, strategy: &str, token: &str, simulated_wei: f64, realized_wei: f64) {
        self.simulated_pnl
            .with_label_values(&[strategy, token])
            .set(simulated_wei);
        
        self.realized_pnl
            .with_label_values(&[strategy, token])
            .set(realized_wei);
    }
    
    /// Record error
    pub fn record_error(&self, error_type: &str, component: &str) {
        self.errors_total
            .with_label_values(&[error_type, component])
            .inc();
    }
}

impl Default for ProductionMetrics {
    fn default() -> Self {
        Arc::try_unwrap(Self::new()).unwrap_or_else(|_| panic!("Failed to unwrap Arc"))
    }
}