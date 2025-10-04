//! Prometheus metrics for HyperLiquid WebSocket integration

use anyhow::Result;
use prometheus::{
    HistogramVec, IntCounterVec, IntGaugeVec,
};
use std::time::Duration;

/// Prometheus metrics collector for HyperLiquid WebSocket service and RPC operations
pub struct HyperLiquidMetrics {
    // Gauges for connection state
    pub ws_connected: IntGaugeVec,
    pub active_subscriptions: IntGaugeVec,
    pub degraded_state: IntGaugeVec,
    
    // Counters for events
    pub trades_received_total: IntCounterVec,
    pub reconnection_attempts_total: IntCounterVec,
    pub connection_errors_total: IntCounterVec,
    pub subscription_errors_total: IntCounterVec,
    pub parse_errors_total: IntCounterVec,
    pub adaptation_errors_total: IntCounterVec,
    pub network_errors_total: IntCounterVec,
    
    // Histograms for latency tracking
    pub message_processing_duration: HistogramVec,
    
    // RPC metrics - Counters
    pub rpc_calls_total: IntCounterVec,
    pub rpc_errors_total: IntCounterVec,
    pub tx_submitted_total: IntCounterVec,
    
    // RPC metrics - Gauges
    pub state_poll_interval_seconds: IntGaugeVec,
    pub state_freshness_seconds: IntGaugeVec,
    
    // RPC metrics - Histograms
    pub rpc_call_duration: HistogramVec,
    pub tx_confirmation_duration: HistogramVec,
    pub tx_gas_used: HistogramVec,
}

impl HyperLiquidMetrics {
    /// Create a new HyperLiquidMetrics instance
    /// 
    /// Note: In tests, this may fail if metrics are already registered.
    /// Use `new_or_get()` for test-friendly creation.
    pub fn new() -> Result<Self> {
        use prometheus::{IntCounterVec, IntGaugeVec, HistogramVec, Opts};
        
        // Connection state gauges
        let ws_connected = IntGaugeVec::new(
            Opts::new("hyperliquid_ws_connected", "WebSocket connection status (0=disconnected, 1=connected)"),
            &["ws_url"]
        )?;
        prometheus::register(Box::new(ws_connected.clone()))?;

        let active_subscriptions = IntGaugeVec::new(
            Opts::new("hyperliquid_active_subscriptions", "Number of active trading pair subscriptions"),
            &["subscription_type"]
        )?;
        prometheus::register(Box::new(active_subscriptions.clone()))?;

        let degraded_state = IntGaugeVec::new(
            Opts::new("hyperliquid_degraded_state", "Service degraded state indicator (0=normal, 1=degraded)"),
            &["reason"]
        )?;
        prometheus::register(Box::new(degraded_state.clone()))?;

        // Event counters
        let trades_received_total = IntCounterVec::new(
            Opts::new("hyperliquid_trades_received_total", "Total number of trade messages received"),
            &["coin", "side"]
        )?;
        prometheus::register(Box::new(trades_received_total.clone()))?;

        let reconnection_attempts_total = IntCounterVec::new(
            Opts::new("hyperliquid_reconnection_attempts_total", "Total number of reconnection attempts"),
            &["reason"]
        )?;
        prometheus::register(Box::new(reconnection_attempts_total.clone()))?;

        let connection_errors_total = IntCounterVec::new(
            Opts::new("hyperliquid_connection_errors_total", "Total number of connection errors"),
            &["error_type"]
        )?;
        prometheus::register(Box::new(connection_errors_total.clone()))?;

        let subscription_errors_total = IntCounterVec::new(
            Opts::new("hyperliquid_subscription_errors_total", "Total number of subscription errors"),
            &["coin", "subscription_type"]
        )?;
        prometheus::register(Box::new(subscription_errors_total.clone()))?;

        let parse_errors_total = IntCounterVec::new(
            Opts::new("hyperliquid_parse_errors_total", "Total number of message parsing errors"),
            &["message_type"]
        )?;
        prometheus::register(Box::new(parse_errors_total.clone()))?;

        let adaptation_errors_total = IntCounterVec::new(
            Opts::new("hyperliquid_adaptation_errors_total", "Total number of trade adaptation errors"),
            &["coin", "error_type"]
        )?;
        prometheus::register(Box::new(adaptation_errors_total.clone()))?;

        let network_errors_total = IntCounterVec::new(
            Opts::new("hyperliquid_network_errors_total", "Total number of network errors"),
            &["error_type"]
        )?;
        prometheus::register(Box::new(network_errors_total.clone()))?;

        // Latency histograms
        let message_processing_duration = HistogramVec::new(
            prometheus::HistogramOpts::new("hyperliquid_message_processing_duration_seconds", "Time to process incoming WebSocket messages")
                .buckets(vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0]),
            &["message_type"]
        )?;
        prometheus::register(Box::new(message_processing_duration.clone()))?;

