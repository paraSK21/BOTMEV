//! Filter configuration loader and manager

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use tracing::{info, warn};

use crate::filters::{FilterAction, FilterCondition, FilterRule};

/// Complete filter configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterConfig {
    pub rules: Vec<FilterRuleConfig>,
    pub settings: FilterSettings,
}

/// Filter rule configuration (matches YAML structure)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterRuleConfig {
    pub name: String,
    pub enabled: bool,
    pub priority: i32,
    pub description: String,
    pub conditions: Vec<FilterConditionConfig>,
    pub action: FilterActionConfig,
}

/// Filter condition configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FilterConditionConfig {
    ValueRange {
        min_wei: Option<String>,
        max_wei: Option<String>,
    },
    GasPriceRange {
        min_gwei: Option<u64>,
        max_gwei: Option<u64>,
    },
    TargetType {
        types: Vec<String>,
    },
    FunctionSignature {
        signatures: Vec<String>,
    },
    ContractAddress {
        addresses: Vec<String>,
        whitelist: bool,
    },
    SenderAddress {
        addresses: Vec<String>,
        whitelist: bool,
    },
    DataSize {
        min_bytes: Option<usize>,
        max_bytes: Option<usize>,
    },
    TimeOfDay {
        start_hour: u8,
        end_hour: u8,
    },
    DataPattern {
        pattern: String,
        case_sensitive: bool,
    },
}

/// Filter action configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FilterActionConfig {
    Accept,
    Reject,
    Tag { tags: Vec<String> },
    SetPriority { level: u8 },
    RateLimit { max_per_second: u32 },
}

/// Global filter settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterSettings {
    pub max_rules_per_transaction: usize,
    pub detailed_logging: bool,
    pub default_priority: u8,
    pub enable_metrics: bool,
    pub filter_timeout_ms: u64,
}

impl Default for FilterSettings {
    fn default() -> Self {
        Self {
            max_rules_per_transaction: 20,
            detailed_logging: false,
            default_priority: 128,
            enable_metrics: true,
            filter_timeout_ms: 10,
        }
    }
}

/// Filter configuration manager
pub struct FilterConfigManager {
    config_path: String,
    current_config: Option<FilterConfig>,
}

impl FilterConfigManager {
    pub fn new<P: AsRef<Path>>(config_path: P) -> Self {
        Self {
            config_path: config_path.as_ref().to_string_lossy().to_string(),
            current_config: None,
        }
    }

    /// Load filter configuration from YAML file
    pub fn load_config(&mut self) -> Result<FilterConfig> {
        info!(config_path = %self.config_path, "Loading filter configuration");

        let config_content = fs::read_to_string(&self.config_path)?;
        let config: FilterConfig = serde_yaml::from_str(&config_content)?;

        // Validate configuration
        self.validate_config(&config)?;

        self.current_config = Some(config.clone());
        
        info!(
            rules_count = config.rules.len(),
            enabled_rules = config.rules.iter().filter(|r| r.enabled).count(),
            "Filter configuration loaded successfully"
        );

        Ok(config)
    }

    /// Save current configuration to file
    pub fn save_config(&self, config: &FilterConfig) -> Result<()> {
        let config_yaml = serde_yaml::to_string(config)?;
        fs::write(&self.config_path, config_yaml)?;
        
        info!(config_path = %self.config_path, "Filter configuration saved");
        Ok(())
    }

    /// Convert configuration to filter rules
    pub fn to_filter_rules(&self, config: &FilterConfig) -> Result<Vec<FilterRule>> {
        let mut rules = Vec::new();

        for rule_config in &config.rules {
            let rule = self.convert_rule_config(rule_config)?;
            rules.push(rule);
        }

        Ok(rules)
    }

    /// Convert a single rule configuration
    fn convert_rule_config(&self, config: &FilterRuleConfig) -> Result<FilterRule> {
        let conditions = config
            .conditions
            .iter()
            .map(|c| self.convert_condition_config(c))
            .collect::<Result<Vec<_>>>()?;

        let action = self.convert_action_config(&config.action)?;

        Ok(FilterRule {
            name: config.name.clone(),
            enabled: config.enabled,
            priority: config.priority,
            conditions,
            action,
            description: config.description.clone(),
        })
    }

