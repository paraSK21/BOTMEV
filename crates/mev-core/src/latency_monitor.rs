//! Latency monitoring and histogram reporting for ultra-low latency decision path
//! 
//! This module provides comprehensive latency measurement and reporting capabilities
//! specifically designed for the MEV bot's decision loop latency requirements.

use crate::metrics::{LatencyHistogram, PrometheusMetrics};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Latency measurement categories
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum LatencyCategory {
    /// Mempool detection latency (broadcast → local detection)
    MempoolDetection,
    /// Filter stage latency (detection → filtered)
    FilterStage,
    /// Simulation stage latency (filtered → simulated)
    SimulationStage,
    /// Bundle stage latency (simulated → bundle)
    BundleStage,
    /// Total decision loop latency (detection → bundle)
    TotalDecisionLoop,
    /// End-to-end latency (broadcast → bundle)
    EndToEnd,
}

impl LatencyCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MempoolDetection => "mempool_detection",
            Self::FilterStage => "filter_stage",
            Self::SimulationStage => "simulation_stage",
            Self::BundleStage => "bundle_stage",
            Self::TotalDecisionLoop => "total_decision_loop",
            Self::EndToEnd => "end_to_end",
        }
    }
}

/// Latency measurement point
#[derive(Debug, Clone)]
pub struct LatencyMeasurement {
    pub category: LatencyCategory,
    pub latency_ms: f64,
    pub timestamp: Instant,
    pub transaction_id: String,
    pub metadata: HashMap<String, String>,
}

/// Latency statistics for a specific category
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyStats {
    pub category: LatencyCategory,
    pub count: u64,
    pub mean_ms: f64,
    pub median_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub std_dev_ms: f64,
    pub target_ms: f64,
    pub target_compliance_rate: f64,
}

/// Latency target configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyTargets {
    pub mempool_detection_ms: f64,
    pub filter_stage_ms: f64,
    pub simulation_stage_ms: f64,
    pub bundle_stage_ms: f64,
    pub total_decision_loop_ms: f64,
    pub end_to_end_ms: f64,
}

impl Default for LatencyTargets {
    fn default() -> Self {
        Self {
            mempool_detection_ms: 20.0,
            filter_stage_ms: 5.0,
            simulation_stage_ms: 15.0,
            bundle_stage_ms: 5.0,
            total_decision_loop_ms: 25.0,
            end_to_end_ms: 45.0,
        }
    }
}

/// Configuration for latency monitoring
#[derive(Debug, Clone)]
pub struct LatencyMonitorConfig {
    pub targets: LatencyTargets,
    pub histogram_size: usize,
    pub reporting_interval_seconds: u64,
    pub enable_detailed_logging: bool,
    pub alert_threshold_multiplier: f64,
}

impl Default for LatencyMonitorConfig {
    fn default() -> Self {
        Self {
            targets: LatencyTargets::default(),
            histogram_size: 10000,
            reporting_interval_seconds: 30,
            enable_detailed_logging: false,
            alert_threshold_multiplier: 2.0,
        }
    }
}

/// Comprehensive latency monitoring system
pub struct LatencyMonitor {
    config: LatencyMonitorConfig,
    histograms: Arc<RwLock<HashMap<LatencyCategory, LatencyHistogram>>>,
    measurements: Arc<RwLock<Vec<LatencyMeasurement>>>,
    metrics: Arc<PrometheusMetrics>,
    
    // Performance counters
    total_measurements: Arc<AtomicU64>,
    target_violations: Arc<RwLock<HashMap<LatencyCategory, AtomicU64>>>,
    
    // Control
    start_time: Instant,
}

