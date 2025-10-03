//! Failure handling and security measures for MEV bot operations


use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Mutex};

/// Failure types that can occur during MEV operations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FailureType {
    /// Nonce collision with another transaction
    NonceCollision { expected: u64, actual: u64 },
    /// Gas price too low for inclusion
    GasUnderpricing { required: u64, provided: u64 },
    /// Blockchain reorganization affected bundle
    Reorg { original_block: u64, new_block: u64 },
    /// Transaction evicted from mempool
    MempoolEviction { reason: String },
    /// Bundle execution failed
    ExecutionFailure { error: String },
    /// Network connectivity issues
    NetworkError { error: String },
    /// Insufficient funds for transaction
    InsufficientFunds { required: u64, available: u64 },
    /// Smart contract revert
    ContractRevert { reason: String },
}

/// Failure handling configuration
#[derive(Debug, Clone)]
pub struct FailureConfig {
    /// Maximum retry attempts per failure type
    pub max_retries: HashMap<FailureType, u32>,
    /// Retry delays per failure type
    pub retry_delays: HashMap<FailureType, Duration>,
    /// Enable automatic rollback on failure
    pub auto_rollback: bool,
    /// Maximum rollback attempts
    pub max_rollback_attempts: u32,
    /// Enable failure persistence for analysis
    pub persist_failures: bool,
}

impl Default for FailureConfig {
    fn default() -> Self {
        let mut max_retries = HashMap::new();
        max_retries.insert(FailureType::NonceCollision { expected: 0, actual: 0 }, 3);
        max_retries.insert(FailureType::GasUnderpricing { required: 0, provided: 0 }, 2);
        max_retries.insert(FailureType::MempoolEviction { reason: String::new() }, 1);
        max_retries.insert(FailureType::NetworkError { error: String::new() }, 5);
        
        let mut retry_delays = HashMap::new();
        retry_delays.insert(FailureType::NonceCollision { expected: 0, actual: 0 }, Duration::from_secs(2));
        retry_delays.insert(FailureType::GasUnderpricing { required: 0, provided: 0 }, Duration::from_secs(5));
        retry_delays.insert(FailureType::MempoolEviction { reason: String::new() }, Duration::from_secs(10));
        retry_delays.insert(FailureType::NetworkError { error: String::new() }, Duration::from_secs(1));
        
        Self {
            max_retries,
            retry_delays,
            auto_rollback: true,
            max_rollback_attempts: 3,
            persist_failures: true,
        }
    }
}

/// Failure record for analysis and persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureRecord {
    pub id: String,
    pub bundle_id: String,
    pub failure_type: FailureType,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub retry_count: u32,
    pub resolved: bool,
    pub resolution_method: Option<String>,
    pub gas_cost: u64,
    pub metadata: HashMap<String, String>,
}

/// Rollback operation for failed transactions
#[derive(Debug, Clone)]
pub struct RollbackOperation {
    pub id: String,
    pub bundle_id: String,
    pub rollback_transactions: Vec<String>,
    pub created_at: Instant,
    pub status: RollbackStatus,
}

/// Status of rollback operation
#[derive(Debug, Clone, PartialEq)]
pub enum RollbackStatus {
    Pending,
    InProgress,
    Completed,
    Failed { reason: String },
}

/// Kill switch for emergency shutdown
#[derive(Debug, Clone)]
pub struct KillSwitch {
    /// Whether the kill switch is activated
    pub activated: bool,
    /// Reason for activation
    pub reason: Option<String>,
    /// Activation timestamp
    pub activated_at: Option<Instant>,
    /// Who activated it
    pub activated_by: Option<String>,
}

/// Main failure handler
pub struct FailureHandler {
    config: FailureConfig,
    /// Active failure records
    failures: Arc<RwLock<HashMap<String, FailureRecord>>>,
    /// Rollback operations
    rollbacks: Arc<RwLock<HashMap<String, RollbackOperation>>>,
    /// Kill switch state
    kill_switch: Arc<Mutex<KillSwitch>>,
    /// Failure statistics
    stats: Arc<RwLock<FailureStats>>,
}

