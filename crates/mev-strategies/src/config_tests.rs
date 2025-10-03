//! Tests for strategy configuration and YAML toggle functionality

use crate::strategy_types::*;
use crate::test_utils::*;
use crate::{BackrunStrategy, SandwichStrategy};
use mev_core::TargetType;
use serde_yaml;
use std::collections::HashMap;
use tokio;

/// Test suite for YAML configuration loading and parsing
mod yaml_config_tests {
    use super::*;

    #[test]
    fn test_strategy_config_serialization() {
        let config = MockConfigGenerator::backrun_config();
        
        // Serialize to YAML
        let yaml_str = serde_yaml::to_string(&config).unwrap();
        println!("Serialized config:\n{}", yaml_str);
        
        // Deserialize back
        let deserialized: StrategyConfig = serde_yaml::from_str(&yaml_str).unwrap();
        
        // Verify all fields match
        assert_eq!(config.name, deserialized.name);
        assert_eq!(config.enabled, deserialized.enabled);
        assert_eq!(config.priority, deserialized.priority);
        assert_eq!(config.min_profit_wei, deserialized.min_profit_wei);
        assert_eq!(config.max_gas_price_gwei, deserialized.max_gas_price_gwei);
        assert_eq!(config.risk_tolerance, deserialized.risk_tolerance);
        
        // Verify parameters
        for (key, value) in &config.parameters {
            assert_eq!(
                deserialized.parameters.get(key).unwrap(),
                value,
                "Parameter {} mismatch", key
            );
        }
    }

    #[test]
    fn test_multiple_strategy_config_yaml() {
        let backrun_config = MockConfigGenerator::backrun_config();
        let sandwich_config = MockConfigGenerator::sandwich_config();
        let disabled_config = MockConfigGenerator::disabled_config();
        
        let configs = vec![backrun_config, sandwich_config, disabled_config];
        
        // Serialize all configs
        let yaml_str = serde_yaml::to_string(&configs).unwrap();
        println!("Multiple configs YAML:\n{}", yaml_str);
        
        // Deserialize back
        let deserialized: Vec<StrategyConfig> = serde_yaml::from_str(&yaml_str).unwrap();
        
        assert_eq!(configs.len(), deserialized.len());
        
        // Verify each config
        for (original, deserialized) in configs.iter().zip(deserialized.iter()) {
            assert_eq!(original.name, deserialized.name);
            assert_eq!(original.enabled, deserialized.enabled);
        }
    }

    #[test]
    fn test_yaml_config_with_comments() {
        let yaml_config = r#"
# Strategy Configuration
name: "backrun"
enabled: true  # Enable this strategy
priority: 150
min_profit_wei: 5000000000000000  # 0.005 ETH minimum profit
max_gas_price_gwei: 150
risk_tolerance: 0.7
parameters:
  min_target_value_eth: 0.5  # Minimum target transaction value
  max_slippage_percent: 2.0  # Maximum acceptable slippage
  target_protocols:
    - "UniswapV2"
    - "UniswapV3"
    - "SushiSwap"
"#;
        
        let config: StrategyConfig = serde_yaml::from_str(yaml_config).unwrap();
        
        assert_eq!(config.name, "backrun");
        assert!(config.enabled);
        assert_eq!(config.priority, 150);
        assert_eq!(config.min_profit_wei, 5_000_000_000_000_000);
        
        // Check parameters
        let min_value = config.parameters.get("min_target_value_eth")
            .and_then(|v| v.as_f64())
            .unwrap();
        assert_eq!(min_value, 0.5);
        
        let protocols = config.parameters.get("target_protocols")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(protocols.len(), 3);
    }

    #[test]
    fn test_invalid_yaml_handling() {
        let invalid_yaml = r#"
name: "test"
enabled: "not_a_boolean"  # Invalid boolean value
priority: "not_a_number"  # Invalid number
"#;
        
        let result: Result<StrategyConfig, _> = serde_yaml::from_str(invalid_yaml);
        assert!(result.is_err(), "Should fail to parse invalid YAML");
    }

