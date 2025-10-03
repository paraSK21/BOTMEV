//! Comprehensive unit tests for BackrunStrategy

use super::*;
use crate::test_utils::*;
use mev_core::TargetType;
use tokio;

/// Test suite for BackrunStrategy evaluation
mod evaluation_tests {
    use super::*;

    #[tokio::test]
    async fn test_large_uniswap_v2_swap_detection() {
        let strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Generate a large UniswapV2 swap (2 ETH)
        let large_swap = generator.generate_swap_transaction(TargetType::UniswapV2, 2.0, 50);
        
        let result = strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        match result {
            StrategyResult::Opportunity(opp) => {
                assert_eq!(opp.strategy_name, "backrun");
                assert!(opp.estimated_profit_wei > 0);
                assert!(opp.confidence_score > 0.0);
                assert!(opp.confidence_score <= 1.0);
                
                // Check opportunity type
                match opp.opportunity_type {
                    OpportunityType::Backrun { target_tx_hash, price_impact, .. } => {
                        assert_eq!(target_tx_hash, large_swap.transaction.hash);
                        assert!(price_impact > 0.0);
                    }
                    _ => panic!("Expected Backrun opportunity type"),
                }
            }
            _ => panic!("Expected opportunity for large UniswapV2 swap"),
        }
    }

    #[tokio::test]
    async fn test_large_uniswap_v3_swap_detection() {
        let strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Generate a large UniswapV3 swap (3 ETH)
        let large_swap = generator.generate_swap_transaction(TargetType::UniswapV3, 3.0, 75);
        
        let result = strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        match result {
            StrategyResult::Opportunity(opp) => {
                assert_eq!(opp.strategy_name, "backrun");
                assert!(opp.estimated_profit_wei > 0);
                
                // UniswapV3 should have different characteristics than V2
                match opp.opportunity_type {
                    OpportunityType::Backrun { price_impact, .. } => {
                        // V3 typically has lower price impact due to concentrated liquidity
                        assert!(price_impact > 0.0);
                        assert!(price_impact < 0.1); // Should be reasonable
                    }
                    _ => panic!("Expected Backrun opportunity type"),
                }
            }
            _ => panic!("Expected opportunity for large UniswapV3 swap"),
        }
    }

    #[tokio::test]
    async fn test_curve_swap_detection() {
        let strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Generate a Curve swap (1.5 ETH)
        let curve_swap = generator.generate_swap_transaction(TargetType::Curve, 1.5, 40);
        
        let result = strategy.evaluate_transaction(&curve_swap).await.unwrap();
        
        match result {
            StrategyResult::Opportunity(opp) => {
                match opp.opportunity_type {
                    OpportunityType::Backrun { price_impact, .. } => {
                        // Curve should have very low price impact (stablecoin optimized)
                        assert!(price_impact > 0.0);
                        assert!(price_impact < 0.02); // Should be very low for Curve
                    }
                    _ => panic!("Expected Backrun opportunity type"),
                }
            }
            _ => panic!("Expected opportunity for Curve swap"),
        }
    }

    #[tokio::test]
    async fn test_small_swap_rejection() {
        let strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Generate a small swap (0.05 ETH - below threshold)
        let small_swap = generator.generate_swap_transaction(TargetType::UniswapV2, 0.05, 50);
        
        let result = strategy.evaluate_transaction(&small_swap).await.unwrap();
        
        match result {
            StrategyResult::NoOpportunity => {
                // Expected - transaction too small
            }
            _ => panic!("Expected no opportunity for small swap"),
        }
    }

    #[tokio::test]
    async fn test_non_dex_transaction_rejection() {
        let strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Generate a simple transfer (not a DEX transaction)
        let transfer = generator.generate_simple_transfer();
        
        let result = strategy.evaluate_transaction(&transfer).await.unwrap();
        
        match result {
            StrategyResult::NoOpportunity => {
                // Expected - not a DEX transaction
            }
            _ => panic!("Expected no opportunity for simple transfer"),
        }
    }

