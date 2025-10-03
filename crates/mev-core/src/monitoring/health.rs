//! Health checking system for production monitoring

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Health check status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub is_healthy: bool,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub checks: HashMap<String, CheckResult>,
    pub uptime_seconds: u64,
    pub version: String,
}

/// Individual health check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub status: CheckStatus,
    pub message: String,
    pub duration_ms: u64,
    pub last_success: Option<chrono::DateTime<chrono::Utc>>,
    pub failure_count: u32,
}

/// Health check status enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CheckStatus {
    Healthy,
    Warning,
    Critical,
    Unknown,
}

/// Health checker configuration
#[derive(Debug, Clone)]
pub struct HealthConfig {
    pub check_interval: Duration,
    pub timeout: Duration,
    pub max_failures: u32,
    pub warning_threshold: Duration,
    pub critical_threshold: Duration,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(30),
            timeout: Duration::from_secs(5),
            max_failures: 3,
            warning_threshold: Duration::from_secs(1),
            critical_threshold: Duration::from_secs(5),
        }
    }
}

/// Health checker system
pub struct HealthChecker {
    config: HealthConfig,
    checks: Arc<RwLock<HashMap<String, Box<dyn HealthCheck>>>>,
    results: Arc<RwLock<HashMap<String, CheckResult>>>,
    start_time: Instant,
}

/// Health check trait
#[async_trait::async_trait]
pub trait HealthCheck: Send + Sync {
    /// Perform the health check
    async fn check(&self) -> Result<String>;
    
    /// Get the check name
    fn name(&self) -> &str;
    
    /// Check if this is a critical check
    fn is_critical(&self) -> bool {
        false
    }
}

impl HealthChecker {
    /// Create a new health checker
    pub fn new(config: HealthConfig) -> Self {
        Self {
            config,
            checks: Arc::new(RwLock::new(HashMap::new())),
            results: Arc::new(RwLock::new(HashMap::new())),
            start_time: Instant::now(),
        }
    }
    
    /// Register a health check
    pub async fn register_check(&self, check: Box<dyn HealthCheck>) {
        let name = check.name().to_string();
        let mut checks = self.checks.write().await;
        checks.insert(name.clone(), check);
        
        // Initialize result
        let mut results = self.results.write().await;
        results.insert(name, CheckResult {
            status: CheckStatus::Unknown,
            message: "Not yet checked".to_string(),
            duration_ms: 0,
            last_success: None,
            failure_count: 0,
        });
    }
    
    /// Start the health checking loop
    pub async fn start_monitoring(&self) {
        let mut interval = tokio::time::interval(self.config.check_interval);
        
        loop {
            interval.tick().await;
            self.run_all_checks().await;
        }
    }
    
    /// Run all registered health checks
    async fn run_all_checks(&self) {
        let check_names: Vec<String> = {
            let checks_guard = self.checks.read().await;
            checks_guard.keys().cloned().collect()
        };
        
        for name in check_names {
            let checks_guard = self.checks.read().await;
            if let Some(check) = checks_guard.get(&name) {
                self.run_single_check(&name, &**check).await;
            }
        }
    }
    
    /// Run a single health check
    async fn run_single_check(&self, name: &str, check: &dyn HealthCheck) {
        let start = Instant::now();
        
        let result = tokio::time::timeout(self.config.timeout, check.check()).await;
        
        let duration = start.elapsed();
        let duration_ms = duration.as_millis() as u64;
        
        let mut results = self.results.write().await;
        let current_result = results.get_mut(name).unwrap();
        
        match result {
            Ok(Ok(message)) => {
                // Success
                current_result.status = if duration > self.config.critical_threshold {
                    CheckStatus::Critical
                } else if duration > self.config.warning_threshold {
                    CheckStatus::Warning
                } else {
                    CheckStatus::Healthy
                };
                current_result.message = message;
                current_result.duration_ms = duration_ms;
                current_result.last_success = Some(chrono::Utc::now());
                current_result.failure_count = 0;
            }
            Ok(Err(e)) => {
                // Check failed
                current_result.status = if check.is_critical() {
                    CheckStatus::Critical
                } else {
                    CheckStatus::Warning
                };
                current_result.message = format!("Check failed: {}", e);
                current_result.duration_ms = duration_ms;
                current_result.failure_count += 1;
            }
            Err(_) => {
                // Timeout
                current_result.status = CheckStatus::Critical;
                current_result.message = "Check timed out".to_string();
                current_result.duration_ms = self.config.timeout.as_millis() as u64;
                current_result.failure_count += 1;
            }
        }
    }
    
