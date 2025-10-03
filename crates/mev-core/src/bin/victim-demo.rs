//! Reproducible demo script for end-to-end MEV bot testing

use anyhow::Result;
use mev_core::{
    BundleBuilder, BundleSubmitter, ExecutionTracker, KeyManager, SubmissionConfig, 
    TrackerConfig, VictimGenerator, VictimGeneratorConfig, TestScenario, SubmissionPath
};
use mev_core::bundle::victim_generator::ExpectedOpportunity;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn, error};

/// Demo configuration
#[derive(Debug, Clone)]
struct DemoConfig {
    /// RPC endpoint for blockchain interaction
    pub rpc_endpoint: String,
    /// Private key for MEV bot
    pub mev_bot_private_key: String,
    /// Private key for victim transactions
    pub victim_private_key: String,
    /// Demo duration in seconds
    pub demo_duration: u64,
    /// Enable detailed logging
    pub verbose: bool,
}

impl Default for DemoConfig {
    fn default() -> Self {
        Self {
            rpc_endpoint: "http://localhost:8545".to_string(),
            mev_bot_private_key: "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318".to_string(),
            victim_private_key: "0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba".to_string(),
            demo_duration: 120, // 2 minutes
            verbose: true,
        }
    }
}

/// MEV bot demo orchestrator
struct MevBotDemo {
    config: DemoConfig,
    key_manager: Arc<KeyManager>,
    bundle_builder: BundleBuilder,
    bundle_submitter: BundleSubmitter,
    execution_tracker: ExecutionTracker,
    victim_generator: VictimGenerator,
}

impl MevBotDemo {
    /// Create a new demo instance
    async fn new(config: DemoConfig) -> Result<Self> {
        // Setup key management
        let key_manager = Arc::new(KeyManager::new());
        key_manager.load_private_key(&config.mev_bot_private_key).await?;
        
        // Setup bundle system
        let signer = Arc::new(mev_core::TransactionSigner::new(key_manager.clone(), 998));
        let mut bundle_builder = BundleBuilder::new(signer);
        
        // Register strategy builders
        bundle_builder.register_strategy_builder(
            "arbitrage".to_string(),
            Box::new(mev_core::ArbitrageBundleBuilder),
        );
        bundle_builder.register_strategy_builder(
            "sandwich".to_string(),
            Box::new(mev_core::SandwichBundleBuilder),
        );
        
        // Setup submission system
        let submission_config = SubmissionConfig {
            rpc_endpoint: config.rpc_endpoint.clone(),
            bundle_rpc_endpoint: None, // Direct submission only for demo
            max_resubmissions: 2,
            gas_bump_multiplier: 1.2,
            submission_timeout: Duration::from_secs(30),
            resubmission_interval: Duration::from_secs(5),
        };
        let bundle_submitter = BundleSubmitter::new(submission_config);
        
        // Setup execution tracking
        let tracker_config = TrackerConfig {
            rpc_endpoint: config.rpc_endpoint.clone(),
            confirmation_blocks: 1, // Fast confirmation for demo
            max_tracking_duration: Duration::from_secs(60),
            polling_interval: Duration::from_secs(2),
            detailed_analysis: true,
        };
        let execution_tracker = ExecutionTracker::new(tracker_config);
        
        // Setup victim generator
        let victim_key_manager = Arc::new(KeyManager::new());
        victim_key_manager.load_private_key(&config.victim_private_key).await?;
        
        let victim_config = VictimGeneratorConfig {
            rpc_endpoint: config.rpc_endpoint.clone(),
            victim_private_key: config.victim_private_key.clone(),
            ..Default::default()
        };
        let victim_generator = VictimGenerator::new(victim_config, victim_key_manager)?;
        
        Ok(Self {
            config,
            key_manager,
            bundle_builder,
            bundle_submitter,
            execution_tracker,
            victim_generator,
        })
    }
    