    #[tokio::test]
    async fn test_disabled_strategy() {
        let mut strategy = BackrunStrategy::new();
        
        // Disable the strategy
        let mut config = strategy.config().clone();
        config.enabled = false;
        strategy.update_config(config).unwrap();
        
        let mut generator = MockTransactionGenerator::new();
        let large_swap = generator.generate_large_swap(TargetType::UniswapV2);
        
        let result = strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        match result {
            StrategyResult::NoOpportunity => {
                // Expected - strategy is disabled
            }
            _ => panic!("Expected no opportunity when strategy is disabled"),
        }
    }

    #[tokio::test]
    async fn test_profit_threshold_filtering() {
        let mut strategy = BackrunStrategy::new();
        
        // Set very high profit threshold
        let mut config = strategy.config().clone();
        config.min_profit_wei = 100_000_000_000_000_000_000; // 100 ETH (unrealistic)
        strategy.update_config(config).unwrap();
        
        let mut generator = MockTransactionGenerator::new();
        let large_swap = generator.generate_large_swap(TargetType::UniswapV2);
        
        let result = strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        match result {
            StrategyResult::NoOpportunity => {
                // Expected - profit below threshold
            }
            _ => panic!("Expected no opportunity when profit threshold too high"),
        }
    }
}

/// Test suite for BackrunStrategy bundle creation
mod bundle_creation_tests {
    use super::*;

    #[tokio::test]
    async fn test_bundle_plan_creation() {
        let strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create an opportunity first
        let large_swap = generator.generate_large_swap(TargetType::UniswapV2);
        let result = strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        let opportunity = match result {
            StrategyResult::Opportunity(opp) => opp,
            _ => panic!("Expected opportunity"),
        };
        
        // Create bundle plan
        let bundle_plan = strategy.create_bundle_plan(&opportunity).await.unwrap();
        
        assert_eq!(bundle_plan.strategy_name, "backrun");
        assert_eq!(bundle_plan.opportunity_id, opportunity.id);
        assert_eq!(bundle_plan.transactions.len(), 1);
        assert!(bundle_plan.estimated_profit_wei > 0);
        assert!(bundle_plan.risk_score >= 0.0 && bundle_plan.risk_score <= 1.0);
        
        // Check transaction details
        let tx = &bundle_plan.transactions[0];
        assert_eq!(tx.transaction_type, TransactionType::BackRun);
        assert!(tx.gas_limit > 0);
        assert!(tx.gas_price > 0);
        assert!(!tx.description.is_empty());
    }

    #[tokio::test]
    async fn test_gas_price_calculation() {
        let strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create transaction with specific gas price
        let target_gas_price_gwei = 100;
        let large_swap = generator.generate_swap_transaction(
            TargetType::UniswapV2, 
            2.0, 
            target_gas_price_gwei
        );
        
        let result = strategy.evaluate_transaction(&large_swap).await.unwrap();
        let opportunity = match result {
            StrategyResult::Opportunity(opp) => opp,
            _ => panic!("Expected opportunity"),
        };
        
        let bundle_plan = strategy.create_bundle_plan(&opportunity).await.unwrap();
        let tx = &bundle_plan.transactions[0];
        
        // Our gas price should be slightly higher than target (premium)
        let target_gas_price_wei = (target_gas_price_gwei as u128) * 1_000_000_000;
        assert!(tx.gas_price > target_gas_price_wei);
        
        // But should respect our maximum
        let max_gas_price_wei = (strategy.config().max_gas_price_gwei as u128) * 1_000_000_000;
        assert!(tx.gas_price <= max_gas_price_wei);
    }

