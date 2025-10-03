//! Test utilities and mock generators for strategy testing

use crate::strategy_types::*;
use mev_core::{DecodedInput, DecodedParameter, ParsedTransaction, Transaction, TargetType};
use chrono::Utc;
use rand::Rng;
use std::collections::HashMap;

/// Mock transaction generator for testing strategies
pub struct MockTransactionGenerator {
    rng: rand::rngs::ThreadRng,
}

impl MockTransactionGenerator {
    pub fn new() -> Self {
        Self {
            rng: rand::thread_rng(),
        }
    }

    /// Generate a mock swap transaction
    pub fn generate_swap_transaction(
        &mut self,
        target_type: TargetType,
        value_eth: f64,
        gas_price_gwei: u64,
    ) -> ParsedTransaction {
        let value_wei = (value_eth * 1e18) as u128;
        let gas_price_wei = (gas_price_gwei as u128) * 1_000_000_000;
        
        let hash = format!("0x{:064x}", self.rng.gen::<u64>());
        let from = format!("0x{:040x}", self.rng.gen::<u64>() as u128);
        let to = self.get_router_address(&target_type);

        let (function_name, function_sig, calldata) = self.generate_swap_calldata(&target_type, value_wei);

        ParsedTransaction {
            transaction: Transaction {
                hash,
                from,
                to: Some(to),
                value: value_wei.to_string(),
                gas_price: gas_price_wei.to_string(),
                gas_limit: "200000".to_string(),
                nonce: self.rng.gen_range(1..1000),
                input: calldata,
                timestamp: Utc::now(),
            },
            decoded_input: Some(DecodedInput {
                function_name,
                function_signature: function_sig,
                parameters: self.generate_swap_parameters(value_wei),
            }),
            target_type,
            processing_time_ms: self.rng.gen_range(1..10),
        }
    }

    /// Generate a large swap transaction suitable for backrunning
    pub fn generate_large_swap(&mut self, target_type: TargetType) -> ParsedTransaction {
        let value_eth = self.rng.gen_range(1.0..10.0); // 1-10 ETH
        let gas_price_gwei = self.rng.gen_range(30..200); // 30-200 gwei
        self.generate_swap_transaction(target_type, value_eth, gas_price_gwei)
    }

    /// Generate a small swap transaction (not suitable for MEV)
    pub fn generate_small_swap(&mut self, target_type: TargetType) -> ParsedTransaction {
        let value_eth = self.rng.gen_range(0.001..0.1); // 0.001-0.1 ETH
        let gas_price_gwei = self.rng.gen_range(20..50); // 20-50 gwei
        self.generate_swap_transaction(target_type, value_eth, gas_price_gwei)
    }

    /// Generate a sandwich victim transaction
    pub fn generate_sandwich_victim(&mut self) -> ParsedTransaction {
        let value_eth = self.rng.gen_range(0.5..5.0); // 0.5-5 ETH
        let gas_price_gwei = self.rng.gen_range(20..100); // 20-100 gwei
        
        // Victims typically use higher slippage tolerance
        let mut tx = self.generate_swap_transaction(TargetType::UniswapV2, value_eth, gas_price_gwei);
        
        // Add high slippage tolerance to metadata
        if let Some(ref mut decoded) = tx.decoded_input {
            decoded.parameters.push(DecodedParameter {
                name: "amountOutMin".to_string(),
                param_type: "uint256".to_string(),
                value: "0".to_string(), // No slippage protection = sandwich opportunity
            });
        }

        tx
    }

    /// Generate a non-MEV transaction (e.g., simple transfer)
    pub fn generate_simple_transfer(&mut self) -> ParsedTransaction {
        let value_eth = self.rng.gen_range(0.1..2.0);
        let value_wei = (value_eth * 1e18) as u128;
        let gas_price_gwei = self.rng.gen_range(20..100);
        let gas_price_wei = (gas_price_gwei as u128) * 1_000_000_000;

        ParsedTransaction {
            transaction: Transaction {
                hash: format!("0x{:064x}", self.rng.gen::<u64>()),
                from: format!("0x{:040x}", self.rng.gen::<u64>() as u128),
                to: Some(format!("0x{:040x}", self.rng.gen::<u64>() as u128)),
                value: value_wei.to_string(),
                gas_price: gas_price_wei.to_string(),
                gas_limit: "21000".to_string(),
                nonce: self.rng.gen_range(1..1000),
                input: "0x".to_string(),
                timestamp: Utc::now(),
            },
            decoded_input: None,
            target_type: TargetType::Unknown,
            processing_time_ms: 1,
        }
    }

