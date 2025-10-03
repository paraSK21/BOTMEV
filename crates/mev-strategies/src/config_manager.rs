//! Configuration manager for MEV strategies with YAML support

use crate::strategy_types::*;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::Path,
};
use tracing::{info, warn};

/// Configuration file structure for multiple strategies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfigFile {
    pub version: String,
    pub global: GlobalConfig,
    pub strategies: Vec<StrategyConfig>,
}

/// Global configuration settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    pub enabled: bool,
    pub max_concurrent_strategies: usize,
    pub evaluation_timeout_ms: u64,
    pub default_min_profit_wei: u128,
    pub default_max_gas_price_gwei: u64,
    pub default_risk_tolerance: f64,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_concurrent_strategies: 5,
            evaluation_timeout_ms: 50,
            default_min_profit_wei: 1_000_000_000_000_000, // 0.001 ETH
            default_max_gas_price_gwei: 100,
            default_risk_tolerance: 0.5,
        }
    }
}

/// Configuration manager for loading and managing strategy configurations
pub struct ConfigManager {
    config_file: StrategyConfigFile,
    config_path: String,
}

impl ConfigManager {
    /// Create new configuration manager
    pub fn new(config_path: String) -> Self {
        Self {
            config_file: StrategyConfigFile::default(),
            config_path,
        }
    }

    /// Load configuration from YAML file
    pub fn load_from_file(&mut self) -> Result<()> {
        if !Path::new(&self.config_path).exists() {
            warn!(
                config_path = %self.config_path,
                "Configuration file not found, creating default"
            );
            self.create_default_config()?;
            return Ok(());
        }

        let yaml_content = fs::read_to_string(&self.config_path)?;
        self.config_file = serde_yaml::from_str(&yaml_content)?;

        info!(
            config_path = %self.config_path,
            strategy_count = self.config_file.strategies.len(),
            "Configuration loaded from file"
        );

        Ok(())
    }

    /// Save configuration to YAML file
    pub fn save_to_file(&self) -> Result<()> {
        let yaml_content = serde_yaml::to_string(&self.config_file)?;
        fs::write(&self.config_path, yaml_content)?;

        info!(
            config_path = %self.config_path,
            "Configuration saved to file"
        );

        Ok(())
    }

    /// Create default configuration file
    fn create_default_config(&mut self) -> Result<()> {
        self.config_file = StrategyConfigFile::default();
        self.save_to_file()?;
        Ok(())
    }

    /// Get configuration for a specific strategy
    pub fn get_strategy_config(&self, strategy_name: &str) -> Option<&StrategyConfig> {
        self.config_file
            .strategies
            .iter()
            .find(|config| config.name == strategy_name)
    }

    /// Update configuration for a specific strategy
    pub fn update_strategy_config(&mut self, config: StrategyConfig) -> Result<()> {
        if let Some(existing_config) = self.config_file
            .strategies
            .iter_mut()
            .find(|c| c.name == config.name)
        {
            *existing_config = config;
        } else {
            self.config_file.strategies.push(config);
        }

        info!(
            strategy_name = %self.config_file.strategies.last().unwrap().name,
            "Strategy configuration updated"
        );

        Ok(())
    }

    /// Enable or disable a strategy
    pub fn set_strategy_enabled(&mut self, strategy_name: &str, enabled: bool) -> Result<()> {
        if let Some(config) = self.config_file
            .strategies
            .iter_mut()
            .find(|c| c.name == strategy_name)
        {
            config.enabled = enabled;
            info!(
                strategy_name = %strategy_name,
                enabled = enabled,
                "Strategy enabled/disabled"
            );
            Ok(())
        } else {
            Err(anyhow::anyhow!("Strategy '{}' not found", strategy_name))
        }
    }

    /// Get all enabled strategies
    pub fn get_enabled_strategies(&self) -> Vec<&StrategyConfig> {
        self.config_file
            .strategies
            .iter()
            .filter(|config| config.enabled && self.config_file.global.enabled)
            .collect()
    }

    /// Get all strategy configurations
    pub fn get_all_strategies(&self) -> &[StrategyConfig] {
        &self.config_file.strategies
    }

    /// Get global configuration
    pub fn get_global_config(&self) -> &GlobalConfig {
        &self.config_file.global
    }

    /// Update global configuration
    pub fn update_global_config(&mut self, global_config: GlobalConfig) {
        self.config_file.global = global_config;
        info!("Global configuration updated");
    }