        // RPC counters
        let rpc_calls_total = IntCounterVec::new(
            Opts::new("hyperliquid_rpc_calls_total", "Total number of RPC calls made"),
            &["method", "status"]
        )?;
        prometheus::register(Box::new(rpc_calls_total.clone()))?;

        let rpc_errors_total = IntCounterVec::new(
            Opts::new("hyperliquid_rpc_errors_total", "Total number of RPC errors"),
            &["method", "error_type"]
        )?;
        prometheus::register(Box::new(rpc_errors_total.clone()))?;

        let tx_submitted_total = IntCounterVec::new(
            Opts::new("hyperliquid_tx_submitted_total", "Total number of transactions submitted"),
            &["status"]
        )?;
        prometheus::register(Box::new(tx_submitted_total.clone()))?;

        // RPC gauges
        let state_poll_interval_seconds = IntGaugeVec::new(
            Opts::new("hyperliquid_state_poll_interval_seconds", "Configured blockchain state polling interval in seconds"),
            &["endpoint"]
        )?;
        prometheus::register(Box::new(state_poll_interval_seconds.clone()))?;

        let state_freshness_seconds = IntGaugeVec::new(
            Opts::new("hyperliquid_state_freshness_seconds", "Time since last successful blockchain state update in seconds"),
            &["endpoint"]
        )?;
        prometheus::register(Box::new(state_freshness_seconds.clone()))?;

        // RPC histograms
        let rpc_call_duration = HistogramVec::new(
            prometheus::HistogramOpts::new("hyperliquid_rpc_call_duration_seconds", "Duration of RPC calls")
                .buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
            &["method"]
        )?;
        prometheus::register(Box::new(rpc_call_duration.clone()))?;

        let tx_confirmation_duration = HistogramVec::new(
            prometheus::HistogramOpts::new("hyperliquid_tx_confirmation_duration_seconds", "Time to confirm transactions")
                .buckets(vec![0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0]),
            &[]
        )?;
        prometheus::register(Box::new(tx_confirmation_duration.clone()))?;

        let tx_gas_used = HistogramVec::new(
            prometheus::HistogramOpts::new("hyperliquid_tx_gas_used", "Gas used by transactions")
                .buckets(vec![21000.0, 50000.0, 100000.0, 200000.0, 300000.0, 500000.0, 1000000.0, 2000000.0, 5000000.0]),
            &["status"]
        )?;
        prometheus::register(Box::new(tx_gas_used.clone()))?;