impl LatencyMonitor {
    /// Create new latency monitor
    pub fn new(config: LatencyMonitorConfig, metrics: Arc<PrometheusMetrics>) -> Self {
        let histograms = Arc::new(RwLock::new(HashMap::new()));
        let target_violations = Arc::new(RwLock::new(HashMap::new()));

        // Initialize histograms for each category
        let categories = [
            LatencyCategory::MempoolDetection,
            LatencyCategory::FilterStage,
            LatencyCategory::SimulationStage,
            LatencyCategory::BundleStage,
            LatencyCategory::TotalDecisionLoop,
            LatencyCategory::EndToEnd,
        ];

        tokio::spawn({
            let histograms = histograms.clone();
            let target_violations = target_violations.clone();
            let histogram_size = config.histogram_size;
            
            async move {
                let mut hist_map = histograms.write().await;
                let mut violations_map = target_violations.write().await;
                
                for category in &categories {
                    hist_map.insert(category.clone(), LatencyHistogram::new(histogram_size));
                    violations_map.insert(category.clone(), AtomicU64::new(0));
                }
            }
        });

        Self {
            config,
            histograms,
            measurements: Arc::new(RwLock::new(Vec::new())),
            metrics,
            total_measurements: Arc::new(AtomicU64::new(0)),
            target_violations,
            start_time: Instant::now(),
        }
    }

    /// Record a latency measurement
    pub async fn record_latency(
        &self,
        category: LatencyCategory,
        latency_ms: f64,
        transaction_id: String,
        metadata: Option<HashMap<String, String>>,
    ) {
        let measurement = LatencyMeasurement {
            category: category.clone(),
            latency_ms,
            timestamp: Instant::now(),
            transaction_id: transaction_id.clone(),
            metadata: metadata.unwrap_or_default(),
        };

        // Record in histogram
        {
            let histograms = self.histograms.read().await;
            if let Some(histogram) = histograms.get(&category) {
                histogram.record(latency_ms);
            }
        }

        // Check against target
        let target = self.get_target_for_category(&category);
        if latency_ms > target {
            // Record target violation
            {
                let violations = self.target_violations.read().await;
                if let Some(counter) = violations.get(&category) {
                    counter.fetch_add(1, Ordering::Relaxed);
                }
            }

            // Log warning if significantly over target
            if latency_ms > target * self.config.alert_threshold_multiplier {
                warn!(
                    category = category.as_str(),
                    latency_ms = format!("{:.2}", latency_ms),
                    target_ms = format!("{:.2}", target),
                    tx_id = %transaction_id,
                    "Latency significantly exceeded target"
                );
            }
        }

        // Record in Prometheus metrics
        self.metrics.record_decision_latency(
            Duration::from_millis(latency_ms as u64),
            category.as_str(),
            if latency_ms <= target { "within_target" } else { "exceeded_target" },
        );

        // Store measurement for detailed analysis
        if self.config.enable_detailed_logging {
            let mut measurements = self.measurements.write().await;
            measurements.push(measurement);
            
            // Limit stored measurements to prevent memory growth
            if measurements.len() > self.config.histogram_size {
                measurements.remove(0);
            }
        }

        // Update total counter
        self.total_measurements.fetch_add(1, Ordering::Relaxed);

        // Log detailed measurement if enabled
        if self.config.enable_detailed_logging {
            debug!(
                category = category.as_str(),
                latency_ms = format!("{:.3}", latency_ms),
                target_ms = format!("{:.1}", target),
                tx_id = %transaction_id,
                "Latency measurement recorded"
            );
        }
    }

    /// Get latency statistics for a category
    pub async fn get_stats(&self, category: &LatencyCategory) -> Option<LatencyStats> {
        let histograms = self.histograms.read().await;
        let histogram = histograms.get(category)?;
        
        let histogram_stats = histogram.get_stats();
        let target = self.get_target_for_category(category);
        
        // Calculate target compliance rate
        let violations = {
            let violations_map = self.target_violations.read().await;
            violations_map.get(category)?.load(Ordering::Relaxed)
        };
        
        let compliance_rate = if histogram_stats.count > 0 {
            1.0 - (violations as f64 / histogram_stats.count as f64)
        } else {
            1.0
        };

        // Calculate standard deviation (simplified)
        let std_dev = if histogram_stats.count > 1 {
            // This is a simplified calculation - in practice you'd calculate proper std dev
            (histogram_stats.p95_ms - histogram_stats.median_ms) / 2.0
        } else {
            0.0
        };

        Some(LatencyStats {
            category: category.clone(),
            count: histogram_stats.count,
            mean_ms: histogram_stats.mean_ms,
            median_ms: histogram_stats.median_ms,
            p95_ms: histogram_stats.p95_ms,
            p99_ms: histogram_stats.p99_ms,
            min_ms: 0.0, // Would need to track separately
            max_ms: 0.0, // Would need to track separately
            std_dev_ms: std_dev,
            target_ms: target,
            target_compliance_rate: compliance_rate,
        })
    }