    /// Run the complete demo
    async fn run_demo(&self) -> Result<()> {
        info!("🚀 Starting MEV Bot Demo");
        info!("Configuration: {:?}", self.config);
        
        // Phase 1: Deploy victim infrastructure
        info!("📦 Phase 1: Deploying victim infrastructure");
        self.deploy_infrastructure().await?;
        
        // Phase 2: Run test scenarios
        info!("🎯 Phase 2: Running test scenarios");
        self.run_test_scenarios().await?;
        
        // Phase 3: Performance analysis
        info!("📊 Phase 3: Performance analysis");
        self.analyze_performance().await?;
        
        info!("✅ Demo completed successfully");
        Ok(())
    }
    
    /// Deploy victim infrastructure
    async fn deploy_infrastructure(&self) -> Result<()> {
        let deployment = mev_core::VictimDeployment {
            network: "testnet".to_string(),
            token_count: 3,
            initial_supply: ethers::types::U256::from(1000000000000000000000u64), // 1000 tokens
            setup_liquidity: true,
            initial_liquidity: ethers::types::U256::from(100000000000000000000u64), // 100 tokens
        };
        
        let deployed_tokens = self.victim_generator.deploy_victim_infrastructure(&deployment).await?;
        
        info!("Deployed {} dummy tokens:", deployed_tokens.len());
        for (name, address) in deployed_tokens {
            info!("  {} -> {:?}", name, address);
        }
        
        // Wait for deployment to settle
        sleep(Duration::from_secs(2)).await;
        
        Ok(())
    }
    
    /// Run multiple test scenarios
    async fn run_test_scenarios(&self) -> Result<()> {
        let scenarios = vec![
            "simple_arbitrage",
            "sandwich_attack",
            "multi_dex_arbitrage",
        ];
        
        for scenario_name in scenarios {
            info!("Running scenario: {}", scenario_name);
            
            let scenario = self.victim_generator.create_test_scenario(scenario_name);
            let results = self.execute_scenario(&scenario).await?;
            
            info!("Scenario '{}' results:", scenario_name);
            info!("  Execution time: {:?}", results.execution_time);
            info!("  Opportunities detected: {}", results.opportunities_detected);
            info!("  Total profit: {} wei", results.total_profit);
            info!("  Ordering success rate: {:.2}%", results.ordering_success_rate * 100.0);
            info!("  Success: {}", if results.success { "✅" } else { "❌" });
            
            // Wait between scenarios
            sleep(Duration::from_secs(5)).await;
        }
        
        Ok(())
    }
    
    /// Execute a single test scenario
    async fn execute_scenario(&self, scenario: &TestScenario) -> Result<mev_core::TestResults> {
        info!("Executing scenario: {}", scenario.name);
        info!("Description: {}", scenario.description);
        
        // Start execution tracking
        let tracker_handle = {
            let tracker = Arc::new(&self.execution_tracker);
            tokio::spawn(async move {
                if let Err(e) = tracker.start_monitoring().await {
                    error!("Execution tracker error: {}", e);
                }
            })
        };
        
        // Execute the scenario
        let results = self.victim_generator.execute_test_scenario(scenario).await?;
        
        // Simulate MEV bot detection and bundle creation
        for expected_opp in &scenario.expected_opportunities {
            info!("Simulating MEV opportunity detection for: {}", expected_opp.target_victim_id);
            
            // In a real implementation, this would be triggered by mempool monitoring
            let simulated_opportunity = self.create_simulated_opportunity(expected_opp).await?;
            
            // Build bundle
            let bundle = self.bundle_builder.build_bundle(&simulated_opportunity).await?;
            info!("Built bundle: {} with {} transactions", bundle.id, bundle.all_transactions().len());
            
            // Submit bundle
            let submission_result = self.bundle_submitter
                .submit_bundle(&bundle, SubmissionPath::Direct)
                .await?;
            info!("Submitted bundle: {}", submission_result.submission_hash);
            
            // Track execution
            self.execution_tracker.track_bundle(bundle, submission_result).await?;
        }
        
        // Wait for execution to complete
        sleep(Duration::from_secs(10)).await;
        
        // Stop tracking
        tracker_handle.abort();
        
        Ok(results)
    }
    