    /// Validate configuration
    pub fn validate_config(&self) -> Result<Vec<String>> {
        let mut warnings = Vec::new();

        // Validate global config
        if self.config_file.global.max_concurrent_strategies == 0 {
            warnings.push("max_concurrent_strategies should be greater than 0".to_string());
        }

        if self.config_file.global.evaluation_timeout_ms < 10 {
            warnings.push("evaluation_timeout_ms should be at least 10ms".to_string());
        }

        // Validate strategy configs
        for config in &self.config_file.strategies {
            if config.min_profit_wei == 0 {
                warnings.push(format!(
                    "Strategy '{}' has min_profit_wei of 0, which may cause excessive gas usage",
                    config.name
                ));
            }

            if config.max_gas_price_gwei > 1000 {
                warnings.push(format!(
                    "Strategy '{}' has very high max_gas_price_gwei ({}), consider lowering",
                    config.name, config.max_gas_price_gwei
                ));
            }

            if config.risk_tolerance > 1.0 || config.risk_tolerance < 0.0 {
                warnings.push(format!(
                    "Strategy '{}' has invalid risk_tolerance ({}), should be between 0.0 and 1.0",
                    config.name, config.risk_tolerance
                ));
            }
        }

        // Check for duplicate strategy names
        let mut strategy_names = HashMap::new();
        for config in &self.config_file.strategies {
            if let Some(existing_priority) = strategy_names.insert(&config.name, config.priority) {
                warnings.push(format!(
                    "Duplicate strategy name '{}' found with priorities {} and {}",
                    config.name, existing_priority, config.priority
                ));
            }
        }

        Ok(warnings)
    }

    /// Hot reload configuration from file
    pub fn hot_reload(&mut self) -> Result<bool> {
        let current_modified = fs::metadata(&self.config_path)?.modified()?;
        
        // For simplicity, always reload. In production, you'd track modification time
        let old_config = self.config_file.clone();
        self.load_from_file()?;
        
        let config_changed = !self.configs_equal(&old_config, &self.config_file);
        
        if config_changed {
            info!("Configuration hot reloaded with changes");
        }
        
        Ok(config_changed)
    }

    /// Compare two configurations for equality
    fn configs_equal(&self, a: &StrategyConfigFile, b: &StrategyConfigFile) -> bool {
        // Simple comparison - in production you might want more sophisticated comparison
        serde_yaml::to_string(a).unwrap_or_default() == serde_yaml::to_string(b).unwrap_or_default()
    }

    /// Export configuration as JSON for API responses
    pub fn export_as_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.config_file)?)
    }

    /// Import configuration from JSON
    pub fn import_from_json(&mut self, json_str: &str) -> Result<()> {
        self.config_file = serde_json::from_str(json_str)?;
        info!("Configuration imported from JSON");
        Ok(())
    }
}

impl Default for StrategyConfigFile {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            global: GlobalConfig::default(),
            strategies: vec![
                // Default backrun strategy configuration
                StrategyConfig {
                    name: "backrun".to_string(),
                    enabled: true,
                    priority: 150,
                    min_profit_wei: 5_000_000_000_000_000, // 0.005 ETH
                    max_gas_price_gwei: 150,
                    risk_tolerance: 0.7,
                    parameters: {
                        let mut params = HashMap::new();
                        params.insert("min_target_value_eth".to_string(), serde_json::json!(0.5));
                        params.insert("max_slippage_percent".to_string(), serde_json::json!(2.0));
                        params.insert("target_protocols".to_string(), serde_json::json!(["UniswapV2", "UniswapV3", "SushiSwap"]));
                        params
                    },
                },
                // Default sandwich strategy configuration
                StrategyConfig {
                    name: "sandwich".to_string(),
                    enabled: false, // Disabled by default due to higher risk
                    priority: 200,
                    min_profit_wei: 10_000_000_000_000_000, // 0.01 ETH
                    max_gas_price_gwei: 200,
                    risk_tolerance: 0.5,
                    parameters: {
                        let mut params = HashMap::new();
                        params.insert("max_slippage_tolerance".to_string(), serde_json::json!(5.0));
                        params.insert("min_victim_value_eth".to_string(), serde_json::json!(1.0));
                        params.insert("max_front_run_multiple".to_string(), serde_json::json!(3.0));
                        params.insert("gas_price_premium_percent".to_string(), serde_json::json!(10.0));
                        params
                    },
                },
                // Default arbitrage strategy configuration
                StrategyConfig {
                    name: "arbitrage".to_string(),
                    enabled: true,
                    priority: 100,
                    min_profit_wei: 2_000_000_000_000_000, // 0.002 ETH
                    max_gas_price_gwei: 120,
                    risk_tolerance: 0.8,
                    parameters: {
                        let mut params = HashMap::new();
                        params.insert("max_price_difference_percent".to_string(), serde_json::json!(1.0));
                        params.insert("supported_dexes".to_string(), serde_json::json!(["UniswapV2", "SushiSwap", "Curve"]));
                        params.insert("min_liquidity_usd".to_string(), serde_json::json!(10000.0));
                        params
                    },
                },
            ],
        }
    }
}

