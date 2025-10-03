//! Strategy system testing tool

use anyhow::Result;
use mev_core::{ParsedTransaction, PrometheusMetrics, TargetType, Transaction, DecodedInput};
use mev_strategies::{ArbitrageStrategy, BackrunStrategy, Strategy, StrategyEngine, StrategyEngineConfig};
use std::{sync::Arc, time::Instant};
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Strategy test scenarios
#[derive(Debug, Clone)]
enum StrategyTestScenario {
    SingleStrategyTest { strategy_name: String },
    MultiStrategyComparison,
    PerformanceTest { transaction_count: usize },
    OpportunityLifecycle,
}

/// Strategy system tester
struct StrategyTester {
    engine: StrategyEngine,
}

impl StrategyTester {
    async fn new() -> Result<Self> {
        let metrics = Arc::new(PrometheusMetrics::new()?);
        let config = StrategyEngineConfig {
            max_concurrent_evaluations: 5,
            evaluation_timeout_ms: 100,
            opportunity_cache_size: 100,
            bundle_cache_ttl_seconds: 60,
            enable_parallel_evaluation: true,
        };

        let engine = StrategyEngine::new(config, metrics);

        // Register strategies
        let arbitrage_strategy = Box::new(ArbitrageStrategy::new());
        let backrun_strategy = Box::new(BackrunStrategy::new());

        engine.register_strategy(arbitrage_strategy).await?;
        engine.register_strategy(backrun_strategy).await?;

        // Start cleanup task
        engine.start_cleanup_task().await;

        Ok(Self { engine })
    }

    /// Run all test scenarios
    async fn run_all_tests(&self) -> Result<()> {
        info!("Starting comprehensive strategy system tests");

        let scenarios = vec![
            StrategyTestScenario::SingleStrategyTest {
                strategy_name: "arbitrage".to_string(),
            },
            StrategyTestScenario::SingleStrategyTest {
                strategy_name: "backrun".to_string(),
            },
            StrategyTestScenario::MultiStrategyComparison,
            StrategyTestScenario::PerformanceTest { transaction_count: 100 },
            StrategyTestScenario::OpportunityLifecycle,
        ];

        for scenario in scenarios {
            info!("Running test scenario: {:?}", scenario);
            
            if let Err(e) = self.run_scenario(scenario).await {
                warn!("Test scenario failed: {}", e);
            }
            
            // Brief pause between tests
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }

        info!("All strategy tests completed");
        Ok(())
    }

    /// Run a specific test scenario
    async fn run_scenario(&self, scenario: StrategyTestScenario) -> Result<()> {
        match scenario {
            StrategyTestScenario::SingleStrategyTest { strategy_name } => {
                self.test_single_strategy(&strategy_name).await
            }
            StrategyTestScenario::MultiStrategyComparison => {
                self.test_multi_strategy_comparison().await
            }
            StrategyTestScenario::PerformanceTest { transaction_count } => {
                self.test_performance(transaction_count).await
            }
            StrategyTestScenario::OpportunityLifecycle => {
                self.test_opportunity_lifecycle().await
            }
        }
    }

    /// Test a single strategy
    async fn test_single_strategy(&self, strategy_name: &str) -> Result<()> {
        info!(strategy = %strategy_name, "Testing single strategy");

        // Create test transactions
        let test_transactions = vec![
            // Large Uniswap V2 swap (good for both arbitrage and backrun)
            self.create_test_transaction(
                "large_uniswap_swap",
                2.0,
                TargetType::UniswapV2,
                Some("0x38ed1739"), // swapExactTokensForTokens
            ),
            
            // Medium Uniswap V3 swap
            self.create_test_transaction(
                "medium_uniswap_v3",
                1.0,
                TargetType::UniswapV3,
                Some("0x414bf389"), // exactInputSingle
            ),
            
            // Small transaction (should not trigger strategies)
            self.create_test_transaction(
                "small_transaction",
                0.1,
                TargetType::UniswapV2,
                Some("0x38ed1739"),
            ),
            
            // Non-DEX transaction (should not trigger strategies)
            self.create_test_transaction(
                "non_dex_transaction",
                1.0,
                TargetType::Unknown,
                None,
            ),
        ];

        let mut total_opportunities = 0;
        let mut strategy_opportunities = 0;

        for tx in test_transactions {
            let opportunities = self.engine.evaluate_transaction(&tx).await?;
            total_opportunities += opportunities.len();
            
            // Count opportunities from our target strategy
            let strategy_opps = opportunities
                .iter()
                .filter(|opp| opp.strategy_name == strategy_name)
                .count();
            strategy_opportunities += strategy_opps;

            if strategy_opps > 0 {
                info!(
                    tx_hash = %tx.transaction.hash,
                    strategy = %strategy_name,
                    opportunities = strategy_opps,
                    "Strategy found opportunities"
                );
            }
        }

        // Get strategy statistics
        if let Some(stats) = self.engine.get_strategy_stats(strategy_name).await {
            info!(
                strategy = %strategy_name,
                total_opportunities = total_opportunities,
                strategy_opportunities = strategy_opportunities,
                stats = ?stats,
                "Single strategy test completed"
            );
        }

        Ok(())
    }

