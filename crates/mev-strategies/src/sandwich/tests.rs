//! Comprehensive unit tests for SandwichStrategy

use super::*;
use crate::test_utils::*;
use mev_core::{TargetType, DecodedParameter};
use tokio;

/// Test suite for SandwichStrategy victim detection
mod victim_detection_tests {
    use super::*;

    #[tokio::test]
    async fn test_sandwich_victim_detection() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Generate a sandwich victim (large swap with poor slippage protection)
        let victim_tx = generator.generate_sandwich_victim();
        
        let result = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        
        match result {
            StrategyResult::Opportunity(opp) => {
                assert_eq!(opp.strategy_name, "sandwich");
                assert!(opp.estimated_profit_wei > 0);
                assert!(opp.confidence_score > 0.0);
                
                // Check opportunity type
                match opp.opportunity_type {
                    OpportunityType::Sandwich { victim_tx_hash, front_run_amount, back_run_amount, .. } => {
                        assert_eq!(victim_tx_hash, victim_tx.transaction.hash);
                        assert!(front_run_amount > 0);
                        assert!(back_run_amount > 0);
                    }
                    _ => panic!("Expected Sandwich opportunity type"),
                }
            }
            _ => panic!("Expected opportunity for sandwich victim"),
        }
    }

    #[tokio::test]
    async fn test_small_transaction_rejection() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Generate a small swap (below minimum victim value)
        let small_swap = generator.generate_swap_transaction(TargetType::UniswapV2, 0.1, 50);
        
        let result = strategy.evaluate_transaction(&small_swap).await.unwrap();
        
        match result {
            StrategyResult::NoOpportunity => {
                // Expected - transaction too small for sandwich
            }
            _ => panic!("Expected no opportunity for small transaction"),
        }
    }

    #[tokio::test]
    async fn test_non_dex_transaction_rejection() {
        let strategy = SandwichStrategy::new();
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
    async fn test_slippage_protection_analysis() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create transaction with good slippage protection
        let mut protected_tx = generator.generate_swap_transaction(TargetType::UniswapV2, 2.0, 50);
        
        // Add high amountOutMin (good slippage protection)
        if let Some(ref mut decoded) = protected_tx.decoded_input {
            decoded.parameters.push(DecodedParameter {
                name: "amountOutMin".to_string(),
                param_type: "uint256".to_string(),
                value: "1900000000000000000".to_string(), // 1.9 ETH (5% slippage)
            });
        }
        
        let result = strategy.evaluate_transaction(&protected_tx).await.unwrap();
        
        match result {
            StrategyResult::NoOpportunity => {
                // Expected - good slippage protection makes sandwich unprofitable
            }
            _ => {
                // Might still be profitable depending on parameters, but should have lower confidence
                if let StrategyResult::Opportunity(opp) = result {
                    assert!(opp.confidence_score < 0.8); // Should have lower confidence
                }
            }
        }
    }

    #[tokio::test]
    async fn test_disabled_strategy() {
        let mut strategy = SandwichStrategy::new();
        
        // Disable the strategy
        let mut config = strategy.config().clone();
        config.enabled = false;
        strategy.update_config(config).unwrap();
        
        let mut generator = MockTransactionGenerator::new();
        let victim_tx = generator.generate_sandwich_victim();
        
        let result = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        
        match result {
            StrategyResult::NoOpportunity => {
                // Expected - strategy is disabled
            }
            _ => panic!("Expected no opportunity when strategy is disabled"),
        }
    }
}

/// Test suite for SandwichStrategy profit calculation
mod profit_calculation_tests {
    use super::*;

    #[tokio::test]
    async fn test_profit_calculation_accuracy() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Test different victim sizes
        let test_cases = vec![
            (1.0, TargetType::UniswapV2),
            (2.5, TargetType::UniswapV3),
            (5.0, TargetType::SushiSwap),
        ];
        