    /// Convert condition configuration
    fn convert_condition_config(&self, config: &FilterConditionConfig) -> Result<FilterCondition> {
        let condition = match config {
            FilterConditionConfig::ValueRange { min_wei, max_wei } => {
                FilterCondition::ValueRange {
                    min_wei: min_wei.clone(),
                    max_wei: max_wei.clone(),
                }
            }
            FilterConditionConfig::GasPriceRange { min_gwei, max_gwei } => {
                FilterCondition::GasPriceRange {
                    min_gwei: *min_gwei,
                    max_gwei: *max_gwei,
                }
            }
            FilterConditionConfig::TargetType { types } => {
                FilterCondition::TargetType {
                    types: types.clone(),
                }
            }
            FilterConditionConfig::FunctionSignature { signatures } => {
                FilterCondition::FunctionSignature {
                    signatures: signatures.clone(),
                }
            }
            FilterConditionConfig::ContractAddress { addresses, whitelist } => {
                FilterCondition::ContractAddress {
                    addresses: addresses.clone(),
                    whitelist: *whitelist,
                }
            }
            FilterConditionConfig::SenderAddress { addresses, whitelist } => {
                FilterCondition::SenderAddress {
                    addresses: addresses.clone(),
                    whitelist: *whitelist,
                }
            }
            FilterConditionConfig::DataSize { min_bytes, max_bytes } => {
                FilterCondition::DataSize {
                    min_bytes: *min_bytes,
                    max_bytes: *max_bytes,
                }
            }
            FilterConditionConfig::TimeOfDay { start_hour, end_hour } => {
                FilterCondition::TimeOfDay {
                    start_hour: *start_hour,
                    end_hour: *end_hour,
                }
            }
            FilterConditionConfig::DataPattern { pattern, case_sensitive } => {
                FilterCondition::DataPattern {
                    pattern: pattern.clone(),
                    case_sensitive: *case_sensitive,
                }
            }
        };

        Ok(condition)
    }

    /// Convert action configuration
    fn convert_action_config(&self, config: &FilterActionConfig) -> Result<FilterAction> {
        let action = match config {
            FilterActionConfig::Accept => FilterAction::Accept,
            FilterActionConfig::Reject => FilterAction::Reject,
            FilterActionConfig::Tag { tags } => FilterAction::Tag {
                tags: tags.clone(),
            },
            FilterActionConfig::SetPriority { level } => FilterAction::SetPriority {
                level: *level,
            },
            FilterActionConfig::RateLimit { max_per_second } => FilterAction::RateLimit {
                max_per_second: *max_per_second,
            },
        };

        Ok(action)
    }

    /// Validate configuration
    fn validate_config(&self, config: &FilterConfig) -> Result<()> {
        // Check for duplicate rule names
        let mut rule_names = std::collections::HashSet::new();
        for rule in &config.rules {
            if !rule_names.insert(&rule.name) {
                return Err(anyhow::anyhow!("Duplicate rule name: {}", rule.name));
            }
        }

        // Validate rule priorities
        for rule in &config.rules {
            if rule.priority < 0 || rule.priority > 1000 {
                warn!(
                    rule = %rule.name,
                    priority = rule.priority,
                    "Rule priority outside recommended range (0-1000)"
                );
            }
        }

        // Validate time conditions
        for rule in &config.rules {
            for condition in &rule.conditions {
                if let FilterConditionConfig::TimeOfDay { start_hour, end_hour } = condition {
                    if *start_hour > 23 || *end_hour > 23 {
                        return Err(anyhow::anyhow!(
                            "Invalid time range in rule '{}': hours must be 0-23",
                            rule.name
                        ));
                    }
                }
            }
        }

        // Validate settings
        if config.settings.max_rules_per_transaction == 0 {
            return Err(anyhow::anyhow!("max_rules_per_transaction must be > 0"));
        }

        if config.settings.filter_timeout_ms == 0 {
            return Err(anyhow::anyhow!("filter_timeout_ms must be > 0"));
        }

        info!("Filter configuration validation passed");
        Ok(())
    }

    /// Get current configuration
    pub fn get_current_config(&self) -> Option<&FilterConfig> {
        self.current_config.as_ref()
    }