    /// Test multiple strategies and compare their performance
    async fn test_multi_strategy_comparison(&self) -> Result<()> {
        info!("Testing multi-strategy comparison");

        // Create a high-value transaction that should trigger multiple strategies
        let high_value_tx = self.create_test_transaction(
            "high_value_comparison",
            5.0, // 5 ETH
            TargetType::UniswapV2,
            Some("0x38ed1739"),
        );

        let start_time = Instant::now();
        let opportunities = self.engine.evaluate_transaction(&high_value_tx).await?;
        let evaluation_time = start_time.elapsed();

        info!(
            tx_hash = %high_value_tx.transaction.hash,
            total_opportunities = opportunities.len(),
            evaluation_time_ms = evaluation_time.as_millis(),
            "Multi-strategy evaluation completed"
        );

        // Analyze opportunities by strategy
        let mut strategy_breakdown = std::collections::HashMap::new();
        for opportunity in &opportunities {
            *strategy_breakdown.entry(opportunity.strategy_name.clone()).or_insert(0) += 1;
        }

        for (strategy, count) in strategy_breakdown {
            info!(
                strategy = %strategy,
                opportunities = count,
                "Strategy performance in comparison"
            );
        }

        // Test bundle creation for the best opportunity
        if let Some(best_opportunity) = opportunities
            .iter()
            .max_by_key(|opp| opp.estimated_profit_wei)
        {
            info!(
                opportunity_id = %best_opportunity.id,
                strategy = %best_opportunity.strategy_name,
                estimated_profit_eth = best_opportunity.estimated_profit_wei as f64 / 1e18,
                "Testing bundle creation for best opportunity"
            );

            match self.engine.create_bundle_plan(&best_opportunity.id).await? {
                Some(bundle_plan) => {
                    info!(
                        bundle_id = %bundle_plan.id,
                        transaction_count = bundle_plan.transactions.len(),
                        estimated_gas = bundle_plan.estimated_gas_total,
                        "Bundle plan created successfully"
                    );
                }
                None => {
                    warn!("Failed to create bundle plan for best opportunity");
                }
            }
        }

        Ok(())
    }

    /// Test strategy performance under load
    async fn test_performance(&self, transaction_count: usize) -> Result<()> {
        info!(transaction_count = transaction_count, "Testing strategy performance");

        let start_time = Instant::now();
        let mut total_opportunities = 0;
        let mut total_evaluation_time = tokio::time::Duration::ZERO;

        // Generate and evaluate transactions
        for i in 0..transaction_count {
            let tx = self.create_random_transaction(i);
            
            let eval_start = Instant::now();
            let opportunities = self.engine.evaluate_transaction(&tx).await?;
            let eval_time = eval_start.elapsed();
            
            total_opportunities += opportunities.len();
            total_evaluation_time += eval_time;

            // Progress reporting
            if i % 20 == 0 && i > 0 {
                let elapsed = start_time.elapsed().as_secs_f64();
                let tps = i as f64 / elapsed;
                info!(
                    processed = i,
                    transactions_per_second = format!("{:.1}", tps),
                    avg_eval_time_ms = format!("{:.2}", total_evaluation_time.as_secs_f64() * 1000.0 / i as f64),
                    "Performance test progress"
                );
            }
        }

        let total_time = start_time.elapsed();
        let transactions_per_second = transaction_count as f64 / total_time.as_secs_f64();
        let avg_evaluation_time_ms = total_evaluation_time.as_secs_f64() * 1000.0 / transaction_count as f64;

        // Get engine statistics
        let engine_stats = self.engine.get_engine_stats().await;

        info!(
            total_transactions = transaction_count,
            total_opportunities = total_opportunities,
            total_time_ms = total_time.as_millis(),
            transactions_per_second = format!("{:.1}", transactions_per_second),
            avg_evaluation_time_ms = format!("{:.2}", avg_evaluation_time_ms),
            engine_stats = ?engine_stats,
            "Performance test completed"
        );

        // Performance assertions
        if transactions_per_second < 50.0 {
            warn!("Strategy performance below target (50 TPS): {:.1}", transactions_per_second);
        } else {
            info!("Strategy performance meets target: {:.1} TPS", transactions_per_second);
        }

        if avg_evaluation_time_ms > 100.0 {
            warn!("Average evaluation time above target (100ms): {:.2}ms", avg_evaluation_time_ms);
        } else {
            info!("Average evaluation time meets target: {:.2}ms", avg_evaluation_time_ms);
        }

        Ok(())
    }