    #[tokio::test]
    async fn test_router_address_selection() {
        let strategy = BackrunStrategy::new();
        
        // Test different target types
        let test_cases = vec![
            (TargetType::UniswapV2, "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D"),
            (TargetType::UniswapV3, "0xE592427A0AEce92De3Edee1F18E0157C05861564"),
            (TargetType::SushiSwap, "0xd9e1cE17f2641f24aE83637ab66a2cca9C378B9F"),
        ];
        
        for (target_type, expected_router) in test_cases {
            let router = strategy.get_router_address(&target_type).unwrap();
            assert_eq!(router, expected_router);
        }
        
        // Test unsupported type
        let result = strategy.get_router_address(&TargetType::Unknown);
        assert!(result.is_err());
    }
}

/// Test suite for BackrunStrategy configuration
mod configuration_tests {
    use super::*;

    #[tokio::test]
    async fn test_configuration_update() {
        let mut strategy = BackrunStrategy::new();
        
        let original_config = strategy.config().clone();
        assert_eq!(original_config.name, "backrun");
        assert!(original_config.enabled);
        
        // Update configuration
        let mut new_config = original_config.clone();
        new_config.enabled = false;
        new_config.min_profit_wei = 1_000_000_000_000_000_000; // 1 ETH
        new_config.max_gas_price_gwei = 200;
        
        strategy.update_config(new_config.clone()).unwrap();
        
        let updated_config = strategy.config();
        assert_eq!(updated_config.enabled, false);
        assert_eq!(updated_config.min_profit_wei, 1_000_000_000_000_000_000);
        assert_eq!(updated_config.max_gas_price_gwei, 200);
    }

    #[tokio::test]
    async fn test_parameter_access() {
        let strategy = BackrunStrategy::new();
        let config = strategy.config();
        
        // Check default parameters
        assert!(config.parameters.contains_key("min_target_value_eth"));
        assert!(config.parameters.contains_key("max_slippage_percent"));
        assert!(config.parameters.contains_key("target_protocols"));
        
        // Verify parameter values
        let min_value = config.parameters.get("min_target_value_eth")
            .and_then(|v| v.as_f64())
            .unwrap();
        assert_eq!(min_value, 0.5);
        
        let max_slippage = config.parameters.get("max_slippage_percent")
            .and_then(|v| v.as_f64())
            .unwrap();
        assert_eq!(max_slippage, 2.0);
    }

    #[tokio::test]
    async fn test_custom_parameters() {
        let mut strategy = BackrunStrategy::new();
        let mut config = strategy.config().clone();
        
        // Add custom parameter
        config.parameters.insert(
            "custom_param".to_string(),
            serde_json::json!("custom_value")
        );
        
        strategy.update_config(config).unwrap();
        
        let updated_config = strategy.config();
        assert!(updated_config.parameters.contains_key("custom_param"));
        assert_eq!(
            updated_config.parameters.get("custom_param").unwrap().as_str().unwrap(),
            "custom_value"
        );
    }
}

/// Test suite for BackrunStrategy performance
mod performance_tests {
    use super::*;

    #[tokio::test]
    async fn test_evaluation_performance() {
        let strategy = BackrunStrategy::new();
        
        // Generate test transactions
        let transactions = PerformanceTestUtils::generate_load_test_transactions(100);
        
        // Measure performance
        let metrics = PerformanceTestUtils::measure_evaluation_performance(&strategy, &transactions).await;
        
        // Verify performance requirements
        assert!(metrics.avg_time_per_tx_ms < 10.0, "Average evaluation time too high: {:.2}ms", metrics.avg_time_per_tx_ms);
        assert!(metrics.throughput_tx_per_sec > 100.0, "Throughput too low: {:.2} tx/sec", metrics.throughput_tx_per_sec);
        assert_eq!(metrics.errors, 0, "Should have no errors");
        
        metrics.print_summary();
    }