/// Failure statistics for monitoring
#[derive(Debug, Clone, Default)]
pub struct FailureStats {
    pub total_failures: u64,
    pub failures_by_type: HashMap<String, u64>,
    pub successful_recoveries: u64,
    pub failed_recoveries: u64,
    pub rollbacks_executed: u64,
    pub kill_switch_activations: u64,
}

impl FailureHandler {
    /// Create a new failure handler
    pub fn new(config: FailureConfig) -> Self {
        Self {
            config,
            failures: Arc::new(RwLock::new(HashMap::new())),
            rollbacks: Arc::new(RwLock::new(HashMap::new())),
            kill_switch: Arc::new(Mutex::new(KillSwitch {
                activated: false,
                reason: None,
                activated_at: None,
                activated_by: None,
            })),
            stats: Arc::new(RwLock::new(FailureStats::default())),
        }
    }
    
    /// Handle a failure occurrence
    pub async fn handle_failure(
        &self,
        bundle_id: &str,
        failure_type: FailureType,
        metadata: HashMap<String, String>,
    ) -> Result<FailureResolution> {
        // Check kill switch first
        if self.is_kill_switch_activated().await {
            return Ok(FailureResolution::KillSwitchActivated);
        }
        
        let failure_id = format!("failure_{}", uuid::Uuid::new_v4());
        
        // Create failure record
        let failure_record = FailureRecord {
            id: failure_id.clone(),
            bundle_id: bundle_id.to_string(),
            failure_type: failure_type.clone(),
            timestamp: chrono::Utc::now(),
            retry_count: 0,
            resolved: false,
            resolution_method: None,
            gas_cost: 0,
            metadata,
        };
        
        // Store failure record
        {
            let mut failures = self.failures.write().await;
            failures.insert(failure_id.clone(), failure_record);
        }
        
        // Update statistics
        self.update_failure_stats(&failure_type).await;
        
        tracing::warn!("Handling failure for bundle {}: {:?}", bundle_id, failure_type);
        
        // Determine resolution strategy
        let resolution = self.determine_resolution_strategy(&failure_type).await?;
        
        match resolution {
            FailureResolution::Retry { delay, max_attempts } => {
                self.handle_retry(&failure_id, delay, max_attempts).await
            }
            FailureResolution::Rollback => {
                self.initiate_rollback(bundle_id).await
            }
            FailureResolution::Abort => {
                self.abort_operation(&failure_id).await
            }
            FailureResolution::KillSwitch => {
                self.activate_kill_switch("Automatic activation due to critical failure", None).await
            }
            _ => Ok(resolution),
        }
    }
    
    /// Determine the appropriate resolution strategy
    async fn determine_resolution_strategy(&self, failure_type: &FailureType) -> Result<FailureResolution> {
        match failure_type {
            FailureType::NonceCollision { .. } => {
                Ok(FailureResolution::Retry {
                    delay: Duration::from_secs(2),
                    max_attempts: 3,
                })
            }
            FailureType::GasUnderpricing { .. } => {
                Ok(FailureResolution::Retry {
                    delay: Duration::from_secs(5),
                    max_attempts: 2,
                })
            }
            FailureType::Reorg { .. } => {
                Ok(FailureResolution::Rollback)
            }
            FailureType::MempoolEviction { .. } => {
                Ok(FailureResolution::Retry {
                    delay: Duration::from_secs(10),
                    max_attempts: 1,
                })
            }
            FailureType::ExecutionFailure { .. } => {
                Ok(FailureResolution::Rollback)
            }
            FailureType::NetworkError { .. } => {
                Ok(FailureResolution::Retry {
                    delay: Duration::from_secs(1),
                    max_attempts: 5,
                })
            }
            FailureType::InsufficientFunds { .. } => {
                Ok(FailureResolution::Abort)
            }
            FailureType::ContractRevert { .. } => {
                Ok(FailureResolution::Rollback)
            }
        }
    }
    