    /// Test complete opportunity lifecycle
    async fn test_opportunity_lifecycle(&self) -> Result<()> {
        info!("Testing opportunity lifecycle");

        // Create a transaction that should generate opportunities
        let tx = self.create_test_transaction(
            "lifecycle_test",
            3.0,
            TargetType::UniswapV2,
            Some("0x38ed1739"),
        );

        // Step 1: Evaluate transaction
        let opportunities = self.engine.evaluate_transaction(&tx).await?;
        info!(opportunities_found = opportunities.len(), "Step 1: Transaction evaluated");

        if opportunities.is_empty() {
            warn!("No opportunities found for lifecycle test");
            return Ok(());
        }

        // Step 2: Get active opportunities
        let active_opportunities = self.engine.get_active_opportunities().await;
        info!(active_opportunities = active_opportunities.len(), "Step 2: Active opportunities retrieved");

        // Step 3: Create bundle plan
        let opportunity = &opportunities[0];
        let bundle_plan = self.engine.create_bundle_plan(&opportunity.id).await?;
        
        match bundle_plan {
            Some(plan) => {
                info!(
                    bundle_id = %plan.id,
                    transactions = plan.transactions.len(),
                    "Step 3: Bundle plan created"
                );

                // Step 4: Retrieve bundle plan
                let retrieved_plan = self.engine.get_bundle_plan(&plan.id).await;
                match retrieved_plan {
                    Some(_) => info!("Step 4: Bundle plan retrieved successfully"),
                    None => warn!("Step 4: Failed to retrieve bundle plan"),
                }
            }
            None => {
                warn!("Step 3: Failed to create bundle plan");
            }
        }

        // Step 5: Wait for expiry and test cleanup
        info!("Step 5: Testing opportunity expiry and cleanup");
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        
        // Force cleanup
        self.engine.cleanup_expired().await;
        
        let active_after_cleanup = self.engine.get_active_opportunities().await;
        info!(
            active_after_cleanup = active_after_cleanup.len(),
            "Step 5: Cleanup completed"
        );

        info!("Opportunity lifecycle test completed");
        Ok(())
    }

    /// Create a test transaction with specific parameters
    fn create_test_transaction(
        &self,
        name: &str,
        value_eth: f64,
        target_type: TargetType,
        function_sig: Option<&str>,
    ) -> ParsedTransaction {
        let value_wei = (value_eth * 1e18) as u128;

        let decoded_input = function_sig.map(|sig| DecodedInput {
            function_name: match sig {
                "0x38ed1739" => "swapExactTokensForTokens".to_string(),
                "0x414bf389" => "exactInputSingle".to_string(),
                _ => "unknown".to_string(),
            },
            function_signature: sig.to_string(),
            parameters: vec![],
        });

        ParsedTransaction {
            transaction: Transaction {
                hash: format!("0x{}", name),
                from: "0x1234567890123456789012345678901234567890".to_string(),
                to: Some("0x0987654321098765432109876543210987654321".to_string()),
                value: value_wei.to_string(),
                gas_price: "50000000000".to_string(), // 50 gwei
                gas_limit: "200000".to_string(),
                nonce: 1,
                input: function_sig.unwrap_or("0x").to_string(),
                timestamp: chrono::Utc::now(),
            },
            decoded_input,
            target_type,
            processing_time_ms: 1,
        }
    }

    /// Create a random transaction for performance testing
    fn create_random_transaction(&self, id: usize) -> ParsedTransaction {
        use rand::Rng;
        let mut rng = rand::thread_rng();

        let value_eth = rng.gen_range(0.1..5.0);
        let target_types = vec![
            TargetType::UniswapV2,
            TargetType::UniswapV3,
            TargetType::SushiSwap,
            TargetType::Unknown,
        ];
        let target_type = target_types[rng.gen_range(0..target_types.len())].clone();

        let function_sigs = vec![
            Some("0x38ed1739"),
            Some("0x414bf389"),
            Some("0x8803dbee"),
            None,
        ];
        let function_sig = function_sigs[rng.gen_range(0..function_sigs.len())];

        self.create_test_transaction(
            &format!("random_{}", id),
            value_eth,
            target_type,
            function_sig,
        )
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("MEV Bot Strategy System Test Tool");

    let tester = StrategyTester::new().await?;
    tester.run_all_tests().await?;

    info!("Strategy system tests completed");
    Ok(())
}