    #[tokio::test]
    async fn test_concurrent_evaluation() {
        let strategy = std::sync::Arc::new(BackrunStrategy::new());
        let mut generator = MockTransactionGenerator::new();
        
        // Create multiple transactions
        let transactions: Vec<_> = (0..10)
            .map(|_| generator.generate_large_swap(TargetType::UniswapV2))
            .collect();
        
        // Evaluate concurrently
        let futures: Vec<_> = transactions
            .iter()
            .map(|tx| {
                let strategy = strategy.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    strategy.evaluate_transaction(&tx).await
                })
            })
            .collect();
        
        let results = futures::future::join_all(futures).await;
        
        // All should succeed
        for result in results {
            let eval_result = result.unwrap().unwrap();
            match eval_result {
                StrategyResult::Opportunity(_) => {
                    // Expected for large swaps
                }
                StrategyResult::NoOpportunity => {
                    // Also acceptable depending on profit calculation
                }
                StrategyResult::BundlePlan(_) => {
                    // Not expected from evaluate_transaction
                    panic!("Unexpected BundlePlan result from evaluation");
                }
                StrategyResult::Error(_) => {
                    panic!("Should not have errors in concurrent evaluation");
                }
            }
        }
    }
}

/// Test suite for BackrunStrategy edge cases
mod edge_case_tests {
    use super::*;

    #[tokio::test]
    async fn test_zero_value_transaction() {
        let strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create transaction with zero value
        let mut zero_value_tx = generator.generate_swap_transaction(TargetType::UniswapV2, 0.0, 50);
        zero_value_tx.transaction.value = "0".to_string();
        
        let result = strategy.evaluate_transaction(&zero_value_tx).await.unwrap();
        
        match result {
            StrategyResult::NoOpportunity => {
                // Expected - zero value transaction
            }
            _ => panic!("Expected no opportunity for zero value transaction"),
        }
    }

    #[tokio::test]
    async fn test_very_high_gas_price() {
        let strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create transaction with very high gas price (1000 gwei)
        let high_gas_tx = generator.generate_swap_transaction(TargetType::UniswapV2, 2.0, 1000);
        
        let result = strategy.evaluate_transaction(&high_gas_tx).await.unwrap();
        
        // Should still detect opportunity but bundle creation might adjust gas price
        match result {
            StrategyResult::Opportunity(opp) => {
                let bundle_plan = strategy.create_bundle_plan(&opp).await.unwrap();
                let tx = &bundle_plan.transactions[0];
                
                // Our gas price should be capped at our maximum
                let max_gas_price_wei = (strategy.config().max_gas_price_gwei as u128) * 1_000_000_000;
                assert!(tx.gas_price <= max_gas_price_wei);
            }
            StrategyResult::NoOpportunity => {
                // Also acceptable if profit calculation determines it's not profitable
            }
            _ => panic!("Unexpected result"),
        }
    }

    #[tokio::test]
    async fn test_malformed_transaction_data() {
        let strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create transaction with malformed data
        let mut malformed_tx = generator.generate_large_swap(TargetType::UniswapV2);
        malformed_tx.transaction.value = "invalid_value".to_string();
        
        let result = strategy.evaluate_transaction(&malformed_tx).await.unwrap();
        
        match result {
            StrategyResult::NoOpportunity => {
                // Expected - malformed data should be handled gracefully
            }
            _ => panic!("Expected no opportunity for malformed transaction"),
        }
    }

    #[tokio::test]
    async fn test_expired_opportunity() {
        let strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        let large_swap = generator.generate_large_swap(TargetType::UniswapV2);
        let result = strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        let opportunity = match result {
            StrategyResult::Opportunity(opp) => opp,
            _ => panic!("Expected opportunity"),
        };
        
        // Check that opportunity has reasonable expiry
        let current_time = chrono::Utc::now().timestamp() as u64;
        assert!(opportunity.expiry_timestamp > current_time);
        assert!(opportunity.expiry_timestamp <= current_time + 60); // Should expire within 60 seconds
    }
}