    /// Generate batch of test transactions
    pub fn generate_transaction_batch(&mut self, count: usize) -> Vec<ParsedTransaction> {
        let mut transactions = Vec::new();
        
        for _ in 0..count {
            let tx_type = self.rng.gen_range(0..4);
            let tx = match tx_type {
                0 => self.generate_large_swap(TargetType::UniswapV2),
                1 => self.generate_small_swap(TargetType::UniswapV3),
                2 => self.generate_sandwich_victim(),
                _ => self.generate_simple_transfer(),
            };
            transactions.push(tx);
        }
        
        transactions
    }

    fn get_router_address(&self, target_type: &TargetType) -> String {
        match target_type {
            TargetType::UniswapV2 => "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D".to_string(),
            TargetType::UniswapV3 => "0xE592427A0AEce92De3Edee1F18E0157C05861564".to_string(),
            TargetType::SushiSwap => "0xd9e1cE17f2641f24aE83637ab66a2cca9C378B9F".to_string(),
            TargetType::Curve => "0xbEbc44782C7dB0a1A60Cb6fe97d0b483032FF1C7".to_string(),
            _ => "0x0000000000000000000000000000000000000000".to_string(),
        }
    }

    fn generate_swap_calldata(&mut self, target_type: &TargetType, _value_wei: u128) -> (String, String, String) {
        match target_type {
            TargetType::UniswapV2 | TargetType::SushiSwap => {
                ("swapExactETHForTokens".to_string(), "0x7ff36ab5".to_string(), "0x7ff36ab5".to_string())
            }
            TargetType::UniswapV3 => {
                ("exactInputSingle".to_string(), "0x414bf389".to_string(), "0x414bf389".to_string())
            }
            TargetType::Curve => {
                ("exchange".to_string(), "0x3df02124".to_string(), "0x3df02124".to_string())
            }
            _ => {
                ("transfer".to_string(), "0xa9059cbb".to_string(), "0xa9059cbb".to_string())
            }
        }
    }

    fn generate_swap_parameters(&mut self, value_wei: u128) -> Vec<DecodedParameter> {
        vec![
            DecodedParameter {
                name: "amountIn".to_string(),
                param_type: "uint256".to_string(),
                value: value_wei.to_string(),
            },
            DecodedParameter {
                name: "amountOutMin".to_string(),
                param_type: "uint256".to_string(),
                value: (value_wei / 2).to_string(), // 50% slippage tolerance
            },
            DecodedParameter {
                name: "path".to_string(),
                param_type: "address[]".to_string(),
                value: "[\"0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2\",\"0xA0b86a33E6441b8dB4B2b8b8b8b8b8b8b8b8b8b8\"]".to_string(),
            },
        ]
    }
}

impl Default for MockTransactionGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Mock strategy configuration generator
pub struct MockConfigGenerator;

impl MockConfigGenerator {
    /// Generate test configuration for backrun strategy
    pub fn backrun_config() -> StrategyConfig {
        let mut config = StrategyConfig {
            name: "backrun".to_string(),
            enabled: true,
            priority: 150,
            min_profit_wei: 5_000_000_000_000_000, // 0.005 ETH
            max_gas_price_gwei: 150,
            risk_tolerance: 0.7,
            parameters: HashMap::new(),
        };

        config.parameters.insert("min_target_value_eth".to_string(), serde_json::json!(0.5));
        config.parameters.insert("max_slippage_percent".to_string(), serde_json::json!(2.0));
        config.parameters.insert("target_protocols".to_string(), serde_json::json!(["UniswapV2", "UniswapV3"]));

        config
    }

    /// Generate test configuration for sandwich strategy
    pub fn sandwich_config() -> StrategyConfig {
        let mut config = StrategyConfig {
            name: "sandwich".to_string(),
            enabled: true,
            priority: 200,
            min_profit_wei: 10_000_000_000_000_000, // 0.01 ETH
            max_gas_price_gwei: 200,
            risk_tolerance: 0.5,
            parameters: HashMap::new(),
        };

        config.parameters.insert("max_slippage_tolerance".to_string(), serde_json::json!(5.0));
        config.parameters.insert("min_victim_value_eth".to_string(), serde_json::json!(1.0));
        config.parameters.insert("max_front_run_multiple".to_string(), serde_json::json!(3.0));

        config
    }

