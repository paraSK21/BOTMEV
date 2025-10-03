//! Basic strategy types for the decision path
//! 
//! This module provides the minimal strategy interfaces needed for the decision path
//! without creating circular dependencies.

use crate::types::ParsedTransaction;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// MEV opportunity detected by a strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opportunity {
    pub id: String,
    pub strategy_name: String,
    pub target_transaction: ParsedTransaction,
    pub opportunity_type: OpportunityType,
    pub estimated_profit_wei: u128,
    pub estimated_gas_cost_wei: u128,
    pub confidence_score: f64, // 0.0 to 1.0
    pub priority: u8,          // 0-255, higher = more urgent
    pub expiry_timestamp: u64,
    pub metadata: HashMap<String, String>,
}

/// Types of MEV opportunities
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OpportunityType {
    Arbitrage {
        token_in: String,
        token_out: String,
        amount_in: u128,
        expected_profit: u128,
        dex_path: Vec<String>,
    },
    Backrun {
        target_tx_hash: String,
        affected_token: String,
        price_impact: f64,
        follow_up_action: String,
    },
    Sandwich {
        victim_tx_hash: String,
        token_pair: (String, String),
        front_run_amount: u128,
        back_run_amount: u128,
        expected_profit: u128,
    },
    Liquidation {
        protocol: String,
        position_owner: String,
        collateral_token: String,
        debt_token: String,
        liquidation_amount: u128,
        liquidation_bonus: f64,
    },
}

/// Strategy evaluation result
#[derive(Debug, Clone)]
pub enum StrategyResult {
    Opportunity(Opportunity),
    NoOpportunity,
    Error(String),
}

/// Basic strategy trait for decision path integration
#[async_trait::async_trait]
pub trait Strategy: Send + Sync {
    /// Get strategy name
    fn name(&self) -> &str;

    /// Evaluate a transaction for MEV opportunities
    async fn evaluate_transaction(&self, tx: &ParsedTransaction) -> Result<StrategyResult>;

    /// Check if strategy is enabled
    fn is_enabled(&self) -> bool {
        true
    }
}

/// Mock strategy engine for decision path testing
pub struct MockStrategyEngine {
    strategies: Vec<Box<dyn Strategy>>,
}

impl MockStrategyEngine {
    pub fn new() -> Self {
        Self {
            strategies: Vec::new(),
        }
    }

    pub async fn register_strategy(&mut self, strategy: Box<dyn Strategy>) -> Result<()> {
        self.strategies.push(strategy);
        Ok(())
    }

    pub async fn evaluate_transaction(&self, tx: &ParsedTransaction) -> Result<Vec<Opportunity>> {
        let mut opportunities = Vec::new();

        for strategy in &self.strategies {
            if !strategy.is_enabled() {
                continue;
            }

            match strategy.evaluate_transaction(tx).await {
                Ok(StrategyResult::Opportunity(opp)) => {
                    opportunities.push(opp);
                }
                Ok(StrategyResult::NoOpportunity) => {
                    // Continue to next strategy
                }
                Ok(StrategyResult::Error(e)) => {
                    tracing::warn!(
                        strategy = strategy.name(),
                        error = %e,
                        "Strategy evaluation error"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        strategy = strategy.name(),
                        error = %e,
                        "Strategy evaluation failed"
                    );
                }
            }
        }

        Ok(opportunities)
    }
}

impl Default for MockStrategyEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Mock arbitrage strategy for testing
pub struct MockArbitrageStrategy {
    name: String,
    enabled: bool,
}

impl MockArbitrageStrategy {
    pub fn new() -> Self {
        Self {
            name: "mock_arbitrage".to_string(),
            enabled: true,
        }
    }
}

#[async_trait::async_trait]
impl Strategy for MockArbitrageStrategy {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn evaluate_transaction(&self, tx: &ParsedTransaction) -> Result<StrategyResult> {
        // Mock evaluation logic
        match tx.target_type {
            crate::types::TargetType::UniswapV2 | crate::types::TargetType::UniswapV3 => {
                // Simulate finding an arbitrage opportunity 20% of the time
                if rand::random::<f64>() < 0.2 {
                    let opportunity = Opportunity {
                        id: format!("arb_{}", uuid::Uuid::new_v4()),
                        strategy_name: self.name.clone(),
                        target_transaction: tx.clone(),
                        opportunity_type: OpportunityType::Arbitrage {
                            token_in: "WETH".to_string(),
                            token_out: "USDC".to_string(),
                            amount_in: 1000000000000000000, // 1 ETH
                            expected_profit: 50000000000000000, // 0.05 ETH
                            dex_path: vec!["UniswapV2".to_string(), "SushiSwap".to_string()],
                        },
                        estimated_profit_wei: 50000000000000000,
                        estimated_gas_cost_wei: 5000000000000000,
                        confidence_score: 0.75,
                        priority: 150,
                        expiry_timestamp: chrono::Utc::now().timestamp() as u64 + 30,
                        metadata: HashMap::new(),
                    };
                    Ok(StrategyResult::Opportunity(opportunity))
                } else {
                    Ok(StrategyResult::NoOpportunity)
                }
            }
            _ => Ok(StrategyResult::NoOpportunity),
        }
    }
}