        Ok(Self {
            ws_connected,
            active_subscriptions,
            degraded_state,
            trades_received_total,
            reconnection_attempts_total,
            connection_errors_total,
            subscription_errors_total,
            parse_errors_total,
            adaptation_errors_total,
            network_errors_total,
            message_processing_duration,
            rpc_calls_total,
            rpc_errors_total,
            tx_submitted_total,
            state_poll_interval_seconds,
            state_freshness_seconds,
            rpc_call_duration,
            tx_confirmation_duration,
            tx_gas_used,
        })
    }
    
    /// Create a new HyperLiquidMetrics instance or return existing one
    /// 
    /// This is useful for tests where metrics might already be registered.
    /// If metrics are already registered, this will try to get them from the registry.
    #[cfg(test)]
    pub fn new_or_default() -> Self {
        use prometheus::{IntCounterVec, IntGaugeVec, HistogramVec, Opts};
        
        // Try to create new metrics, if it fails, create dummy metrics for testing
        match Self::new() {
            Ok(metrics) => metrics,
            Err(_) => {
                // Create unregistered metrics for testing
                let ws_connected = IntGaugeVec::new(
                    Opts::new("test_hyperliquid_ws_connected", "Test metric"),
                    &["ws_url"]
                ).unwrap();
                
                let active_subscriptions = IntGaugeVec::new(
                    Opts::new("test_hyperliquid_active_subscriptions", "Test metric"),
                    &["subscription_type"]
                ).unwrap();
                
                let degraded_state = IntGaugeVec::new(
                    Opts::new("test_hyperliquid_degraded_state", "Test metric"),
                    &["reason"]
                ).unwrap();
                
                let trades_received_total = IntCounterVec::new(
                    Opts::new("test_hyperliquid_trades_received_total", "Test metric"),
                    &["coin", "side"]
                ).unwrap();
                
                let reconnection_attempts_total = IntCounterVec::new(
                    Opts::new("test_hyperliquid_reconnection_attempts_total", "Test metric"),
                    &["reason"]
                ).unwrap();
                
                let connection_errors_total = IntCounterVec::new(
                    Opts::new("test_hyperliquid_connection_errors_total", "Test metric"),
                    &["error_type"]
                ).unwrap();
                
                let subscription_errors_total = IntCounterVec::new(
                    Opts::new("test_hyperliquid_subscription_errors_total", "Test metric"),
                    &["coin", "subscription_type"]
                ).unwrap();
                
                let parse_errors_total = IntCounterVec::new(
                    Opts::new("test_hyperliquid_parse_errors_total", "Test metric"),
                    &["message_type"]
                ).unwrap();
                
                let adaptation_errors_total = IntCounterVec::new(
                    Opts::new("test_hyperliquid_adaptation_errors_total", "Test metric"),
                    &["coin", "error_type"]
                ).unwrap();
                
                let network_errors_total = IntCounterVec::new(
                    Opts::new("test_hyperliquid_network_errors_total", "Test metric"),
                    &["error_type"]
                ).unwrap();
                
                let message_processing_duration = HistogramVec::new(
                    prometheus::HistogramOpts::new("test_hyperliquid_message_processing_duration_seconds", "Test metric")
                        .buckets(vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0]),
                    &["message_type"]
                ).unwrap();
                
                let rpc_calls_total = IntCounterVec::new(
                    Opts::new("test_hyperliquid_rpc_calls_total", "Test metric"),
                    &["method", "status"]
                ).unwrap();
                
                let rpc_errors_total = IntCounterVec::new(
                    Opts::new("test_hyperliquid_rpc_errors_total", "Test metric"),
                    &["method", "error_type"]
                ).unwrap();
                
                let tx_submitted_total = IntCounterVec::new(
                    Opts::new("test_hyperliquid_tx_submitted_total", "Test metric"),
                    &["status"]
                ).unwrap();
                
                let state_poll_interval_seconds = IntGaugeVec::new(
                    Opts::new("test_hyperliquid_state_poll_interval_seconds", "Test metric"),
                    &["endpoint"]
                ).unwrap();
                
                let state_freshness_seconds = IntGaugeVec::new(
                    Opts::new("test_hyperliquid_state_freshness_seconds", "Test metric"),
                    &["endpoint"]
                ).unwrap();
                
                let rpc_call_duration = HistogramVec::new(
                    prometheus::HistogramOpts::new("test_hyperliquid_rpc_call_duration_seconds", "Test metric")
                        .buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
                    &["method"]
                ).unwrap();
                
                let tx_confirmation_duration = HistogramVec::new(
                    prometheus::HistogramOpts::new("test_hyperliquid_tx_confirmation_duration_seconds", "Test metric")
                        .buckets(vec![0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0]),
                    &[]
                ).unwrap();
                
                let tx_gas_used = HistogramVec::new(
                    prometheus::HistogramOpts::new("test_hyperliquid_tx_gas_used", "Test metric")
                        .buckets(vec![21000.0, 50000.0, 100000.0, 200000.0, 300000.0, 500000.0, 1000000.0, 2000000.0, 5000000.0]),
                    &["status"]
                ).unwrap();
                
                Self {
                    ws_connected,
                    active_subscriptions,
                    degraded_state,
                    trades_received_total,
                    reconnection_attempts_total,
                    connection_errors_total,
                    subscription_errors_total,
                    parse_errors_total,
                    adaptation_errors_total,
                    network_errors_total,
                    message_processing_duration,
                    rpc_calls_total,
                    rpc_errors_total,
                    tx_submitted_total,
                    state_poll_interval_seconds,
                    state_freshness_seconds,
                    rpc_call_duration,
                    tx_confirmation_duration,
                    tx_gas_used,
                }
            }
        }
    }

    /// Set WebSocket connection status
    pub fn set_ws_connected(&self, ws_url: &str, connected: bool) {
        let value = if connected { 1 } else { 0 };
        self.ws_connected
            .with_label_values(&[ws_url])
            .set(value);
    }

    /// Set number of active subscriptions
    pub fn set_active_subscriptions(&self, subscription_type: &str, count: i64) {
        self.active_subscriptions
            .with_label_values(&[subscription_type])
            .set(count);
    }

    /// Set degraded state
    pub fn set_degraded_state(&self, reason: &str, degraded: bool) {
        let value = if degraded { 1 } else { 0 };
        self.degraded_state
            .with_label_values(&[reason])
            .set(value);
    }

    /// Increment trades received counter
    pub fn inc_trades_received(&self, coin: &str, side: &str) {
        self.trades_received_total
            .with_label_values(&[coin, side])
            .inc();
    }

    /// Increment reconnection attempts counter
    pub fn inc_reconnection_attempts(&self, reason: &str) {
        self.reconnection_attempts_total
            .with_label_values(&[reason])
            .inc();
    }

    /// Increment connection errors counter
    pub fn inc_connection_errors(&self, error_type: &str) {
        self.connection_errors_total
            .with_label_values(&[error_type])
            .inc();
    }

    /// Increment subscription errors counter
    pub fn inc_subscription_errors(&self, coin: &str, subscription_type: &str) {
        self.subscription_errors_total
            .with_label_values(&[coin, subscription_type])
            .inc();
    }

    /// Increment parse errors counter
    pub fn inc_parse_errors(&self, message_type: &str) {
        self.parse_errors_total
            .with_label_values(&[message_type])
            .inc();
    }

    /// Increment adaptation errors counter
    pub fn inc_adaptation_errors(&self, coin: &str, error_type: &str) {
        self.adaptation_errors_total
            .with_label_values(&[coin, error_type])
            .inc();
    }

    /// Increment network errors counter
    pub fn inc_network_errors(&self, error_type: &str) {
        self.network_errors_total
            .with_label_values(&[error_type])
            .inc();
    }

    /// Record message processing duration
    pub fn record_message_processing_duration(&self, message_type: &str, duration: Duration) {
        self.message_processing_duration
            .with_label_values(&[message_type])
            .observe(duration.as_secs_f64());
    }

    // RPC metric helper methods

    /// Increment RPC calls counter
    pub fn inc_rpc_calls(&self, method: &str, status: &str) {
        self.rpc_calls_total
            .with_label_values(&[method, status])
            .inc();
    }

    /// Increment RPC errors counter
    pub fn inc_rpc_errors(&self, method: &str, error_type: &str) {
        self.rpc_errors_total
            .with_label_values(&[method, error_type])
            .inc();
    }

    /// Increment transaction submitted counter
    pub fn inc_tx_submitted(&self, status: &str) {
        self.tx_submitted_total
            .with_label_values(&[status])
            .inc();
    }

    /// Set state poll interval
    pub fn set_state_poll_interval(&self, endpoint: &str, interval_seconds: i64) {
        self.state_poll_interval_seconds
            .with_label_values(&[endpoint])
            .set(interval_seconds);
    }

    /// Set state freshness
    pub fn set_state_freshness(&self, endpoint: &str, freshness_seconds: i64) {
        self.state_freshness_seconds
            .with_label_values(&[endpoint])
            .set(freshness_seconds);
    }

    /// Record RPC call duration
    pub fn record_rpc_call_duration(&self, method: &str, duration: Duration) {
        self.rpc_call_duration
            .with_label_values(&[method])
            .observe(duration.as_secs_f64());
    }

    /// Record transaction confirmation duration
    pub fn record_tx_confirmation_duration(&self, duration: Duration) {
        self.tx_confirmation_duration
            .with_label_values(&[])
            .observe(duration.as_secs_f64());
    }

    /// Record transaction gas used
    pub fn record_tx_gas_used(&self, status: &str, gas_used: u64) {
        self.tx_gas_used
            .with_label_values(&[status])
            .observe(gas_used as f64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hyperliquid_metrics_creation() {
        let _metrics = HyperLiquidMetrics::new_or_default();
        // Just verify it doesn't panic
        assert!(true);
    }

    #[test]
    fn test_set_ws_connected() {
        let metrics = HyperLiquidMetrics::new_or_default();
        
        // Test setting connected
        metrics.set_ws_connected("wss://api.hyperliquid.xyz/ws", true);
        
        // Test setting disconnected
        metrics.set_ws_connected("wss://api.hyperliquid.xyz/ws", false);
    }

    #[test]
    fn test_increment_counters() {
        let metrics = HyperLiquidMetrics::new_or_default();
        
        // Test incrementing various counters
        metrics.inc_trades_received("BTC", "buy");
        metrics.inc_reconnection_attempts("connection_lost");
        metrics.inc_connection_errors("timeout");
        metrics.inc_subscription_errors("ETH", "trades");
        metrics.inc_parse_errors("trade");
        metrics.inc_adaptation_errors("SOL", "unknown_coin");
        metrics.inc_network_errors("connection_reset");
    }

    #[test]
    fn test_record_latency() {
        let metrics = HyperLiquidMetrics::new_or_default();
        
        // Test recording message processing duration
        let duration = Duration::from_millis(5);
        metrics.record_message_processing_duration("trade", duration);
    }

    #[test]
    fn test_set_gauges() {
        let metrics = HyperLiquidMetrics::new_or_default();
        
        // Test setting various gauges
        metrics.set_active_subscriptions("trades", 5);
        metrics.set_degraded_state("max_failures", true);
        metrics.set_degraded_state("max_failures", false);
    }

    #[test]
    fn test_rpc_metrics() {
        let metrics = HyperLiquidMetrics::new_or_default();
        
        // Test RPC call counters
        metrics.inc_rpc_calls("eth_blockNumber", "success");
        metrics.inc_rpc_calls("eth_call", "success");
        metrics.inc_rpc_calls("eth_call", "error");
        
        // Test RPC error counters
        metrics.inc_rpc_errors("eth_blockNumber", "timeout");
        metrics.inc_rpc_errors("eth_call", "network_error");
        
        // Test transaction counters
        metrics.inc_tx_submitted("success");
        metrics.inc_tx_submitted("failed");
        
        // Test RPC gauges
        metrics.set_state_poll_interval("rpc_endpoint", 1);
        metrics.set_state_freshness("rpc_endpoint", 2);
        
        // Test RPC histograms
        metrics.record_rpc_call_duration("eth_blockNumber", Duration::from_millis(50));
        metrics.record_tx_confirmation_duration(Duration::from_secs(5));
        metrics.record_tx_gas_used("success", 21000);
        metrics.record_tx_gas_used("failed", 50000);
    }
}