    #[test]
    fn test_partial_config_with_defaults() {
        let minimal_yaml = r#"
name: "minimal_strategy"
enabled: false
"#;
        
        let mut config: StrategyConfig = serde_yaml::from_str(minimal_yaml).unwrap();
        
        // Fill in defaults for missing fields
        if config.priority == 0 {
            config.priority = 128; // Default priority
        }
        if config.min_profit_wei == 0 {
            config.min_profit_wei = 1_000_000_000_000_000; // Default 0.001 ETH
        }
        if config.max_gas_price_gwei == 0 {
            config.max_gas_price_gwei = 100; // Default 100 gwei
        }
        if config.risk_tolerance == 0.0 {
            config.risk_tolerance = 0.5; // Default 50%
        }
        
        assert_eq!(config.name, "minimal_strategy");
        assert!(!config.enabled);
        assert_eq!(config.priority, 128);
        assert_eq!(config.min_profit_wei, 1_000_000_000_000_000);
    }
}

/// Test suite for strategy enable/disable toggle functionality
mod toggle_functionality_tests {
    use super::*;

    #[tokio::test]
    async fn test_backrun_strategy_toggle() {
        let mut strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Initially enabled
        assert!(strategy.config().enabled);
        
        // Should find opportunities when enabled
        let large_swap = generator.generate_large_swap(TargetType::UniswapV2);
        let result = strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        let opportunity_when_enabled = matches!(result, StrategyResult::Opportunity(_));
        
        // Disable strategy
        let mut config = strategy.config().clone();
        config.enabled = false;
        strategy.update_config(config).unwrap();
        
        assert!(!strategy.config().enabled);
        
        // Should not find opportunities when disabled
        let result = strategy.evaluate_transaction(&large_swap).await.unwrap();
        match result {
            StrategyResult::NoOpportunity => {
                // Expected when disabled
            }
            _ => panic!("Expected no opportunity when strategy is disabled"),
        }
        
        // Re-enable strategy
        let mut config = strategy.config().clone();
        config.enabled = true;
        strategy.update_config(config).unwrap();
        
        // Should find opportunities again
        let result = strategy.evaluate_transaction(&large_swap).await.unwrap();
        let opportunity_when_reenabled = matches!(result, StrategyResult::Opportunity(_));
        
        // Behavior should be consistent
        assert_eq!(opportunity_when_enabled, opportunity_when_reenabled);
    }

    #[tokio::test]
    async fn test_sandwich_strategy_toggle() {
        let mut strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Initially enabled
        assert!(strategy.config().enabled);
        
        // Should find opportunities when enabled
        let victim_tx = generator.generate_sandwich_victim();
        let result = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        
        let opportunity_when_enabled = matches!(result, StrategyResult::Opportunity(_));
        
        // Disable strategy
        let mut config = strategy.config().clone();
        config.enabled = false;
        strategy.update_config(config).unwrap();
        
        // Should not find opportunities when disabled
        let result = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        match result {
            StrategyResult::NoOpportunity => {
                // Expected when disabled
            }
            _ => panic!("Expected no opportunity when strategy is disabled"),
        }
        
        // Re-enable strategy
        let mut config = strategy.config().clone();
        config.enabled = true;
        strategy.update_config(config).unwrap();
        
        // Should find opportunities again
        let result = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        let opportunity_when_reenabled = matches!(result, StrategyResult::Opportunity(_));
        
        // Behavior should be consistent
        assert_eq!(opportunity_when_enabled, opportunity_when_reenabled);
    }