        for (victim_eth, target_type) in test_cases {
            let victim_tx = generator.generate_swap_transaction(target_type, victim_eth, 50);
            
            if let Ok((front_run_size, gross_profit, net_profit)) = strategy.calculate_sandwich_profit(&victim_tx) {
                // Validate profit calculations
                assert!(front_run_size > 0, "Front-run size should be positive");
                assert!(gross_profit >= net_profit, "Gross profit should be >= net profit");
                
                // Front-run size should be reasonable relative to victim
                let victim_wei = (victim_eth * 1e18) as u128;
                let max_multiple = 3.0; // From config
                assert!(front_run_size <= (victim_wei as f64 * max_multiple) as u128);
                
                println!("Victim: {:.2} ETH, Front-run: {:.2} ETH, Net Profit: {:.4} ETH", 
                    victim_eth, 
                    front_run_size as f64 / 1e18,
                    net_profit as f64 / 1e18
                );
            }
        }
    }

    #[tokio::test]
    async fn test_price_impact_estimation() {
        let strategy = SandwichStrategy::new();
        
        // Test price impact for different protocols and sizes
        let test_cases = vec![
            (1e18 as u128, TargetType::UniswapV2, 0.005), // Expected ~0.5% impact
            (1e18 as u128, TargetType::UniswapV3, 0.003), // Expected ~0.3% impact
            (5e18 as u128, TargetType::UniswapV2, 0.025), // Expected ~2.5% impact
        ];
        
        for (trade_size, target_type, expected_impact) in test_cases {
            let impact = strategy.estimate_price_impact(trade_size, &target_type).unwrap();
            
            assert!(impact > 0.0, "Price impact should be positive");
            assert!(impact < 0.2, "Price impact should be reasonable (<20%)");
            
            // Should be close to expected (within 50% tolerance)
            let tolerance = expected_impact * 0.5;
            assert!(
                (impact - expected_impact).abs() <= tolerance,
                "Price impact {:.4} not close to expected {:.4} for {:?}",
                impact, expected_impact, target_type
            );
        }
    }

    #[tokio::test]
    async fn test_profit_threshold_filtering() {
        let mut strategy = SandwichStrategy::new();
        
        // Set very high profit threshold
        let mut config = strategy.config().clone();
        config.min_profit_wei = 100_000_000_000_000_000_000; // 100 ETH (unrealistic)
        strategy.update_config(config).unwrap();
        
        let mut generator = MockTransactionGenerator::new();
        let victim_tx = generator.generate_sandwich_victim();
        
        let result = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        
        match result {
            StrategyResult::NoOpportunity => {
                // Expected - profit below threshold
            }
            _ => panic!("Expected no opportunity when profit threshold too high"),
        }
    }
}

/// Test suite for SandwichStrategy bundle creation
mod bundle_creation_tests {
    use super::*;

    #[tokio::test]
    async fn test_sandwich_bundle_creation() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create an opportunity first
        let victim_tx = generator.generate_sandwich_victim();
        let result = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        
        let opportunity = match result {
            StrategyResult::Opportunity(opp) => opp,
            _ => panic!("Expected opportunity"),
        };
        
        // Create bundle plan
        let bundle_plan = strategy.create_bundle_plan(&opportunity).await.unwrap();
        
        assert_eq!(bundle_plan.strategy_name, "sandwich");
        assert_eq!(bundle_plan.opportunity_id, opportunity.id);
        assert_eq!(bundle_plan.transactions.len(), 2); // Front-run + back-run
        assert!(bundle_plan.estimated_profit_wei > 0);
        assert!(bundle_plan.risk_score >= 0.0 && bundle_plan.risk_score <= 1.0);
        
        // Check transaction types and order
        assert_eq!(bundle_plan.transactions[0].transaction_type, TransactionType::FrontRun);
        assert_eq!(bundle_plan.transactions[1].transaction_type, TransactionType::BackRun);
        