    /// Generate disabled strategy configuration
    pub fn disabled_config() -> StrategyConfig {
        StrategyConfig {
            name: "disabled_strategy".to_string(),
            enabled: false,
            priority: 100,
            min_profit_wei: 1_000_000_000_000_000,
            max_gas_price_gwei: 100,
            risk_tolerance: 0.8,
            parameters: HashMap::new(),
        }
    }

    /// Generate high-risk configuration
    pub fn high_risk_config() -> StrategyConfig {
        StrategyConfig {
            name: "high_risk".to_string(),
            enabled: true,
            priority: 255,
            min_profit_wei: 100_000_000_000_000, // 0.0001 ETH
            max_gas_price_gwei: 500,
            risk_tolerance: 0.9,
            parameters: HashMap::new(),
        }
    }

    /// Generate conservative configuration
    pub fn conservative_config() -> StrategyConfig {
        StrategyConfig {
            name: "conservative".to_string(),
            enabled: true,
            priority: 50,
            min_profit_wei: 50_000_000_000_000_000, // 0.05 ETH
            max_gas_price_gwei: 50,
            risk_tolerance: 0.2,
            parameters: HashMap::new(),
        }
    }
}

/// Test scenario builder for complex testing
pub struct TestScenarioBuilder {
    pub transactions: Vec<ParsedTransaction>,
    pub expected_opportunities: usize,
    pub expected_bundles: usize,
    pub scenario_name: String,
}

impl TestScenarioBuilder {
    pub fn new(name: &str) -> Self {
        Self {
            transactions: Vec::new(),
            expected_opportunities: 0,
            expected_bundles: 0,
            scenario_name: name.to_string(),
        }
    }

    /// Add a profitable backrun scenario
    pub fn add_backrun_scenario(mut self) -> Self {
        let mut generator = MockTransactionGenerator::new();
        self.transactions.push(generator.generate_large_swap(TargetType::UniswapV2));
        self.expected_opportunities += 1;
        self.expected_bundles += 1;
        self
    }

    /// Add a sandwich scenario
    pub fn add_sandwich_scenario(mut self) -> Self {
        let mut generator = MockTransactionGenerator::new();
        self.transactions.push(generator.generate_sandwich_victim());
        self.expected_opportunities += 1;
        self.expected_bundles += 1;
        self
    }

    /// Add noise transactions (no MEV opportunities)
    pub fn add_noise_transactions(mut self, count: usize) -> Self {
        let mut generator = MockTransactionGenerator::new();
        for _ in 0..count {
            self.transactions.push(generator.generate_simple_transfer());
        }
        self
    }

    /// Add unprofitable transactions
    pub fn add_unprofitable_transactions(mut self, count: usize) -> Self {
        let mut generator = MockTransactionGenerator::new();
        for _ in 0..count {
            self.transactions.push(generator.generate_small_swap(TargetType::UniswapV2));
        }
        self
    }

    pub fn build(self) -> TestScenario {
        TestScenario {
            name: self.scenario_name,
            transactions: self.transactions,
            expected_opportunities: self.expected_opportunities,
            expected_bundles: self.expected_bundles,
        }
    }
}

/// Complete test scenario
pub struct TestScenario {
    pub name: String,
    pub transactions: Vec<ParsedTransaction>,
    pub expected_opportunities: usize,
    pub expected_bundles: usize,
}

impl TestScenario {
    /// Validate scenario results
    pub fn validate_results(&self, opportunities: usize, bundles: usize) -> bool {
        opportunities == self.expected_opportunities && bundles == self.expected_bundles
    }

    /// Print scenario summary
    pub fn print_summary(&self) {
        println!("Test Scenario: {}", self.name);
        println!("  Transactions: {}", self.transactions.len());
        println!("  Expected Opportunities: {}", self.expected_opportunities);
        println!("  Expected Bundles: {}", self.expected_bundles);
    }
}

/// Performance test utilities
pub struct PerformanceTestUtils;