/// Integration tests with mock scenarios
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_complete_backrun_scenario() {
        let strategy = BackrunStrategy::new();
        
        // Build a complete test scenario
        let scenario = TestScenarioBuilder::new("backrun_integration")
            .add_backrun_scenario()
            .add_noise_transactions(3)
            .add_unprofitable_transactions(2)
            .build();
        
        let mut opportunities = 0;
        let mut bundles = 0;
        
        // Process all transactions
        for tx in &scenario.transactions {
            let result = strategy.evaluate_transaction(tx).await.unwrap();
            
            match result {
                StrategyResult::Opportunity(opp) => {
                    opportunities += 1;
                    
                    // Try to create bundle
                    let bundle_result = strategy.create_bundle_plan(&opp).await;
                    if bundle_result.is_ok() {
                        bundles += 1;
                    }
                }
                _ => {}
            }
        }
        
        // Validate results
        assert!(scenario.validate_results(opportunities, bundles));
        scenario.print_summary();
        println!("Actual - Opportunities: {}, Bundles: {}", opportunities, bundles);
    }

    #[tokio::test]
    async fn test_strategy_statistics() {
        let mut strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Process multiple transactions
        for _ in 0..5 {
            let tx = generator.generate_large_swap(TargetType::UniswapV2);
            let _ = strategy.evaluate_transaction(&tx).await;
        }
        
        let stats = strategy.get_stats();
        
        // Initially stats should be default (no mutable access in this test)
        assert_eq!(stats.opportunities_detected, 0);
        assert_eq!(stats.bundles_created, 0);
        
        // Test stats reset
        strategy.reset_stats();
        let reset_stats = strategy.get_stats();
        assert_eq!(reset_stats.opportunities_detected, 0);
    }
}

/// Benchmark tests for performance validation
#[cfg(test)]
mod benchmark_tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn benchmark_evaluation_speed() {
        let strategy = BackrunStrategy::new();
        let transactions = PerformanceTestUtils::generate_load_test_transactions(1000);
        
        let start = Instant::now();
        
        for tx in &transactions {
            let _ = strategy.evaluate_transaction(tx).await.unwrap();
        }
        
        let duration = start.elapsed();
        let avg_time_ms = duration.as_millis() as f64 / transactions.len() as f64;
        let throughput = transactions.len() as f64 / duration.as_secs_f64();
        
        println!("Benchmark Results:");
        println!("  Transactions: {}", transactions.len());
        println!("  Total Time: {:.2}ms", duration.as_millis());
        println!("  Avg Time per Tx: {:.3}ms", avg_time_ms);
        println!("  Throughput: {:.2} tx/sec", throughput);
        
        // Performance requirements from design document
        assert!(avg_time_ms < 25.0, "Average evaluation time exceeds 25ms requirement: {:.3}ms", avg_time_ms);
        assert!(throughput > 40.0, "Throughput below minimum requirement: {:.2} tx/sec", throughput);
    }

    #[tokio::test]
    async fn benchmark_bundle_creation_speed() {
        let strategy = BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create opportunities first
        let mut opportunities = Vec::new();
        for _ in 0..100 {
            let tx = generator.generate_large_swap(TargetType::UniswapV2);
            if let Ok(StrategyResult::Opportunity(opp)) = strategy.evaluate_transaction(&tx).await {
                opportunities.push(opp);
            }
        }
        
        let start = Instant::now();
        
        for opp in &opportunities {
            let _ = strategy.create_bundle_plan(opp).await.unwrap();
        }
        
        let duration = start.elapsed();
        let avg_time_ms = duration.as_millis() as f64 / opportunities.len() as f64;
        
        println!("Bundle Creation Benchmark:");
        println!("  Opportunities: {}", opportunities.len());
        println!("  Total Time: {:.2}ms", duration.as_millis());
        println!("  Avg Time per Bundle: {:.3}ms", avg_time_ms);
        
        assert!(avg_time_ms < 5.0, "Bundle creation too slow: {:.3}ms", avg_time_ms);
    }
}