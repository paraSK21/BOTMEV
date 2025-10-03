//! Structured logging system with JSON format

use serde_json::json;
use tracing::{Event, Subscriber};
use tracing_subscriber::{
    fmt::{format::Writer, FmtContext, FormatEvent, FormatFields},
    registry::LookupSpan,
    EnvFilter,
};
use std::fmt;

/// JSON formatter for structured logging
pub struct JsonFormatter;

impl<S, N> FormatEvent<S, N> for JsonFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        
        let mut visitor = JsonVisitor::new();
        event.record(&mut visitor);
        
        let json_log = json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "level": metadata.level().to_string(),
            "target": metadata.target(),
            "module": metadata.module_path(),
            "file": metadata.file(),
            "line": metadata.line(),
            "fields": visitor.fields,
            "spans": collect_spans(ctx.lookup_current().as_ref()),
        });
        
        writeln!(writer, "{}", json_log)?;
        Ok(())
    }
}

/// Visitor for collecting event fields
struct JsonVisitor {
    fields: serde_json::Map<String, serde_json::Value>,
}

impl JsonVisitor {
    fn new() -> Self {
        Self {
            fields: serde_json::Map::new(),
        }
    }
}

impl tracing::field::Visit for JsonVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        self.fields.insert(
            field.name().to_string(),
            json!(format!("{:?}", value)),
        );
    }
    
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.fields.insert(field.name().to_string(), json!(value));
    }
    
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields.insert(field.name().to_string(), json!(value));
    }
    
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.insert(field.name().to_string(), json!(value));
    }
    
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.insert(field.name().to_string(), json!(value));
    }
    
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.fields.insert(field.name().to_string(), json!(value));
    }
}

/// Collect span information for context
fn collect_spans<S>(span: Option<&tracing_subscriber::registry::SpanRef<S>>) -> Vec<serde_json::Value>
where
    S: for<'a> LookupSpan<'a>,
{
    let mut spans = Vec::new();
    
    if let Some(span) = span {
        for span in span.scope() {
            let metadata = span.metadata();
            spans.push(json!({
                "name": metadata.name(),
                "target": metadata.target(),
                "level": metadata.level().to_string(),
            }));
        }
    }
    
    spans
}

/// Initialize structured logging
pub fn init_logging(log_level: &str, json_format: bool) -> Result<(), Box<dyn std::error::Error>> {
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(log_level))?;
    
    if json_format {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .event_format(JsonFormatter)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .init();
    }
    
    Ok(())
}

/// Logging macros for MEV-specific events
#[macro_export]
macro_rules! log_opportunity_detected {
    ($strategy:expr, $confidence:expr, $profit:expr, $target_tx:expr) => {
        tracing::info!(
            strategy = $strategy,
            confidence = $confidence,
            estimated_profit_wei = $profit,
            target_tx_hash = $target_tx,
            event_type = "opportunity_detected",
            "MEV opportunity detected"
        );
    };
}

#[macro_export]
macro_rules! log_bundle_created {
    ($bundle_id:expr, $strategy:expr, $tx_count:expr, $gas_limit:expr) => {
        tracing::info!(
            bundle_id = $bundle_id,
            strategy = $strategy,
            transaction_count = $tx_count,
            total_gas_limit = $gas_limit,
            event_type = "bundle_created",
            "Bundle created successfully"
        );
    };
}

#[macro_export]
macro_rules! log_bundle_submitted {
    ($bundle_id:expr, $submission_hash:expr, $gas_price:expr, $target_block:expr) => {
        tracing::info!(
            bundle_id = $bundle_id,
            submission_hash = $submission_hash,
            gas_price_gwei = $gas_price,
            target_block = $target_block,
            event_type = "bundle_submitted",
            "Bundle submitted to mempool"
        );
    };
}

#[macro_export]
macro_rules! log_bundle_included {
    ($bundle_id:expr, $block_number:expr, $position:expr, $profit:expr) => {
        tracing::info!(
            bundle_id = $bundle_id,
            block_number = $block_number,
            block_position = $position,
            realized_profit_wei = $profit,
            event_type = "bundle_included",
            "Bundle successfully included in block"
        );
    };
}

#[macro_export]
macro_rules! log_bundle_failed {
    ($bundle_id:expr, $reason:expr, $gas_cost:expr) => {
        tracing::warn!(
            bundle_id = $bundle_id,
            failure_reason = $reason,
            gas_cost_wei = $gas_cost,
            event_type = "bundle_failed",
            "Bundle execution failed"
        );
    };
}

#[macro_export]
macro_rules! log_simulation_result {
    ($bundle_id:expr, $success:expr, $gas_used:expr, $profit:expr, $duration_ms:expr) => {
        tracing::debug!(
            bundle_id = $bundle_id,
            simulation_success = $success,
            gas_used = $gas_used,
            simulated_profit_wei = $profit,
            simulation_duration_ms = $duration_ms,
            event_type = "simulation_completed",
            "Bundle simulation completed"
        );
    };
}

#[macro_export]
macro_rules! log_error {
    ($component:expr, $error_type:expr, $error_msg:expr) => {
        tracing::error!(
            component = $component,
            error_type = $error_type,
            error_message = $error_msg,
            event_type = "error",
            "System error occurred"
        );
    };
}

#[macro_export]
macro_rules! log_performance {
    ($operation:expr, $duration_ms:expr, $throughput:expr) => {
        tracing::debug!(
            operation = $operation,
            duration_ms = $duration_ms,
            throughput = $throughput,
            event_type = "performance",
            "Performance measurement"
        );
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::info;
    
    #[test]
    fn test_json_formatter() {
        // This test would require more setup to properly test the formatter
        // For now, just ensure it compiles
        let _formatter = JsonFormatter;
    }
    
    #[test]
    fn test_logging_macros() {
        // Test that macros compile correctly
        log_opportunity_detected!("arbitrage", 0.85, 1000000000000000000u64, "0x123");
        log_bundle_created!("bundle_1", "arbitrage", 3, 500000);
        log_bundle_submitted!("bundle_1", "0xabc", 50, 1000);
        log_bundle_included!("bundle_1", 1001, 2, 1500000000000000000u64);
        log_bundle_failed!("bundle_1", "gas_underpriced", 100000000000000000u64);
        log_simulation_result!("bundle_1", true, 300000, 2000000000000000000u64, 150);
        log_error!("mempool", "connection_timeout", "RPC connection failed");
        log_performance!("bundle_creation", 25, 40.5);
    }
}