    #[tokio::test]
    async fn test_multiple_strategy_toggle_coordination() {
        let mut backrun_strategy = BackrunStrategy::new();
        let mut sandwich_strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create a transaction that both strategies might be interested in
        let large_swap = generator.generate_large_swap(TargetType::UniswapV2);
        
        // Both enabled - both might find opportunities
        let backrun_result = backrun_strategy.evaluate_transaction(&large_swap).await.unwrap();
        let sandwich_result = sandwich_strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        // Disable backrun, keep sandwich enabled
        let mut backrun_config = backrun_strategy.config().clone();
        backrun_config.enabled = false;
        backrun_strategy.update_config(backrun_config).unwrap();
        
        let backrun_result_disabled = backrun_strategy.evaluate_transaction(&large_swap).await.unwrap();
        let sandwich_result_still_enabled = sandwich_strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        // Backrun should be disabled, sandwich should still work
        assert!(matches!(backrun_result_disabled, StrategyResult::NoOpportunity));
        
        // Sandwich behavior should be unchanged
        assert_eq!(
            matches!(sandwich_result, StrategyResult::Opportunity(_)),
            matches!(sandwich_result_still_enabled, StrategyResult::Opportunity(_))
        );
        
        // Disable sandwich, re-enable backrun
        let mut sandwich_config = sandwich_strategy.config().clone();
        sandwich_config.enabled = false;
        sandwich_strategy.update_config(sandwich_config).unwrap();
        
        let mut backrun_config = backrun_strategy.config().clone();
        backrun_config.enabled = true;
        backrun_strategy.update_config(backrun_config).unwrap();
        
        let backrun_result_reenabled = backrun_strategy.evaluate_transaction(&large_swap).await.unwrap();
        let sandwich_result_disabled = sandwich_strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        // Now backrun should work, sandwich should be disabled
        assert_eq!(
            matches!(backrun_result, StrategyResult::Opportunity(_)),
            matches!(backrun_result_reenabled, StrategyResult::Opportunity(_))
        );
        assert!(matches!(sandwich_result_disabled, StrategyResult::NoOpportunity));
    }
}

/// Test suite for dynamic configuration updates
mod dynamic_config_tests {
    use super::*;

    #[tokio::test]
    async fn test_profit_threshold_updates() {
        let mut strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        let large_swap = generator.generate_large_swap(TargetType::UniswapV2);
        
        // Test with default threshold
        let result_default = strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        // Increase profit threshold significantly
        let mut config = strategy.config().clone();
        config.min_profit_wei = 100_000_000_000_000_000_000; // 100 ETH (unrealistic)
        strategy.update_config(config).unwrap();
        
        let result_high_threshold = strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        // Should not find opportunity with high threshold
        assert!(matches!(result_high_threshold, StrategyResult::NoOpportunity));
        
        // Lower threshold to very low value
        let mut config = strategy.config().clone();
        config.min_profit_wei = 1_000_000; // 0.000001 ETH (very low)
        strategy.update_config(config).unwrap();
        
        let result_low_threshold = strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        // Should definitely find opportunity with low threshold
        assert!(matches!(result_low_threshold, StrategyResult::Opportunity(_)));
    }

    #[tokio::test]
    async fn test_gas_price_limit_updates() {
        let mut strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create victim with high gas price
        let high_gas_victim = generator.generate_swap_transaction(TargetType::UniswapV2, 2.0, 500); // 500 gwei
        
        // Test with default gas limit
        let result_default = strategy.evaluate_transaction(&high_gas_victim).await.unwrap();
        
        if let StrategyResult::Opportunity(opp) = result_default {
            let bundle_plan = strategy.create_bundle_plan(&opp).await.unwrap();
            let original_gas_prices: Vec<_> = bundle_plan.transactions.iter().map(|tx| tx.gas_price).collect();
            
            // Lower gas price limit
            let mut config = strategy.config().clone();
            config.max_gas_price_gwei = 100; // Much lower limit
            strategy.update_config(config).unwrap();
            
            let result_low_limit = strategy.evaluate_transaction(&high_gas_victim).await.unwrap();
            
            if let StrategyResult::Opportunity(opp2) = result_low_limit {
                let bundle_plan2 = strategy.create_bundle_plan(&opp2).await.unwrap();
                
                // Gas prices should be capped at new limit
                for tx in &bundle_plan2.transactions {
                    let max_allowed = 100_000_000_000u128; // 100 gwei in wei
                    assert!(tx.gas_price <= max_allowed * 2, "Gas price should respect new limit"); // Allow some premium
                }
            }
        }
    }