    /// Create a simulated MEV opportunity for testing
    async fn create_simulated_opportunity(
        &self,
        expected: &ExpectedOpportunity,
    ) -> Result<mev_core::Opportunity> {
        use mev_core::{Opportunity, OpportunityType, ParsedTransaction, Transaction, TargetType};
        use std::collections::HashMap;
        
        let target_tx = ParsedTransaction {
            transaction: Transaction {
                hash: format!("0x{}", expected.target_victim_id),
                from: "0x742d35Cc6634C0532925a3b8D4C9db96C4b5Da5e".to_string(),
                to: Some("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D".to_string()), // Uniswap router
                value: "0".to_string(),
                gas_price: "50000000000".to_string(),
                gas_limit: "200000".to_string(),
                nonce: 1,
                input: "0xa9059cbb".to_string(), // transfer function
                timestamp: chrono::Utc::now(),
            },
            decoded_input: None,
            target_type: TargetType::UniswapV2,
            processing_time_ms: 1,
        };
        
        let opportunity_type = match expected.opportunity_type.as_str() {
            "arbitrage" => OpportunityType::Arbitrage {
                token_in: "DUMMY_A".to_string(),
                token_out: "DUMMY_B".to_string(),
                amount_in: 1000000000000000000,
                expected_profit: expected.expected_profit_range.0.as_u128(),
                dex_path: vec!["UniswapV2".to_string(), "SushiSwap".to_string()],
            },
            "sandwich" => OpportunityType::Sandwich {
                victim_tx_hash: expected.target_victim_id.clone(),
                token_pair: ("DUMMY_A".to_string(), "DUMMY_B".to_string()),
                front_run_amount: 500000000000000000,
                back_run_amount: 500000000000000000,
                expected_profit: expected.expected_profit_range.0.as_u128(),
            },
            _ => OpportunityType::Arbitrage {
                token_in: "DUMMY_A".to_string(),
                token_out: "DUMMY_B".to_string(),
                amount_in: 1000000000000000000,
                expected_profit: expected.expected_profit_range.0.as_u128(),
                dex_path: vec!["UniswapV2".to_string()],
            },
        };
        
        Ok(Opportunity {
            id: format!("opp_{}", uuid::Uuid::new_v4()),
            strategy_name: expected.opportunity_type.clone(),
            target_transaction: target_tx,
            opportunity_type,
            estimated_profit_wei: expected.expected_profit_range.0.as_u128(),
            estimated_gas_cost_wei: expected.expected_gas_cost.as_u128(),
            confidence_score: expected.min_confidence,
            priority: 150,
            expiry_timestamp: chrono::Utc::now().timestamp() as u64 + 30,
            metadata: HashMap::new(),
        })
    }
    
    /// Analyze performance metrics
    async fn analyze_performance(&self) -> Result<()> {
        info!("Analyzing MEV bot performance...");
        
        // Get submission statistics
        let submission_stats = self.bundle_submitter.get_stats().await;
        info!("Submission Statistics:");
        info!("  Total submissions: {}", submission_stats.total_submissions);
        info!("  Gas bumps: {}", submission_stats.gas_bump_count);
        
        // Get tracking statistics
        let tracking_stats = self.execution_tracker.get_tracking_stats().await;
        info!("Execution Tracking Statistics:");
        info!("  Total tracked: {}", tracking_stats.total_tracked);
        info!("  Pending: {}", tracking_stats.pending);
        info!("  Included: {}", tracking_stats.included);
        info!("  Failed: {}", tracking_stats.failed);
        info!("  Expired: {}", tracking_stats.expired);
        
        // Calculate success rate
        let total_completed = tracking_stats.included + tracking_stats.failed + tracking_stats.expired;
        if total_completed > 0 {
            let success_rate = tracking_stats.included as f64 / total_completed as f64;
            info!("  Success rate: {:.2}%", success_rate * 100.0);
        }
        
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info,mev_core=debug")
        .init();
    
    // Parse command line arguments
    let config = DemoConfig::default();
    
    info!("🤖 MEV Bot End-to-End Demo");
    info!("==========================");
    
    // Create and run demo
    let demo = MevBotDemo::new(config).await?;
    demo.run_demo().await?;
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_demo_creation() {
        let config = DemoConfig::default();
        let demo = MevBotDemo::new(config).await;
        assert!(demo.is_ok());
    }
    
    #[test]
    fn test_demo_config() {
        let config = DemoConfig::default();
        assert_eq!(config.demo_duration, 120);
        assert!(config.verbose);
    }
}