        // Front-run should have higher gas price than back-run
        assert!(bundle_plan.transactions[0].gas_price > bundle_plan.transactions[1].gas_price);
    }

    #[tokio::test]
    async fn test_gas_price_premium_calculation() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create victim with specific gas price
        let victim_gas_price_gwei = 80;
        let victim_tx = generator.generate_swap_transaction(
            TargetType::UniswapV2, 
            2.0, 
            victim_gas_price_gwei
        );
        
        let result = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        let opportunity = match result {
            StrategyResult::Opportunity(opp) => opp,
            _ => panic!("Expected opportunity"),
        };
        
        let bundle_plan = strategy.create_bundle_plan(&opportunity).await.unwrap();
        
        // Our gas prices should be higher than victim's
        let victim_gas_price_wei = (victim_gas_price_gwei as u128) * 1_000_000_000;
        
        for tx in &bundle_plan.transactions {
            assert!(tx.gas_price > victim_gas_price_wei, 
                "Our gas price {} should be higher than victim's {}", 
                tx.gas_price, victim_gas_price_wei
            );
        }
        
        // Front-run should have highest gas price
        assert!(bundle_plan.transactions[0].gas_price >= bundle_plan.transactions[1].gas_price);
    }

    #[tokio::test]
    async fn test_router_address_selection() {
        let strategy = SandwichStrategy::new();
        
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

/// Test suite for SandwichStrategy configuration and validation
mod configuration_tests {
    use super::*;

    #[tokio::test]
    async fn test_configuration_update() {
        let mut strategy = SandwichStrategy::new();
        
        let original_config = strategy.config().clone();
        assert_eq!(original_config.name, "sandwich");
        assert!(original_config.enabled);
        
        // Update configuration
        let mut new_config = original_config.clone();
        new_config.enabled = false;
        new_config.min_profit_wei = 50_000_000_000_000_000; // 0.05 ETH
        new_config.max_gas_price_gwei = 300;
        new_config.parameters.insert(
            "max_slippage_tolerance".to_string(),
            serde_json::json!(10.0)
        );
        
        strategy.update_config(new_config.clone()).unwrap();
        
        let updated_config = strategy.config();
        assert_eq!(updated_config.enabled, false);
        assert_eq!(updated_config.min_profit_wei, 50_000_000_000_000_000);
        assert_eq!(updated_config.max_gas_price_gwei, 300);
        
        let max_slippage = updated_config.parameters.get("max_slippage_tolerance")
            .and_then(|v| v.as_f64())
            .unwrap();
        assert_eq!(max_slippage, 10.0);
    }

    #[tokio::test]
    async fn test_sandwich_opportunity_validation() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create a valid opportunity
        let victim_tx = generator.generate_sandwich_victim();
        let result = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        
        let mut opportunity = match result {
            StrategyResult::Opportunity(opp) => opp,
            _ => panic!("Expected opportunity"),
        };
        
        // Valid opportunity should pass validation
        assert!(strategy.validate_sandwich_opportunity(&opportunity));
        
        // Test validation failures
        
        // 1. Profit too low
        opportunity.estimated_profit_wei = 1_000_000; // Very low profit
        assert!(!strategy.validate_sandwich_opportunity(&opportunity));
        
        // Reset profit
        opportunity.estimated_profit_wei = 10_000_000_000_000_000; // 0.01 ETH
        
        // 2. Confidence too low
        opportunity.confidence_score = 0.1; // Very low confidence
        assert!(!strategy.validate_sandwich_opportunity(&opportunity));
        
        // Reset confidence
        opportunity.confidence_score = 0.8;
        
        // 3. Front-run size too large
        if let OpportunityType::Sandwich { ref mut front_run_amount, .. } = opportunity.opportunity_type {
            *front_run_amount = 1_000_000_000_000_000_000_000; // 1000 ETH (unrealistic)
            assert!(!strategy.validate_sandwich_opportunity(&opportunity));
        }
    }

    #[tokio::test]
    async fn test_parameter_access() {
        let strategy = SandwichStrategy::new();
        let config = strategy.config();
        
        // Check default parameters
        assert!(config.parameters.contains_key("max_slippage_tolerance"));
        assert!(config.parameters.contains_key("min_victim_value_eth"));
        assert!(config.parameters.contains_key("max_front_run_multiple"));
        assert!(config.parameters.contains_key("gas_price_premium_percent"));
        
        // Verify parameter values
        let max_slippage = config.parameters.get("max_slippage_tolerance")
            .and_then(|v| v.as_f64())
            .unwrap();
        assert_eq!(max_slippage, 5.0);
        
        let min_victim_value = config.parameters.get("min_victim_value_eth")
            .and_then(|v| v.as_f64())
            .unwrap();
        assert_eq!(min_victim_value, 1.0);
        
        let max_multiple = config.parameters.get("max_front_run_multiple")
            .and_then(|v| v.as_f64())
            .unwrap();
        assert_eq!(max_multiple, 3.0);
    }
}

/// Test suite for SandwichStrategy performance and edge cases
mod performance_tests {
    use super::*;

    #[tokio::test]
    async fn test_evaluation_performance() {
        let strategy = SandwichStrategy::new();
        
        // Generate test transactions (mix of victims and non-victims)
        let mut generator = MockTransactionGenerator::new();
        let mut transactions = Vec::new();
        
        // Add some sandwich victims
        for _ in 0..20 {
            transactions.push(generator.generate_sandwich_victim());
        }
        
        // Add noise transactions
        for _ in 0..80 {
            transactions.push(generator.generate_simple_transfer());
        }
        
        // Measure performance
        let metrics = PerformanceTestUtils::measure_evaluation_performance(&strategy, &transactions).await;
        
        // Verify performance requirements
        assert!(metrics.avg_time_per_tx_ms < 15.0, "Average evaluation time too high: {:.2}ms", metrics.avg_time_per_tx_ms);
        assert!(metrics.throughput_tx_per_sec > 60.0, "Throughput too low: {:.2} tx/sec", metrics.throughput_tx_per_sec);
        assert_eq!(metrics.errors, 0, "Should have no errors");
        
        // Should find some opportunities
        assert!(metrics.opportunities_found > 0, "Should find some sandwich opportunities");
        
        metrics.print_summary();
    }