    /// Get comprehensive latency report
    pub async fn get_comprehensive_report(&self) -> LatencyReport {
        let categories = [
            LatencyCategory::MempoolDetection,
            LatencyCategory::FilterStage,
            LatencyCategory::SimulationStage,
            LatencyCategory::BundleStage,
            LatencyCategory::TotalDecisionLoop,
            LatencyCategory::EndToEnd,
        ];

        let mut category_stats = HashMap::new();
        
        for category in &categories {
            if let Some(stats) = self.get_stats(category).await {
                category_stats.insert(category.clone(), stats);
            }
        }

        let uptime = self.start_time.elapsed();
        let total_measurements = self.total_measurements.load(Ordering::Relaxed);
        let throughput = total_measurements as f64 / uptime.as_secs_f64();

        LatencyReport {
            timestamp: chrono::Utc::now(),
            uptime_seconds: uptime.as_secs(),
            total_measurements,
            throughput_measurements_per_second: throughput,
            category_stats,
            targets: self.config.targets.clone(),
        }
    }

    /// Start periodic reporting
    pub async fn start_periodic_reporting(&self) {
        let monitor = self.clone();
        let interval_seconds = self.config.reporting_interval_seconds;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));
            
            loop {
                interval.tick().await;
                
                let report = monitor.get_comprehensive_report().await;
                monitor.log_performance_report(&report).await;
            }
        });

        info!(
            interval_seconds = interval_seconds,
            "Started periodic latency reporting"
        );
    }

    /// Log performance report
    async fn log_performance_report(&self, report: &LatencyReport) {
        info!("📊 Latency Performance Report ({}s uptime)", report.uptime_seconds);
        info!("   Total Measurements: {} ({:.1}/sec)", report.total_measurements, report.throughput_measurements_per_second);

        // Report each category
        for (category, stats) in &report.category_stats {
            let compliance_status = if stats.target_compliance_rate >= 0.95 {
                "✅"
            } else if stats.target_compliance_rate >= 0.90 {
                "⚠️"
            } else {
                "❌"
            };

            let latency_status = if stats.median_ms <= stats.target_ms {
                "✅"
            } else if stats.median_ms <= stats.target_ms * 1.5 {
                "⚠️"
            } else {
                "❌"
            };

            info!(
                "   {} {}: {:.2}ms median (target: {:.1}ms) {} | P95: {:.2}ms | Compliance: {:.1}% {}",
                latency_status,
                format!("{:?}", category),
                stats.median_ms,
                stats.target_ms,
                latency_status,
                stats.p95_ms,
                stats.target_compliance_rate * 100.0,
                compliance_status
            );
        }

        // Overall assessment
        if let Some(decision_loop_stats) = report.category_stats.get(&LatencyCategory::TotalDecisionLoop) {
            let overall_status = if decision_loop_stats.median_ms <= decision_loop_stats.target_ms {
                "🏆 MEETING TARGETS"
            } else if decision_loop_stats.median_ms <= decision_loop_stats.target_ms * 1.2 {
                "⚠️ CLOSE TO TARGET"
            } else {
                "❌ MISSING TARGETS"
            };

            info!("   Overall Status: {}", overall_status);
        }
    }

    /// Get target latency for a category
    fn get_target_for_category(&self, category: &LatencyCategory) -> f64 {
        match category {
            LatencyCategory::MempoolDetection => self.config.targets.mempool_detection_ms,
            LatencyCategory::FilterStage => self.config.targets.filter_stage_ms,
            LatencyCategory::SimulationStage => self.config.targets.simulation_stage_ms,
            LatencyCategory::BundleStage => self.config.targets.bundle_stage_ms,
            LatencyCategory::TotalDecisionLoop => self.config.targets.total_decision_loop_ms,
            LatencyCategory::EndToEnd => self.config.targets.end_to_end_ms,
        }
    }

    /// Export latency data for external analysis
    pub async fn export_measurements(&self) -> Vec<LatencyMeasurement> {
        let measurements = self.measurements.read().await;
        measurements.clone()
    }

    /// Reset all statistics
    pub async fn reset_stats(&self) {
        {
            let mut histograms = self.histograms.write().await;
            for _histogram in histograms.values_mut() {
                // Reset histogram (would need to implement reset method)
            }
        }

        {
            let violations = self.target_violations.read().await;
            for counter in violations.values() {
                counter.store(0, Ordering::Relaxed);
            }
        }

        {
            let mut measurements = self.measurements.write().await;
            measurements.clear();
        }

        self.total_measurements.store(0, Ordering::Relaxed);

        info!("Latency monitor statistics reset");
    }
}