/// Mock backrun strategy for testing
pub struct MockBackrunStrategy {
    name: String,
    enabled: bool,
}

impl MockBackrunStrategy {
    pub fn new() -> Self {
        Self {
            name: "mock_backrun".to_string(),
            enabled: true,
        }
    }
}

#[async_trait::async_trait]
impl Strategy for MockBackrunStrategy {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn evaluate_transaction(&self, tx: &ParsedTransaction) -> Result<StrategyResult> {
        // Mock evaluation logic
        match tx.target_type {
            crate::types::TargetType::UniswapV2 | 
            crate::types::TargetType::SushiSwap |
            crate::types::TargetType::Curve => {
                // Check transaction value to determine if it's worth backrunning
                if let Ok(value) = tx.transaction.value.parse::<u128>() {
                    if value > 5000000000000000000 { // > 5 ETH
                        let opportunity = Opportunity {
                            id: format!("backrun_{}", uuid::Uuid::new_v4()),
                            strategy_name: self.name.clone(),
                            target_transaction: tx.clone(),
                            opportunity_type: OpportunityType::Backrun {
                                target_tx_hash: tx.transaction.hash.clone(),
                                affected_token: "WETH".to_string(),
                                price_impact: 0.02, // 2% price impact
                                follow_up_action: "arbitrage".to_string(),
                            },
                            estimated_profit_wei: 25000000000000000, // 0.025 ETH
                            estimated_gas_cost_wei: 3000000000000000, // 0.003 ETH
                            confidence_score: 0.85,
                            priority: 180,
                            expiry_timestamp: chrono::Utc::now().timestamp() as u64 + 15,
                            metadata: HashMap::new(),
                        };
                        Ok(StrategyResult::Opportunity(opportunity))
                    } else {
                        Ok(StrategyResult::NoOpportunity)
                    }
                } else {
                    Ok(StrategyResult::NoOpportunity)
                }
            }
            _ => Ok(StrategyResult::NoOpportunity),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Transaction, TargetType};

    fn create_test_transaction() -> ParsedTransaction {
        ParsedTransaction {
            transaction: Transaction {
                hash: "0x1234567890abcdef".to_string(),
                from: "0xfrom".to_string(),
                to: Some("0xto".to_string()),
                value: "6000000000000000000".to_string(), // 6 ETH
                gas_price: "50000000000".to_string(),
                gas_limit: "200000".to_string(),
                nonce: 1,
                input: "0x38ed1739".to_string(),
                timestamp: chrono::Utc::now(),
            },
            decoded_input: None,
            target_type: TargetType::UniswapV2,
            processing_time_ms: 1,
        }
    }

    #[tokio::test]
    async fn test_mock_strategy_engine() {
        let mut engine = MockStrategyEngine::new();
        
        engine.register_strategy(Box::new(MockArbitrageStrategy::new())).await.unwrap();
        engine.register_strategy(Box::new(MockBackrunStrategy::new())).await.unwrap();

        let tx = create_test_transaction();
        let opportunities = engine.evaluate_transaction(&tx).await.unwrap();
        
        // Should find at least the backrun opportunity for large transactions
        assert!(!opportunities.is_empty());
    }

    #[tokio::test]
    async fn test_mock_backrun_strategy() {
        let strategy = MockBackrunStrategy::new();
        let tx = create_test_transaction();
        
        let result = strategy.evaluate_transaction(&tx).await.unwrap();
        
        match result {
            StrategyResult::Opportunity(opp) => {
                assert_eq!(opp.strategy_name, "mock_backrun");
                assert!(matches!(opp.opportunity_type, OpportunityType::Backrun { .. }));
            }
            _ => panic!("Expected opportunity for large transaction"),
        }
    }
}