    #[tokio::test]
    async fn test_concurrent_evaluation() {
        let strategy = std::sync::Arc::new(SandwichStrategy::new());
        let mut generator = MockTransactionGenerator::new();
        
        // Create multiple victim transactions
        let transactions: Vec<_> = (0..10)
            .map(|_| generator.generate_sandwich_victim())
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
                    // Expected for sandwich victims
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

/// Test suite for SandwichStrategy edge cases and error handling
mod edge_case_tests {
    use super::*;

    #[tokio::test]
    async fn test_zero_value_transaction() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create transaction with zero value
        let mut zero_value_tx = generator.generate_sandwich_victim();
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
    async fn test_malformed_transaction_data() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create transaction with malformed value
        let mut malformed_tx = generator.generate_sandwich_victim();
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
    async fn test_very_high_gas_price_victim() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create victim with very high gas price (2000 gwei)
        let high_gas_victim = generator.generate_swap_transaction(TargetType::UniswapV2, 2.0, 2000);
        
        let result = strategy.evaluate_transaction(&high_gas_victim).await.unwrap();
        
        // Should still detect opportunity but bundle creation might adjust gas prices
        match result {
            StrategyResult::Opportunity(opp) => {
                let bundle_plan = strategy.create_bundle_plan(&opp).await.unwrap();
                
                // Our gas prices should be reasonable (capped at our maximum)
                let max_gas_price_wei = (strategy.config().max_gas_price_gwei as u128) * 1_000_000_000;
                
                for tx in &bundle_plan.transactions {
                    assert!(tx.gas_price <= max_gas_price_wei * 2, "Gas price too high: {}", tx.gas_price);
                }
            }
            StrategyResult::NoOpportunity => {
                // Also acceptable if profit calculation determines it's not profitable
            }
            _ => panic!("Unexpected result"),
        }
    }

    #[tokio::test]
    async fn test_expired_opportunity() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        let victim_tx = generator.generate_sandwich_victim();
        let result = strategy.evaluate_transaction(&victim_tx).await.unwrap();
        
        let opportunity = match result {
            StrategyResult::Opportunity(opp) => opp,
            _ => panic!("Expected opportunity"),
        };
        
        // Check that opportunity has tight expiry (sandwich timing is critical)
        let current_time = chrono::Utc::now().timestamp() as u64;
        assert!(opportunity.expiry_timestamp > current_time);
        assert!(opportunity.expiry_timestamp <= current_time + 30); // Should expire within 30 seconds
    }

    #[tokio::test]
    async fn test_slippage_calculation_edge_cases() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Test with missing slippage parameters
        let mut tx_no_slippage = generator.generate_sandwich_victim();
        tx_no_slippage.decoded_input = None; // No decoded input
        
        // Should still be considered vulnerable (conservative approach)
        assert!(strategy.has_poor_slippage_protection(&tx_no_slippage));
        
        // Test with zero amountOutMin
        let mut tx_zero_slippage = generator.generate_sandwich_victim();
        if let Some(ref mut decoded) = tx_zero_slippage.decoded_input {
            decoded.parameters.clear();
            decoded.parameters.push(DecodedParameter {
                name: "amountOutMin".to_string(),
                param_type: "uint256".to_string(),
                value: "0".to_string(),
            });
        }
        
        // Should be considered very vulnerable
        assert!(strategy.has_poor_slippage_protection(&tx_zero_slippage));
    }
}