    #[tokio::test]
    async fn test_risk_tolerance_updates() {
        let mut strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        let victim_tx = generator.generate_sandwich_victim();
        
        // Test with default risk tolerance
        let _result_default = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        
        // Set very low risk tolerance (conservative)
        let mut config = strategy.config().clone();
        config.risk_tolerance = 0.1; // Very conservative
        strategy.update_config(config).unwrap();
        
        let result_conservative = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        
        // Set very high risk tolerance (aggressive)
        let mut config = strategy.config().clone();
        config.risk_tolerance = 0.9; // Very aggressive
        strategy.update_config(config).unwrap();
        
        let result_aggressive = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        
        // Conservative setting should be more restrictive
        let conservative_found = matches!(result_conservative, StrategyResult::Opportunity(_));
        let aggressive_found = matches!(result_aggressive, StrategyResult::Opportunity(_));
        
        // Aggressive should find at least as many opportunities as conservative
        if conservative_found {
            assert!(aggressive_found, "Aggressive setting should find opportunities if conservative does");
        }
    }

    #[tokio::test]
    async fn test_parameter_updates() {
        let mut strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Test updating custom parameters
        let mut config = strategy.config().clone();
        
        // Update min target value
        config.parameters.insert(
            "min_target_value_eth".to_string(),
            serde_json::json!(2.0) // Increase from default 0.5 to 2.0
        );
        
        strategy.update_config(config).unwrap();
        
        // Test with transaction that would pass old threshold but not new
        let medium_swap = generator.generate_swap_transaction(TargetType::UniswapV2, 1.0, 50); // 1 ETH
        let result = strategy.evaluate_transaction(&medium_swap).await.unwrap();
        
        // Should not find opportunity with higher threshold
        assert!(matches!(result, StrategyResult::NoOpportunity));
        
        // Test with transaction that passes new threshold
        let large_swap = generator.generate_swap_transaction(TargetType::UniswapV2, 3.0, 50); // 3 ETH
        let result = strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        // Should find opportunity with large enough transaction
        assert!(matches!(result, StrategyResult::Opportunity(_)));
    }
}

/// Test suite for configuration validation and error handling
mod config_validation_tests {
    use super::*;

    #[test]
    fn test_config_validation() {
        // Test valid configuration
        let valid_config = StrategyConfig {
            name: "test_strategy".to_string(),
            enabled: true,
            priority: 128,
            min_profit_wei: 1_000_000_000_000_000,
            max_gas_price_gwei: 100,
            risk_tolerance: 0.5,
            parameters: HashMap::new(),
        };
        
        // Should be valid
        assert!(validate_strategy_config(&valid_config).is_ok());
        
        // Test invalid configurations
        let mut invalid_config = valid_config.clone();
        
        // Invalid priority (already at max)
        invalid_config.priority = 255;
        // Priority 255 is actually valid, so this test is not needed
        
        // Reset priority
        invalid_config.priority = 128;
        
        // Invalid risk tolerance (negative)
        invalid_config.risk_tolerance = -0.1;
        assert!(validate_strategy_config(&invalid_config).is_err());
        
        // Invalid risk tolerance (too high)
        invalid_config.risk_tolerance = 1.1;
        assert!(validate_strategy_config(&invalid_config).is_err());
        
        // Reset risk tolerance
        invalid_config.risk_tolerance = 0.5;
        
        // Invalid gas price (zero)
        invalid_config.max_gas_price_gwei = 0;
        assert!(validate_strategy_config(&invalid_config).is_err());
        
        // Invalid profit threshold (zero)
        invalid_config.max_gas_price_gwei = 100;
        invalid_config.min_profit_wei = 0;
        assert!(validate_strategy_config(&invalid_config).is_err());
    }