impl Clone for LatencyMonitor {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            histograms: self.histograms.clone(),
            measurements: self.measurements.clone(),
            metrics: self.metrics.clone(),
            total_measurements: self.total_measurements.clone(),
            target_violations: self.target_violations.clone(),
            start_time: self.start_time,
        }
    }
}

/// Comprehensive latency report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyReport {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub uptime_seconds: u64,
    pub total_measurements: u64,
    pub throughput_measurements_per_second: f64,
    pub category_stats: HashMap<LatencyCategory, LatencyStats>,
    pub targets: LatencyTargets,
}

impl LatencyReport {
    /// Export report as JSON
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Check if all targets are being met
    pub fn all_targets_met(&self) -> bool {
        self.category_stats.values().all(|stats| stats.median_ms <= stats.target_ms)
    }

    /// Get worst performing category
    pub fn worst_performing_category(&self) -> Option<(&LatencyCategory, &LatencyStats)> {
        self.category_stats
            .iter()
            .max_by(|(_, a), (_, b)| {
                let a_ratio = a.median_ms / a.target_ms;
                let b_ratio = b.median_ms / b.target_ms;
                a_ratio.partial_cmp(&b_ratio).unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Get overall performance grade
    pub fn performance_grade(&self) -> &'static str {
        if let Some(decision_stats) = self.category_stats.get(&LatencyCategory::TotalDecisionLoop) {
            let ratio = decision_stats.median_ms / decision_stats.target_ms;
            if ratio <= 0.8 {
                "A+ (Excellent)"
            } else if ratio <= 1.0 {
                "A (Good)"
            } else if ratio <= 1.2 {
                "B (Acceptable)"
            } else if ratio <= 1.5 {
                "C (Needs Improvement)"
            } else {
                "D (Poor)"
            }
        } else {
            "N/A"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_latency_monitor_creation() {
        let config = LatencyMonitorConfig::default();
        let metrics = Arc::new(PrometheusMetrics::new().unwrap());
        let monitor = LatencyMonitor::new(config, metrics);
        
        // Give time for initialization
        tokio::time::sleep(Duration::from_millis(10)).await;
        
        let report = monitor.get_comprehensive_report().await;
        assert_eq!(report.total_measurements, 0);
    }

    #[tokio::test]
    async fn test_latency_recording() {
        let config = LatencyMonitorConfig::default();
        let metrics = Arc::new(PrometheusMetrics::new().unwrap());
        let monitor = LatencyMonitor::new(config, metrics);
        
        // Give time for initialization
        tokio::time::sleep(Duration::from_millis(10)).await;
        
        monitor.record_latency(
            LatencyCategory::FilterStage,
            3.5,
            "test_tx_1".to_string(),
            None,
        ).await;

        let stats = monitor.get_stats(&LatencyCategory::FilterStage).await;
        assert!(stats.is_some());
        
        let stats = stats.unwrap();
        assert_eq!(stats.count, 1);
        assert_eq!(stats.mean_ms, 3.5);
    }

    #[test]
    fn test_latency_targets() {
        let targets = LatencyTargets::default();
        assert_eq!(targets.total_decision_loop_ms, 25.0);
        assert_eq!(targets.filter_stage_ms, 5.0);
        assert_eq!(targets.simulation_stage_ms, 15.0);
    }

    #[test]
    fn test_latency_category_string() {
        assert_eq!(LatencyCategory::TotalDecisionLoop.as_str(), "total_decision_loop");
        assert_eq!(LatencyCategory::FilterStage.as_str(), "filter_stage");
    }
}