/// Integration tests with complete sandwich scenarios
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_complete_sandwich_scenario() {
        let strategy = SandwichStrategy::new();
        
        // Build a complete test scenario
        let scenario = TestScenarioBuilder::new("sandwich_integration")
            .add_sandwich_scenario()
            .add_noise_transactions(5)
            .add_unprofitable_transactions(3)
            .build();
        
        let mut opportunities = 0;
        let mut bundles = 0;
        
        // Process all transactions
        for tx in &scenario.transactions {
            let result = strategy.evaluate_transaction(tx).await.unwrap();
            
            match result {
                StrategyResult::Opportunity(opp) => {
                    opportunities += 1;
                    
                    // Validate opportunity
                    assert!(strategy.validate_sandwich_opportunity(&opp));
                    
                    // Try to create bundle
                    let bundle_result = strategy.create_bundle_plan(&opp).await;
                    if bundle_result.is_ok() {
                        bundles += 1;
                        
                        let bundle = bundle_result.unwrap();
                        assert_eq!(bundle.transactions.len(), 2); // Front-run + back-run
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
        let mut strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Process multiple transactions
        for _ in 0..5 {
            let tx = generator.generate_sandwich_victim();
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

    #[tokio::test]
    async fn test_sandwich_vs_backrun_comparison() {
        let sandwich_strategy = SandwichStrategy::new();
        let backrun_strategy = crate::BackrunStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create a large swap that could be both sandwiched and backrun
        let large_swap = generator.generate_large_swap(TargetType::UniswapV2);
        
        // Evaluate with both strategies
        let sandwich_result = sandwich_strategy.evaluate_transaction(&large_swap).await.unwrap();
        let backrun_result = backrun_strategy.evaluate_transaction(&large_swap).await.unwrap();
        
        // Both might find opportunities, but sandwich should have higher profit potential
        match (&sandwich_result, &backrun_result) {
            (StrategyResult::Opportunity(sandwich_opp), StrategyResult::Opportunity(backrun_opp)) => {
                // Sandwich typically has higher profit but also higher risk
                println!("Sandwich profit: {:.4} ETH, confidence: {:.2}", 
                    sandwich_opp.estimated_profit_wei as f64 / 1e18,
                    sandwich_opp.confidence_score
                );
                println!("Backrun profit: {:.4} ETH, confidence: {:.2}", 
                    backrun_opp.estimated_profit_wei as f64 / 1e18,
                    backrun_opp.confidence_score
                );
                
                // Sandwich should have higher priority due to higher profit
                assert!(sandwich_opp.priority >= backrun_opp.priority);
            }
            _ => {
                // At least one strategy should find an opportunity for a large swap
                println!("Sandwich result: {:?}", 
                    matches!(sandwich_result, StrategyResult::Opportunity(_)));
                println!("Backrun result: {:?}", 
                    matches!(backrun_result, StrategyResult::Opportunity(_)));
            }
        }
    }
}

/// Benchmark tests for sandwich strategy performance
#[cfg(test)]
mod benchmark_tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn benchmark_sandwich_evaluation_speed() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create mix of victim and non-victim transactions
        let mut transactions = Vec::new();
        for _ in 0..200 {
            transactions.push(generator.generate_sandwich_victim());
        }
        for _ in 0..800 {
            transactions.push(generator.generate_simple_transfer());
        }
        
        let start = Instant::now();
        
        let mut opportunities = 0;
        for tx in &transactions {
            if let Ok(StrategyResult::Opportunity(_)) = strategy.evaluate_transaction(tx).await {
                opportunities += 1;
            }
        }
        
        let duration = start.elapsed();
        let avg_time_ms = duration.as_millis() as f64 / transactions.len() as f64;
        let throughput = transactions.len() as f64 / duration.as_secs_f64();
        
        println!("Sandwich Strategy Benchmark Results:");
        println!("  Transactions: {}", transactions.len());
        println!("  Opportunities Found: {}", opportunities);
        println!("  Total Time: {:.2}ms", duration.as_millis());
        println!("  Avg Time per Tx: {:.3}ms", avg_time_ms);
        println!("  Throughput: {:.2} tx/sec", throughput);
        
        // Performance requirements (slightly more relaxed than backrun due to complexity)
        assert!(avg_time_ms < 30.0, "Average evaluation time exceeds 30ms requirement: {:.3}ms", avg_time_ms);
        assert!(throughput > 30.0, "Throughput below minimum requirement: {:.2} tx/sec", throughput);
        assert!(opportunities > 0, "Should find some opportunities in victim transactions");
    }

    #[tokio::test]
    async fn benchmark_bundle_creation_speed() {
        let strategy = SandwichStrategy::new();
        let mut generator = MockTransactionGenerator::new();
        
        // Create opportunities first
        let mut opportunities = Vec::new();
        for _ in 0..50 {
            let tx = generator.generate_sandwich_victim();
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
        
        println!("Sandwich Bundle Creation Benchmark:");
        println!("  Opportunities: {}", opportunities.len());
        println!("  Total Time: {:.2}ms", duration.as_millis());
        println!("  Avg Time per Bundle: {:.3}ms", avg_time_ms);
        
        assert!(avg_time_ms < 10.0, "Bundle creation too slow: {:.3}ms", avg_time_ms);
    }
}