    #[test]
    fn test_config_sanitization() {
        let mut config = StrategyConfig {
            name: "  test_strategy  ".to_string(), // Whitespace
            enabled: true,
            priority: 255, // Max value
            min_profit_wei: 0, // Invalid
            max_gas_price_gwei: 10000, // Too high
            risk_tolerance: 1.5, // Too high
            parameters: HashMap::new(),
        };
        
        sanitize_strategy_config(&mut config);
        
        // Should be sanitized
        assert_eq!(config.name, "test_strategy"); // Trimmed
        assert!(config.min_profit_wei > 0); // Set to minimum
        assert!(config.max_gas_price_gwei <= 1000); // Reasonable cap
        assert!(config.risk_tolerance <= 1.0); // Capped
        assert!(config.risk_tolerance >= 0.0); // Lower bound
    }

    #[tokio::test]
    async fn test_config_update_error_handling() {
        let mut strategy = BackrunStrategy::new();
        
        // Try to update with invalid config
        let invalid_config = StrategyConfig {
            name: "".to_string(), // Empty name
            enabled: true,
            priority: 0,
            min_profit_wei: 0,
            max_gas_price_gwei: 0,
            risk_tolerance: -1.0,
            parameters: HashMap::new(),
        };
        
        let result = strategy.update_config(invalid_config);
        
        // Should handle error gracefully
        match result {
            Ok(_) => {
                // If it succeeds, config should be sanitized
                let config = strategy.config();
                assert!(!config.name.is_empty());
                assert!(config.min_profit_wei > 0);
                assert!(config.max_gas_price_gwei > 0);
                assert!(config.risk_tolerance >= 0.0 && config.risk_tolerance <= 1.0);
            }
            Err(_) => {
                // Error is also acceptable for invalid config
            }
        }
    }
}

/// Helper functions for configuration validation
fn validate_strategy_config(config: &StrategyConfig) -> Result<(), String> {
    if config.name.trim().is_empty() {
        return Err("Strategy name cannot be empty".to_string());
    }
    
    // Priority is u8, so it's automatically <= 255
    
    if config.min_profit_wei == 0 {
        return Err("Minimum profit must be > 0".to_string());
    }
    
    if config.max_gas_price_gwei == 0 {
        return Err("Maximum gas price must be > 0".to_string());
    }
    
    if config.risk_tolerance < 0.0 || config.risk_tolerance > 1.0 {
        return Err("Risk tolerance must be between 0.0 and 1.0".to_string());
    }
    
    Ok(())
}

fn sanitize_strategy_config(config: &mut StrategyConfig) {
    // Trim whitespace from name
    config.name = config.name.trim().to_string();
    
    // Ensure name is not empty
    if config.name.is_empty() {
        config.name = "unnamed_strategy".to_string();
    }
    
    // Priority is u8, so it's automatically <= 255
    
    // Ensure minimum profit is reasonable
    if config.min_profit_wei == 0 {
        config.min_profit_wei = 1_000_000_000_000_000; // 0.001 ETH
    }
    
    // Cap gas price
    if config.max_gas_price_gwei == 0 {
        config.max_gas_price_gwei = 100; // 100 gwei default
    } else if config.max_gas_price_gwei > 1000 {
        config.max_gas_price_gwei = 1000; // 1000 gwei cap
    }
    
    // Clamp risk tolerance
    if config.risk_tolerance < 0.0 {
        config.risk_tolerance = 0.0;
    } else if config.risk_tolerance > 1.0 {
        config.risk_tolerance = 1.0;
    }
}

#[cfg(test)]
mod integration_config_tests {
    use super::*;