    /// Handle retry logic
    async fn handle_retry(
        &self,
        failure_id: &str,
        delay: Duration,
        max_attempts: u32,
    ) -> Result<FailureResolution> {
        let mut failures = self.failures.write().await;
        let failure = failures.get_mut(failure_id)
            .ok_or_else(|| anyhow!("Failure record not found: {}", failure_id))?;
        
        failure.retry_count += 1;
        
        if failure.retry_count >= max_attempts {
            tracing::error!("Max retry attempts reached for failure {}", failure_id);
            failure.resolved = true;
            failure.resolution_method = Some("max_retries_exceeded".to_string());
            
            // Escalate to rollback
            return Ok(FailureResolution::Rollback);
        }
        
        tracing::info!("Retrying operation for failure {} (attempt {}/{})", 
                      failure_id, failure.retry_count, max_attempts);
        
        // Wait for retry delay
        tokio::time::sleep(delay).await;
        
        Ok(FailureResolution::Retry { delay, max_attempts })
    }
    
    /// Initiate rollback operation
    async fn initiate_rollback(&self, bundle_id: &str) -> Result<FailureResolution> {
        if !self.config.auto_rollback {
            tracing::warn!("Auto-rollback disabled, manual intervention required for bundle {}", bundle_id);
            return Ok(FailureResolution::ManualIntervention);
        }
        
        let rollback_id = format!("rollback_{}", uuid::Uuid::new_v4());
        
        let rollback_op = RollbackOperation {
            id: rollback_id.clone(),
            bundle_id: bundle_id.to_string(),
            rollback_transactions: vec![], // Would be populated with actual rollback transactions
            created_at: Instant::now(),
            status: RollbackStatus::Pending,
        };
        
        {
            let mut rollbacks = self.rollbacks.write().await;
            rollbacks.insert(rollback_id.clone(), rollback_op);
        }
        
        // Execute rollback
        self.execute_rollback(&rollback_id).await?;
        
        // Update statistics
        let mut stats = self.stats.write().await;
        stats.rollbacks_executed += 1;
        
        Ok(FailureResolution::Rollback)
    }
    
    /// Execute rollback operation
    async fn execute_rollback(&self, rollback_id: &str) -> Result<()> {
        let mut rollbacks = self.rollbacks.write().await;
        let rollback = rollbacks.get_mut(rollback_id)
            .ok_or_else(|| anyhow!("Rollback operation not found: {}", rollback_id))?;
        
        rollback.status = RollbackStatus::InProgress;
        
        tracing::info!("Executing rollback operation: {}", rollback_id);
        
        // In a real implementation, this would:
        // 1. Identify transactions to rollback
        // 2. Create compensating transactions
        // 3. Submit rollback transactions
        // 4. Monitor rollback success
        
        // Simulate rollback execution
        tokio::time::sleep(Duration::from_millis(500)).await;
        
        rollback.status = RollbackStatus::Completed;
        
        tracing::info!("Rollback operation completed: {}", rollback_id);
        
        Ok(())
    }
    
    /// Abort operation
    async fn abort_operation(&self, failure_id: &str) -> Result<FailureResolution> {
        let mut failures = self.failures.write().await;
        let failure = failures.get_mut(failure_id)
            .ok_or_else(|| anyhow!("Failure record not found: {}", failure_id))?;
        
        failure.resolved = true;
        failure.resolution_method = Some("aborted".to_string());
        
        tracing::warn!("Operation aborted for failure: {}", failure_id);
        
        Ok(FailureResolution::Abort)
    }
    
    /// Activate kill switch
    pub async fn activate_kill_switch(&self, reason: &str, activated_by: Option<String>) -> Result<FailureResolution> {
        let mut kill_switch = self.kill_switch.lock().await;
        
        if kill_switch.activated {
            tracing::warn!("Kill switch already activated");
            return Ok(FailureResolution::KillSwitchActivated);
        }
        
        kill_switch.activated = true;
        kill_switch.reason = Some(reason.to_string());
        kill_switch.activated_at = Some(Instant::now());
        kill_switch.activated_by = activated_by;
        
        // Update statistics
        let mut stats = self.stats.write().await;
        stats.kill_switch_activations += 1;
        
        tracing::error!("🚨 KILL SWITCH ACTIVATED: {}", reason);
        
        // In a real implementation, this would:
        // 1. Stop all active operations
        // 2. Cancel pending transactions
        // 3. Notify operators
        // 4. Save state for recovery
        
        Ok(FailureResolution::KillSwitchActivated)
    }
    