    /// Update a specific rule
    pub fn update_rule(&mut self, rule_name: &str, enabled: bool) -> Result<()> {
        if let Some(config) = &mut self.current_config {
            if let Some(rule) = config.rules.iter_mut().find(|r| r.name == rule_name) {
                rule.enabled = enabled;
                info!(rule = rule_name, enabled = enabled, "Rule updated");
                return Ok(());
            }
        }
        
        Err(anyhow::anyhow!("Rule '{}' not found", rule_name))
    }

    /// Add a new rule
    pub fn add_rule(&mut self, rule: FilterRuleConfig) -> Result<()> {
        if let Some(config) = &mut self.current_config {
            // Check for duplicate name
            if config.rules.iter().any(|r| r.name == rule.name) {
                return Err(anyhow::anyhow!("Rule '{}' already exists", rule.name));
            }

            config.rules.push(rule.clone());
            info!(rule = %rule.name, "Rule added");
            return Ok(());
        }

        Err(anyhow::anyhow!("No configuration loaded"))
    }

    /// Remove a rule
    pub fn remove_rule(&mut self, rule_name: &str) -> Result<()> {
        if let Some(config) = &mut self.current_config {
            let initial_len = config.rules.len();
            config.rules.retain(|r| r.name != rule_name);
            
            if config.rules.len() < initial_len {
                info!(rule = rule_name, "Rule removed");
                return Ok(());
            }
        }

        Err(anyhow::anyhow!("Rule '{}' not found", rule_name))
    }

    /// Get rule statistics
    pub fn get_rule_stats(&self) -> Option<FilterRuleStats> {
        if let Some(config) = &self.current_config {
            let total_rules = config.rules.len();
            let enabled_rules = config.rules.iter().filter(|r| r.enabled).count();
            let disabled_rules = total_rules - enabled_rules;

            let mut priority_distribution = std::collections::HashMap::new();
            for rule in &config.rules {
                let priority_range = match rule.priority {
                    0..=50 => "Low (0-50)",
                    51..=100 => "Medium (51-100)",
                    101..=200 => "High (101-200)",
                    _ => "Very High (200+)",
                };
                *priority_distribution.entry(priority_range).or_insert(0) += 1;
            }

            Some(FilterRuleStats {
                total_rules,
                enabled_rules,
                disabled_rules,
                priority_distribution,
            })
        } else {
            None
        }
    }
}

/// Filter rule statistics
#[derive(Debug, Clone)]
pub struct FilterRuleStats {
    pub total_rules: usize,
    pub enabled_rules: usize,
    pub disabled_rules: usize,
    pub priority_distribution: std::collections::HashMap<&'static str, usize>,
}

impl FilterRuleStats {
    pub fn print_summary(&self) {
        info!(
            total_rules = self.total_rules,
            enabled_rules = self.enabled_rules,
            disabled_rules = self.disabled_rules,
            "Filter rule statistics"
        );

        for (priority_range, count) in &self.priority_distribution {
            info!(priority_range = priority_range, count = count, "Priority distribution");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[test]
    fn test_config_loading() {
        let config_yaml = r#"
rules:
  - name: "test_rule"
    enabled: true
    priority: 100
    description: "Test rule"
    conditions:
      - type: "ValueRange"
        min_wei: "1000000000000000000"
    action:
      type: "Accept"

settings:
  max_rules_per_transaction: 10
  detailed_logging: false
  default_priority: 128
  enable_metrics: true
  filter_timeout_ms: 5
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(config_yaml.as_bytes()).unwrap();

        let mut manager = FilterConfigManager::new(temp_file.path());
        let config = manager.load_config().unwrap();

        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].name, "test_rule");
        assert_eq!(config.settings.max_rules_per_transaction, 10);
    }

    #[test]
    fn test_rule_conversion() {
        let mut manager = FilterConfigManager::new("dummy_path");
        
        let rule_config = FilterRuleConfig {
            name: "test".to_string(),
            enabled: true,
            priority: 100,
            description: "Test".to_string(),
            conditions: vec![FilterConditionConfig::ValueRange {
                min_wei: Some("1000000000000000000".to_string()),
                max_wei: None,
            }],
            action: FilterActionConfig::Accept,
        };

        let rule = manager.convert_rule_config(&rule_config).unwrap();
        assert_eq!(rule.name, "test");
        assert_eq!(rule.priority, 100);
        assert!(matches!(rule.action, FilterAction::Accept));
    }
}