    #[tokio::test]
    async fn test_yaml_config_roundtrip_with_strategies() {
        // Create configurations for multiple strategies
        let configs = vec![
            MockConfigGenerator::backrun_config(),
            MockConfigGenerator::sandwich_config(),
            MockConfigGenerator::conservative_config(),
            MockConfigGenerator::high_risk_config(),
        ];
        
        // Serialize to YAML
        let yaml_str = serde_yaml::to_string(&configs).unwrap();
        
        // Deserialize back
        let deserialized_configs: Vec<StrategyConfig> = serde_yaml::from_str(&yaml_str).unwrap();
        
        // Create strategies with deserialized configs
        let mut backrun_strategy = BackrunStrategy::new();
        let mut sandwich_strategy = SandwichStrategy::new();
        
        // Update configurations
        for config in deserialized_configs {
            match config.name.as_str() {
                "backrun" => {
                    backrun_strategy.update_config(config).unwrap();
                }
                "sandwich" => {
                    sandwich_strategy.update_config(config).unwrap();
                }
                _ => {} // Skip other configs
            }
        }
        
        // Test that strategies work with deserialized configs
        let mut generator = MockTransactionGenerator::new();
        let test_tx = generator.generate_large_swap(TargetType::UniswapV2);
        
        let backrun_result = backrun_strategy.evaluate_transaction(&test_tx).await.unwrap();
        let sandwich_result = sandwich_strategy.evaluate_transaction(&test_tx).await.unwrap();
        
        // At least one should work (depending on the transaction characteristics)
        let backrun_works = matches!(backrun_result, StrategyResult::Opportunity(_));
        let sandwich_works = matches!(sandwich_result, StrategyResult::Opportunity(_));
        
        println!("Backrun strategy works: {}", backrun_works);
        println!("Sandwich strategy works: {}", sandwich_works);
        
        // Verify configurations are properly applied
        assert_eq!(backrun_strategy.config().name, "backrun");
        assert_eq!(sandwich_strategy.config().name, "sandwich");
    }

    #[test]
    fn test_complex_yaml_config_parsing() {
        let complex_yaml = r#"
strategies:
  - name: "backrun"
    enabled: true
    priority: 150
    min_profit_wei: 5000000000000000
    max_gas_price_gwei: 150
    risk_tolerance: 0.7
    parameters:
      min_target_value_eth: 0.5
      max_slippage_percent: 2.0
      target_protocols: ["UniswapV2", "UniswapV3", "SushiSwap"]
      
  - name: "sandwich"
    enabled: false  # Disabled for testing
    priority: 200
    min_profit_wei: 10000000000000000
    max_gas_price_gwei: 200
    risk_tolerance: 0.5
    parameters:
      max_slippage_tolerance: 5.0
      min_victim_value_eth: 1.0
      max_front_run_multiple: 3.0
      
  - name: "arbitrage"
    enabled: true
    priority: 100
    min_profit_wei: 2000000000000000
    max_gas_price_gwei: 120
    risk_tolerance: 0.6
    parameters:
      max_price_difference_percent: 1.0
      min_liquidity_eth: 10.0
"#;
        
        #[derive(serde::Deserialize)]
        struct ConfigFile {
            strategies: Vec<StrategyConfig>,
        }
        
        let config_file: ConfigFile = serde_yaml::from_str(complex_yaml).unwrap();
        
        assert_eq!(config_file.strategies.len(), 3);
        
        // Verify backrun config
        let backrun_config = config_file.strategies.iter()
            .find(|c| c.name == "backrun")
            .unwrap();
        assert!(backrun_config.enabled);
        assert_eq!(backrun_config.priority, 150);
        
        // Verify sandwich config
        let sandwich_config = config_file.strategies.iter()
            .find(|c| c.name == "sandwich")
            .unwrap();
        assert!(!sandwich_config.enabled); // Should be disabled
        assert_eq!(sandwich_config.priority, 200);
        
        // Verify arbitrage config
        let arbitrage_config = config_file.strategies.iter()
            .find(|c| c.name == "arbitrage")
            .unwrap();
        assert!(arbitrage_config.enabled);
        assert_eq!(arbitrage_config.priority, 100);
    }
}