    /// Deactivate kill switch
    pub async fn deactivate_kill_switch(&self, deactivated_by: String) -> Result<()> {
        let mut kill_switch = self.kill_switch.lock().await;
        
        if !kill_switch.activated {
            return Err(anyhow!("Kill switch is not activated"));
        }
        
        kill_switch.activated = false;
        kill_switch.reason = None;
        kill_switch.activated_at = None;
        kill_switch.activated_by = Some(deactivated_by);
        
        tracing::info!("Kill switch deactivated");
        
        Ok(())
    }
    
    /// Check if kill switch is activated
    pub async fn is_kill_switch_activated(&self) -> bool {
        let kill_switch = self.kill_switch.lock().await;
        kill_switch.activated
    }
    
    /// Update failure statistics
    async fn update_failure_stats(&self, failure_type: &FailureType) {
        let mut stats = self.stats.write().await;
        stats.total_failures += 1;
        
        let type_key = format!("{:?}", failure_type).split(' ').next().unwrap_or("Unknown").to_string();
        *stats.failures_by_type.entry(type_key).or_insert(0) += 1;
    }
    
    /// Get failure statistics
    pub async fn get_failure_stats(&self) -> FailureStats {
        let stats = self.stats.read().await;
        stats.clone()
    }
    
    /// Get active failures
    pub async fn get_active_failures(&self) -> Vec<FailureRecord> {
        let failures = self.failures.read().await;
        failures.values()
            .filter(|f| !f.resolved)
            .cloned()
            .collect()
    }
    
    /// Get rollback operations
    pub async fn get_rollback_operations(&self) -> Vec<RollbackOperation> {
        let rollbacks = self.rollbacks.read().await;
        rollbacks.values().cloned().collect()
    }
    
    /// Clean up old failure records
    pub async fn cleanup_old_records(&self, max_age: Duration) {
        let mut failures = self.failures.write().await;
        let mut rollbacks = self.rollbacks.write().await;
        
        let cutoff = chrono::Utc::now() - chrono::Duration::from_std(max_age).unwrap();
        
        failures.retain(|_, failure| failure.timestamp > cutoff);
        rollbacks.retain(|_, rollback| {
            rollback.created_at.elapsed() < max_age
        });
    }
}

/// Resolution strategy for failures
#[derive(Debug, Clone, PartialEq)]
pub enum FailureResolution {
    /// Retry the operation with delay and max attempts
    Retry { delay: Duration, max_attempts: u32 },
    /// Rollback the operation
    Rollback,
    /// Abort the operation
    Abort,
    /// Activate kill switch
    KillSwitch,
    /// Kill switch is already activated
    KillSwitchActivated,
    /// Manual intervention required
    ManualIntervention,
}

/// Security checklist for safe operations
#[derive(Debug, Clone)]
pub struct SecurityChecklist {
    pub items: Vec<SecurityCheckItem>,
}

/// Individual security check item
#[derive(Debug, Clone)]
pub struct SecurityCheckItem {
    pub id: String,
    pub description: String,
    pub category: SecurityCategory,
    pub severity: SecuritySeverity,
    pub checked: bool,
    pub notes: Option<String>,
}

/// Security check categories
#[derive(Debug, Clone, PartialEq)]
pub enum SecurityCategory {
    KeyManagement,
    NetworkSecurity,
    TransactionSafety,
    FundProtection,
    OperationalSecurity,
}

/// Security severity levels
#[derive(Debug, Clone, PartialEq)]
pub enum SecuritySeverity {
    Critical,
    High,
    Medium,
    Low,
}