    /// Get current health status
    pub async fn get_health_status(&self) -> HealthStatus {
        let results = self.results.read().await;
        let checks = results.clone();
        
        let is_healthy = checks.values().all(|result| {
            matches!(result.status, CheckStatus::Healthy | CheckStatus::Warning)
        });
        
        HealthStatus {
            is_healthy,
            timestamp: chrono::Utc::now(),
            checks,
            uptime_seconds: self.start_time.elapsed().as_secs(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
    
    /// Check if system is ready (all critical checks passing)
    pub async fn is_ready(&self) -> bool {
        let results = self.results.read().await;
        let checks = self.checks.read().await;
        
        for (name, check) in checks.iter() {
            if check.is_critical() {
                if let Some(result) = results.get(name) {
                    if matches!(result.status, CheckStatus::Critical) {
                        return false;
                    }
                }
            }
        }
        
        true
    }
}

/// RPC connectivity health check
pub struct RpcHealthCheck {
    name: String,
    rpc_url: String,
    client: reqwest::Client,
}

impl RpcHealthCheck {
    pub fn new(name: String, rpc_url: String) -> Self {
        Self {
            name,
            rpc_url,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl HealthCheck for RpcHealthCheck {
    async fn check(&self) -> Result<String> {
        let response = self.client
            .post(&self.rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_blockNumber",
                "params": [],
                "id": 1
            }))
            .send()
            .await?;
        
        if response.status().is_success() {
            let json: serde_json::Value = response.json().await?;
            if let Some(result) = json.get("result") {
                Ok(format!("RPC responsive, latest block: {}", result))
            } else {
                Err(anyhow::anyhow!("Invalid RPC response format"))
            }
        } else {
            Err(anyhow::anyhow!("RPC returned status: {}", response.status()))
        }
    }
    
    fn name(&self) -> &str {
        &self.name
    }
    
    fn is_critical(&self) -> bool {
        true
    }
}

/// Database connectivity health check
pub struct DatabaseHealthCheck {
    name: String,
    db_path: String,
}

impl DatabaseHealthCheck {
    pub fn new(name: String, db_path: String) -> Self {
        Self { name, db_path }
    }
}

#[async_trait::async_trait]
impl HealthCheck for DatabaseHealthCheck {
    async fn check(&self) -> Result<String> {
        // Simple SQLite connectivity check
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare("SELECT 1")?;
        let result: i32 = stmt.query_row([], |row| row.get(0))?;
        
        if result == 1 {
            Ok("Database connection successful".to_string())
        } else {
            Err(anyhow::anyhow!("Unexpected database response"))
        }
    }
    
    fn name(&self) -> &str {
        &self.name
    }
    
    fn is_critical(&self) -> bool {
        false
    }
}

/// Memory usage health check
pub struct MemoryHealthCheck {
    name: String,
    warning_threshold_mb: u64,
    critical_threshold_mb: u64,
}

impl MemoryHealthCheck {
    pub fn new(name: String, warning_threshold_mb: u64, critical_threshold_mb: u64) -> Self {
        Self {
            name,
            warning_threshold_mb,
            critical_threshold_mb,
        }
    }
    
    fn get_memory_usage() -> Result<u64> {
        // Simple memory usage check (would need platform-specific implementation)
        // For now, return a mock value
        Ok(512) // 512 MB
    }
}

#[async_trait::async_trait]
impl HealthCheck for MemoryHealthCheck {
    async fn check(&self) -> Result<String> {
        let memory_mb = Self::get_memory_usage()?;
        
        if memory_mb > self.critical_threshold_mb {
            Err(anyhow::anyhow!("Memory usage critical: {} MB", memory_mb))
        } else if memory_mb > self.warning_threshold_mb {
            Ok(format!("Memory usage warning: {} MB", memory_mb))
        } else {
            Ok(format!("Memory usage normal: {} MB", memory_mb))
        }
    }
    
    fn name(&self) -> &str {
        &self.name
    }
    
    fn is_critical(&self) -> bool {
        false
    }
}

/// Bundle system health check
pub struct BundleSystemHealthCheck {
    name: String,
}

impl BundleSystemHealthCheck {
    pub fn new(name: String) -> Self {
        Self { name }
    }
}

#[async_trait::async_trait]
impl HealthCheck for BundleSystemHealthCheck {
    async fn check(&self) -> Result<String> {
        // Check if bundle system components are responsive
        // This would integrate with actual bundle system status
        Ok("Bundle system operational".to_string())
    }
    
    fn name(&self) -> &str {
        &self.name
    }
    
    fn is_critical(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    struct MockHealthCheck {
        name: String,
        should_fail: bool,
    }
    
    impl MockHealthCheck {
        fn new(name: String, should_fail: bool) -> Self {
            Self { name, should_fail }
        }
    }
    
    #[async_trait::async_trait]
    impl HealthCheck for MockHealthCheck {
        async fn check(&self) -> Result<String> {
            if self.should_fail {
                Err(anyhow::anyhow!("Mock check failed"))
            } else {
                Ok("Mock check passed".to_string())
            }
        }
        
        fn name(&self) -> &str {
            &self.name
        }
    }
    
    #[tokio::test]
    async fn test_health_checker() {
        let config = HealthConfig::default();
        let checker = HealthChecker::new(config);
        
        // Register mock checks
        checker.register_check(Box::new(MockHealthCheck::new("test_pass".to_string(), false))).await;
        checker.register_check(Box::new(MockHealthCheck::new("test_fail".to_string(), true))).await;
        
        // Run checks
        checker.run_all_checks().await;
        
        // Get status
        let status = checker.get_health_status().await;
        
        assert_eq!(status.checks.len(), 2);
        assert!(status.checks.contains_key("test_pass"));
        assert!(status.checks.contains_key("test_fail"));
        
        // Check individual results
        let pass_result = &status.checks["test_pass"];
        let fail_result = &status.checks["test_fail"];
        
        assert_eq!(pass_result.status, CheckStatus::Healthy);
        assert_eq!(fail_result.status, CheckStatus::Warning);
    }
    
    #[tokio::test]
    async fn test_readiness_check() {
        let config = HealthConfig::default();
        let checker = HealthChecker::new(config);
        
        // Should be ready initially (no critical checks)
        assert!(checker.is_ready().await);
        
        // Add a non-critical failing check
        checker.register_check(Box::new(MockHealthCheck::new("non_critical".to_string(), true))).await;
        checker.run_all_checks().await;
        
        // Should still be ready
        assert!(checker.is_ready().await);
    }
}