impl PerformanceTestUtils {
    /// Measure strategy evaluation performance
    pub async fn measure_evaluation_performance<S: Strategy>(
        strategy: &S,
        transactions: &[ParsedTransaction],
    ) -> PerformanceMetrics {
        let start = std::time::Instant::now();
        let mut opportunities = 0;
        let mut errors = 0;

        for tx in transactions {
            match strategy.evaluate_transaction(tx).await {
                Ok(StrategyResult::Opportunity(_)) => opportunities += 1,
                Ok(_) => {},
                Err(_) => errors += 1,
            }
        }

        let duration = start.elapsed();

        PerformanceMetrics {
            total_transactions: transactions.len(),
            opportunities_found: opportunities,
            errors: errors,
            total_time_ms: duration.as_millis() as f64,
            avg_time_per_tx_ms: duration.as_millis() as f64 / transactions.len() as f64,
            throughput_tx_per_sec: transactions.len() as f64 / duration.as_secs_f64(),
        }
    }

    /// Generate load test transactions
    pub fn generate_load_test_transactions(count: usize) -> Vec<ParsedTransaction> {
        let mut generator = MockTransactionGenerator::new();
        generator.generate_transaction_batch(count)
    }
}

/// Performance measurement results
#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    pub total_transactions: usize,
    pub opportunities_found: usize,
    pub errors: usize,
    pub total_time_ms: f64,
    pub avg_time_per_tx_ms: f64,
    pub throughput_tx_per_sec: f64,
}

impl PerformanceMetrics {
    pub fn print_summary(&self) {
        println!("Performance Metrics:");
        println!("  Total Transactions: {}", self.total_transactions);
        println!("  Opportunities Found: {}", self.opportunities_found);
        println!("  Errors: {}", self.errors);
        println!("  Total Time: {:.2}ms", self.total_time_ms);
        println!("  Avg Time per Tx: {:.2}ms", self.avg_time_per_tx_ms);
        println!("  Throughput: {:.2} tx/sec", self.throughput_tx_per_sec);
    }

    /// Check if performance meets requirements
    pub fn meets_requirements(&self, max_avg_time_ms: f64, min_throughput: f64) -> bool {
        self.avg_time_per_tx_ms <= max_avg_time_ms && self.throughput_tx_per_sec >= min_throughput
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_transaction_generator() {
        let mut generator = MockTransactionGenerator::new();
        
        let large_swap = generator.generate_large_swap(TargetType::UniswapV2);
        assert_eq!(large_swap.target_type, TargetType::UniswapV2);
        assert!(large_swap.transaction.value.parse::<u128>().unwrap() >= 1_000_000_000_000_000_000); // >= 1 ETH
        
        let small_swap = generator.generate_small_swap(TargetType::UniswapV3);
        assert_eq!(small_swap.target_type, TargetType::UniswapV3);
        assert!(small_swap.transaction.value.parse::<u128>().unwrap() < 100_000_000_000_000_000); // < 0.1 ETH
    }

    #[test]
    fn test_config_generator() {
        let backrun_config = MockConfigGenerator::backrun_config();
        assert_eq!(backrun_config.name, "backrun");
        assert!(backrun_config.enabled);
        assert!(backrun_config.parameters.contains_key("min_target_value_eth"));

        let disabled_config = MockConfigGenerator::disabled_config();
        assert!(!disabled_config.enabled);
    }

    #[test]
    fn test_scenario_builder() {
        let scenario = TestScenarioBuilder::new("test_scenario")
            .add_backrun_scenario()
            .add_sandwich_scenario()
            .add_noise_transactions(5)
            .build();

        assert_eq!(scenario.name, "test_scenario");
        assert_eq!(scenario.transactions.len(), 7); // 2 MEV + 5 noise
        assert_eq!(scenario.expected_opportunities, 2);
        assert_eq!(scenario.expected_bundles, 2);
    }

    #[test]
    fn test_performance_metrics() {
        let metrics = PerformanceMetrics {
            total_transactions: 1000,
            opportunities_found: 50,
            errors: 2,
            total_time_ms: 500.0,
            avg_time_per_tx_ms: 0.5,
            throughput_tx_per_sec: 2000.0,
        };

        assert!(metrics.meets_requirements(1.0, 1000.0)); // Should pass
        assert!(!metrics.meets_requirements(0.1, 5000.0)); // Should fail
    }
}