impl SecurityChecklist {
    /// Create default security checklist
    pub fn default_checklist() -> Self {
        let items = vec![
            SecurityCheckItem {
                id: "key_encryption".to_string(),
                description: "Private keys are encrypted and stored securely".to_string(),
                category: SecurityCategory::KeyManagement,
                severity: SecuritySeverity::Critical,
                checked: false,
                notes: None,
            },
            SecurityCheckItem {
                id: "network_tls".to_string(),
                description: "All network connections use TLS encryption".to_string(),
                category: SecurityCategory::NetworkSecurity,
                severity: SecuritySeverity::High,
                checked: false,
                notes: None,
            },
            SecurityCheckItem {
                id: "gas_limits".to_string(),
                description: "Gas limits are set to prevent excessive costs".to_string(),
                category: SecurityCategory::TransactionSafety,
                severity: SecuritySeverity::High,
                checked: false,
                notes: None,
            },
            SecurityCheckItem {
                id: "fund_limits".to_string(),
                description: "Maximum fund exposure limits are configured".to_string(),
                category: SecurityCategory::FundProtection,
                severity: SecuritySeverity::Critical,
                checked: false,
                notes: None,
            },
            SecurityCheckItem {
                id: "kill_switch".to_string(),
                description: "Kill switch is functional and tested".to_string(),
                category: SecurityCategory::OperationalSecurity,
                severity: SecuritySeverity::Critical,
                checked: false,
                notes: None,
            },
        ];
        
        Self { items }
    }
    
    /// Check if all critical items are completed
    pub fn all_critical_checked(&self) -> bool {
        self.items.iter()
            .filter(|item| item.severity == SecuritySeverity::Critical)
            .all(|item| item.checked)
    }
    
    /// Get unchecked items
    pub fn get_unchecked_items(&self) -> Vec<&SecurityCheckItem> {
        self.items.iter()
            .filter(|item| !item.checked)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_failure_handler_creation() {
        let config = FailureConfig::default();
        let handler = FailureHandler::new(config);
        
        assert!(!handler.is_kill_switch_activated().await);
        
        let stats = handler.get_failure_stats().await;
        assert_eq!(stats.total_failures, 0);
    }
    
    #[tokio::test]
    async fn test_failure_handling() {
        let config = FailureConfig::default();
        let handler = FailureHandler::new(config);
        
        let failure_type = FailureType::NonceCollision { expected: 1, actual: 2 };
        let metadata = HashMap::new();
        
        let resolution = handler.handle_failure("test_bundle", failure_type, metadata).await.unwrap();
        
        assert!(matches!(resolution, FailureResolution::Retry { .. }));
        
        let stats = handler.get_failure_stats().await;
        assert_eq!(stats.total_failures, 1);
    }
    
    #[tokio::test]
    async fn test_kill_switch() {
        let config = FailureConfig::default();
        let handler = FailureHandler::new(config);
        
        // Activate kill switch
        let resolution = handler.activate_kill_switch("Test activation", Some("test_user".to_string())).await.unwrap();
        assert_eq!(resolution, FailureResolution::KillSwitchActivated);
        
        // Check activation
        assert!(handler.is_kill_switch_activated().await);
        
        // Deactivate
        handler.deactivate_kill_switch("test_user".to_string()).await.unwrap();
        assert!(!handler.is_kill_switch_activated().await);
    }
    
    #[tokio::test]
    async fn test_rollback_operation() {
        let config = FailureConfig::default();
        let handler = FailureHandler::new(config);
        
        let resolution = handler.initiate_rollback("test_bundle").await.unwrap();
        assert_eq!(resolution, FailureResolution::Rollback);
        
        let rollbacks = handler.get_rollback_operations().await;
        assert_eq!(rollbacks.len(), 1);
        assert_eq!(rollbacks[0].status, RollbackStatus::Completed);
    }
    
    #[test]
    fn test_security_checklist() {
        let checklist = SecurityChecklist::default_checklist();
        
        assert!(!checklist.all_critical_checked());
        
        let unchecked = checklist.get_unchecked_items();
        assert_eq!(unchecked.len(), 5);
        
        // Check critical items count
        let critical_count = checklist.items.iter()
            .filter(|item| item.severity == SecuritySeverity::Critical)
            .count();
        assert_eq!(critical_count, 3);
    }
}