/// Configuration watcher for automatic reloading
pub struct ConfigWatcher {
    config_manager: ConfigManager,
    watch_interval_seconds: u64,
}

impl ConfigWatcher {
    pub fn new(config_manager: ConfigManager, watch_interval_seconds: u64) -> Self {
        Self {
            config_manager,
            watch_interval_seconds,
        }
    }

    /// Start watching for configuration changes
    pub async fn start_watching<F>(&mut self, mut on_change: F) -> Result<()>
    where
        F: FnMut(&StrategyConfigFile) + Send + 'static,
    {
        let mut interval = tokio::time::interval(
            tokio::time::Duration::from_secs(self.watch_interval_seconds)
        );

        loop {
            interval.tick().await;
            
            match self.config_manager.hot_reload() {
                Ok(true) => {
                    info!("Configuration changed, notifying listeners");
                    on_change(&self.config_manager.config_file);
                }
                Ok(false) => {
                    // No changes
                }
                Err(e) => {
                    warn!("Failed to reload configuration: {}", e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config_creation() {
        let config_file = StrategyConfigFile::default();
        
        assert_eq!(config_file.version, "1.0");
        assert!(config_file.global.enabled);
        assert_eq!(config_file.strategies.len(), 3);
        
        // Check that backrun is enabled by default
        let backrun_config = config_file.strategies.iter().find(|s| s.name == "backrun").unwrap();
        assert!(backrun_config.enabled);
        
        // Check that sandwich is disabled by default
        let sandwich_config = config_file.strategies.iter().find(|s| s.name == "sandwich").unwrap();
        assert!(!sandwich_config.enabled);
    }

    #[test]
    fn test_config_serialization() {
        let config_file = StrategyConfigFile::default();
        
        // Test YAML serialization
        let yaml_str = serde_yaml::to_string(&config_file).unwrap();
        assert!(yaml_str.contains("backrun"));
        assert!(yaml_str.contains("sandwich"));
        assert!(yaml_str.contains("arbitrage"));
        
        // Test deserialization
        let deserialized: StrategyConfigFile = serde_yaml::from_str(&yaml_str).unwrap();
        assert_eq!(deserialized.strategies.len(), config_file.strategies.len());
    }

    #[test]
    fn test_config_manager_operations() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_path = temp_file.path().to_string_lossy().to_string();
        
        let mut manager = ConfigManager::new(config_path);
        
        // Test loading (should create default)
        assert!(manager.load_from_file().is_ok());
        
        // Test getting strategy config
        let backrun_config = manager.get_strategy_config("backrun");
        assert!(backrun_config.is_some());
        assert_eq!(backrun_config.unwrap().name, "backrun");
        
        // Test enabling/disabling strategy
        assert!(manager.set_strategy_enabled("sandwich", true).is_ok());
        let sandwich_config = manager.get_strategy_config("sandwich").unwrap();
        assert!(sandwich_config.enabled);
        
        // Test getting enabled strategies
        let enabled = manager.get_enabled_strategies();
        assert!(enabled.len() >= 2); // backrun and sandwich should be enabled
    }

    #[test]
    fn test_config_validation() {
        let mut config_file = StrategyConfigFile::default();
        
        // Add invalid configuration
        config_file.strategies[0].risk_tolerance = 1.5; // Invalid: > 1.0
        config_file.strategies[0].max_gas_price_gwei = 2000; // Very high
        
        let manager = ConfigManager {
            config_file,
            config_path: "test".to_string(),
        };
        
        let warnings = manager.validate_config().unwrap();
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.contains("risk_tolerance")));
        assert!(warnings.iter().any(|w| w.contains("max_gas_price_gwei")));
    }

    #[test]
    fn test_json_export_import() {
        let config_file = StrategyConfigFile::default();
        let manager = ConfigManager {
            config_file,
            config_path: "test".to_string(),
        };
        
        // Test export
        let json_str = manager.export_as_json().unwrap();
        assert!(json_str.contains("backrun"));
        
        // Test import
        let mut new_manager = ConfigManager::new("test2".to_string());
        assert!(new_manager.import_from_json(&json_str).is_ok());
        assert_eq!(new_manager.config_file.strategies.len(), 3);
    }
}