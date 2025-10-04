//! Backrun strategy implementation for MEV bot

use crate::strategy_types::*;
use anyhow::Result;
use mev_core::{ParsedTransaction, SimulationBundle, SimulationResult, TargetType};
use serde_json::json;
use std::collections::HashMap;
use tracing::{debug, info};

/// Backrun strategy that follows large transactions to capture price movements
pub struct BackrunStrategy {
    config: StrategyConfig,
    stats: StrategyStats,
}

impl BackrunStrategy {
    /// Create new backrun strategy
    pub fn new() -> Self {
        let mut config = StrategyConfig {
            name: "backrun".to_string(),
            enabled: true,
            priority: 150,
            min_profit_wei: 5_000_000_000_000_000, // 0.005 ETH
            max_gas_price_gwei: 150,
            risk_tolerance: 0.7,
            parameters: HashMap::new(),
        };

        // Add backrun-specific parameters
        config.parameters.insert(
            "min_target_value_eth".to_string(),
            json!(0.5), // Minimum target transaction value
        );
        config.parameters.insert(
            "max_slippage_percent".to_string(),
            json!(2.0), // Maximum acceptable slippage
        );
        config.parameters.insert(
            "target_protocols".to_string(),
            json!(["UniswapV2", "UniswapV3", "SushiSwap"]),
        );

        Self {
            config,
            stats: StrategyStats::default(),
        }
    }

    /// Check if transaction is suitable for backrunning
    fn is_backrun_candidate(&self, tx: &ParsedTransaction) -> bool {
        // Check if it's a DEX transaction
        match tx.target_type {
            TargetType::UniswapV2 | TargetType::UniswapV3 | TargetType::SushiSwap | TargetType::Curve => {
                // Check transaction value
                if let Ok(value_wei) = tx.transaction.value.parse::<u128>() {
                    let min_value_eth = self.config.parameters
                        .get("min_target_value_eth")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.5);
                    let min_value_wei = (min_value_eth * 1e18) as u128;
                    
                    if value_wei >= min_value_wei {
                        debug!(
                            tx_hash = %tx.transaction.hash,
                            value_eth = value_wei as f64 / 1e18,
                            target_type = ?tx.target_type,
                            "Found backrun candidate"
                        );
                        return true;
                    }
                }
            }
            _ => {}
        }

        false
    }

    /// Analyze the price impact of a transaction
    fn analyze_price_impact(&self, tx: &ParsedTransaction) -> Result<f64> {
        // Simplified price impact calculation
        // In a real implementation, this would:
        // 1. Identify the token pair being traded
        // 2. Calculate the pool reserves
        // 3. Estimate the price impact based on trade size
        // 4. Consider the AMM's bonding curve

        let value_wei = tx.transaction.value.parse::<u128>().unwrap_or(0);
        let value_eth = value_wei as f64 / 1e18;

        // Simple heuristic: larger trades have higher impact
        let estimated_impact = match tx.target_type {
            TargetType::UniswapV2 | TargetType::SushiSwap => {
                // V2 AMMs have higher slippage for large trades
                (value_eth * 0.003).min(0.05) // 0.3% per ETH, max 5%
            }
            TargetType::UniswapV3 => {
                // V3 has concentrated liquidity, potentially lower slippage
                (value_eth * 0.002).min(0.03) // 0.2% per ETH, max 3%
            }
            TargetType::Curve => {
                // Curve is optimized for stablecoins, very low slippage
                (value_eth * 0.001).min(0.01) // 0.1% per ETH, max 1%
            }
            _ => 0.0,
        };

        Ok(estimated_impact)
    }

    /// Calculate potential profit from backrunning
    fn calculate_backrun_profit(&self, tx: &ParsedTransaction, price_impact: f64) -> Result<u128> {
        // Simplified profit calculation
        // Real implementation would:
        // 1. Simulate the target transaction
        // 2. Calculate new pool state
        // 3. Find optimal arbitrage amount
        // 4. Simulate our backrun transaction
        // 5. Calculate net profit after gas

        let value_wei = tx.transaction.value.parse::<u128>().unwrap_or(0);
        
        // Estimate profit as a fraction of the price impact
        // This is a simplified heuristic
        let profit_rate = match tx.target_type {
            TargetType::UniswapV2 | TargetType::SushiSwap => price_impact * 0.3, // Capture 30% of impact
            TargetType::UniswapV3 => price_impact * 0.4, // Better efficiency with V3
            TargetType::Curve => price_impact * 0.5, // High efficiency with stablecoins
            _ => 0.0,
        };

        let estimated_profit_wei = (value_wei as f64 * profit_rate) as u128;
        
        // Subtract estimated gas costs
        let gas_cost_wei = strategy_utils::estimate_gas_cost(150_000, 50); // ~150k gas at 50 gwei
        
        Ok(estimated_profit_wei.saturating_sub(gas_cost_wei))
    }

    /// Create backrun transaction data
    fn create_backrun_transaction(&self, opportunity: &Opportunity) -> Result<PlannedTransaction> {
        // Extract opportunity details
        let (affected_token, follow_up_action) = match &opportunity.opportunity_type {
            OpportunityType::Backrun { affected_token, follow_up_action, .. } => {
                (affected_token.clone(), follow_up_action.clone())
            }
            _ => return Err(anyhow::anyhow!("Invalid opportunity type for backrun strategy")),
        };

        // Create the backrun transaction
        // This is simplified - real implementation would construct proper calldata
        let transaction = PlannedTransaction {
            transaction_type: TransactionType::BackRun,
            to: self.get_router_address(&opportunity.target_transaction.target_type)?,
            value: 0, // Usually no ETH value for token swaps
            gas_limit: 200_000, // Conservative gas limit
            gas_price: self.calculate_optimal_gas_price(&opportunity.target_transaction)?,
            data: self.build_swap_calldata(&affected_token, opportunity.estimated_profit_wei)?,
            description: format!("Backrun {} transaction", follow_up_action),
        };

        Ok(transaction)
    }

    /// Get router address for the target protocol
    fn get_router_address(&self, target_type: &TargetType) -> Result<String> {
        let address = match target_type {
            TargetType::UniswapV2 => "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D", // Uniswap V2 Router
            TargetType::UniswapV3 => "0xE592427A0AEce92De3Edee1F18E0157C05861564", // Uniswap V3 Router
            TargetType::SushiSwap => "0xd9e1cE17f2641f24aE83637ab66a2cca9C378B9F", // SushiSwap Router
            _ => return Err(anyhow::anyhow!("Unsupported target type for backrun")),
        };
        
        Ok(address.to_string())
    }

    /// Calculate optimal gas price for backrun transaction
    fn calculate_optimal_gas_price(&self, target_tx: &ParsedTransaction) -> Result<u128> {
        // Parse target transaction gas price
        let target_gas_price = target_tx.transaction.gas_price.parse::<u128>().unwrap_or(20_000_000_000);
        
        // Add small premium to ensure we're included in the same block
        let premium = target_gas_price / 20; // 5% premium
        let our_gas_price = target_gas_price + premium;
        
        // Cap at our maximum
        let max_gas_price = (self.config.max_gas_price_gwei as u128) * 1_000_000_000;
        
        Ok(our_gas_price.min(max_gas_price))
    }

    /// Build swap calldata for backrun transaction
    fn build_swap_calldata(&self, _token: &str, _amount: u128) -> Result<String> {
        // This is a placeholder - real implementation would:
        // 1. Determine optimal swap path
        // 2. Calculate exact amounts
        // 3. Build proper ABI-encoded calldata
        // 4. Include slippage protection
        
        // For now, return a placeholder
        Ok("0x38ed1739".to_string()) // swapExactTokensForTokens selector
    }
}

#[async_trait::async_trait]
impl Strategy for BackrunStrategy {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn config(&self) -> &StrategyConfig {
        &self.config
    }

    fn update_config(&mut self, config: StrategyConfig) -> Result<()> {
        self.config = config;
        info!(strategy = self.name(), "Configuration updated");
        Ok(())
    }

    async fn evaluate_transaction(&self, tx: &ParsedTransaction) -> Result<StrategyResult> {
        if !self.config.enabled {
            return Ok(StrategyResult::NoOpportunity);
        }

        // Check if this transaction is a backrun candidate
        if !self.is_backrun_candidate(tx) {
            return Ok(StrategyResult::NoOpportunity);
        }

        // Analyze price impact
        let price_impact = self.analyze_price_impact(tx)?;
        if price_impact < 0.001 { // Less than 0.1% impact
            debug!(
                tx_hash = %tx.transaction.hash,
                price_impact = price_impact,
                "Price impact too low for backrun"
            );
            return Ok(StrategyResult::NoOpportunity);
        }

        // Calculate potential profit
        let estimated_profit = self.calculate_backrun_profit(tx, price_impact)?;
        if estimated_profit < self.config.min_profit_wei {
            debug!(
                tx_hash = %tx.transaction.hash,
                estimated_profit_wei = estimated_profit,
                min_profit_wei = self.config.min_profit_wei,
                "Profit too low for backrun"
            );
            return Ok(StrategyResult::NoOpportunity);
        }

        // Calculate confidence score
        let confidence = strategy_utils::calculate_confidence_score(
            estimated_profit as f64 / tx.transaction.value.parse::<u128>().unwrap_or(1) as f64,
            0.8, // Assume reasonable gas price stability
            price_impact, // Higher volatility = lower confidence
        );

        // Create opportunity
        let opportunity = Opportunity {
            id: strategy_utils::generate_opportunity_id(self.name(), &tx.transaction.hash),
            strategy_name: self.name().to_string(),
            target_transaction: tx.clone(),
            opportunity_type: OpportunityType::Backrun {
                target_tx_hash: tx.transaction.hash.clone(),
                affected_token: "UNKNOWN".to_string(), // Would be parsed from transaction
                price_impact,
                follow_up_action: "arbitrage_swap".to_string(),
            },
            estimated_profit_wei: estimated_profit,
            estimated_gas_cost_wei: strategy_utils::estimate_gas_cost(200_000, 50),
            confidence_score: confidence,
            priority: strategy_utils::calculate_priority_score(
                estimated_profit,
                confidence,
                self.config.priority,
            ),
            expiry_timestamp: chrono::Utc::now().timestamp() as u64 + 30, // 30 second expiry
            metadata: {
                let mut meta = HashMap::new();
                meta.insert("price_impact".to_string(), price_impact.to_string());
                meta.insert("target_protocol".to_string(), format!("{:?}", tx.target_type));
                meta
            },
        };

        info!(
            opportunity_id = %opportunity.id,
            target_tx = %tx.transaction.hash,
            estimated_profit_eth = estimated_profit as f64 / 1e18,
            confidence = confidence,
            "Backrun opportunity detected"
        );

        Ok(StrategyResult::Opportunity(opportunity))
    }

    async fn create_bundle_plan(&self, opportunity: &Opportunity) -> Result<BundlePlan> {
        // Create the backrun transaction
        let backrun_tx = self.create_backrun_transaction(opportunity)?;
        
        let bundle_plan = BundlePlan {
            id: format!("bundle_{}", opportunity.id),
            strategy_name: self.name().to_string(),
            opportunity_id: opportunity.id.clone(),
            transactions: vec![backrun_tx],
            estimated_gas_total: 200_000, // Conservative estimate
            estimated_profit_wei: opportunity.estimated_profit_wei,
            risk_score: 1.0 - opportunity.confidence_score, // Higher confidence = lower risk
            execution_deadline: opportunity.expiry_timestamp,
        };

        info!(
            bundle_id = %bundle_plan.id,
            opportunity_id = %opportunity.id,
            transaction_count = bundle_plan.transactions.len(),
            "Created backrun bundle plan"
        );

        Ok(bundle_plan)
    }

    async fn validate_bundle(&self, _bundle: &SimulationBundle) -> Result<bool> {
        // Simplified validation - real implementation would:
        // 1. Check that target transaction is still in mempool
        // 2. Verify gas prices are still competitive
        // 3. Confirm liquidity is still available
        // 4. Validate slippage bounds
        
        Ok(true)
    }

    async fn analyze_execution(&self, bundle_plan: &BundlePlan, result: &SimulationResult) -> Result<()> {
        let success = result.success && result.profit_estimate.net_profit_wei > ethers::types::U256::zero();
        let actual_profit = result.profit_estimate.net_profit_wei.as_u128();
        let gas_cost = result.gas_cost.as_u128();

        info!(
            bundle_id = %bundle_plan.id,
            success = success,
            estimated_profit_wei = bundle_plan.estimated_profit_wei,
            actual_profit_wei = actual_profit,
            gas_cost_wei = gas_cost,
            "Backrun execution analysis"
        );

        // Update strategy statistics would happen here in a mutable context
        // self.stats.record_execution(success, actual_profit, gas_cost);

        Ok(())
    }

    fn get_stats(&self) -> StrategyStats {
        self.stats.clone()
    }

    fn reset_stats(&mut self) {
        self.stats = StrategyStats::default();
        info!(strategy = self.name(), "Statistics reset");
    }
}

impl Default for BackrunStrategy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod basic_tests {
    use super::*;
    use mev_core::{Transaction, DecodedInput};

    fn create_test_transaction(value_eth: f64, target_type: TargetType) -> ParsedTransaction {
        let value_wei = (value_eth * 1e18) as u128;
        
        ParsedTransaction {
            transaction: Transaction {
                hash: "0x1234567890abcdef".to_string(),
                from: "0xfrom".to_string(),
                to: Some("0xto".to_string()),
                value: value_wei.to_string(),
                gas_price: "50000000000".to_string(), // 50 gwei
                gas_limit: "200000".to_string(),
                nonce: 1,
                input: "0x38ed1739".to_string(),
                timestamp: chrono::Utc::now(),
            },
            decoded_input: Some(DecodedInput {
                function_name: "swapExactTokensForTokens".to_string(),
                function_signature: "0x38ed1739".to_string(),
                parameters: vec![],
            }),
            target_type,
            processing_time_ms: 1,
        }
    }

    #[test]
    fn test_backrun_candidate_detection() {
        let strategy = BackrunStrategy::new();
        
        // Large Uniswap transaction should be a candidate
        let large_uniswap_tx = create_test_transaction(2.0, TargetType::UniswapV2);
        assert!(strategy.is_backrun_candidate(&large_uniswap_tx));
        
        // Small transaction should not be a candidate
        let small_tx = create_test_transaction(0.1, TargetType::UniswapV2);
        assert!(!strategy.is_backrun_candidate(&small_tx));
        
        // Non-DEX transaction should not be a candidate
        let non_dex_tx = create_test_transaction(2.0, TargetType::Unknown);
        assert!(!strategy.is_backrun_candidate(&non_dex_tx));
    }

    #[test]
    fn test_price_impact_analysis() {
        let strategy = BackrunStrategy::new();
        
        let tx = create_test_transaction(1.0, TargetType::UniswapV2);
        let impact = strategy.analyze_price_impact(&tx).unwrap();
        
        assert!(impact > 0.0);
        assert!(impact < 0.1); // Should be reasonable
    }

    #[tokio::test]
    async fn test_opportunity_evaluation() {
        let strategy = BackrunStrategy::new();
        
        // Test with a good candidate
        let good_tx = create_test_transaction(2.0, TargetType::UniswapV2);
        let result = strategy.evaluate_transaction(&good_tx).await.unwrap();
        
        match result {
            StrategyResult::Opportunity(opp) => {
                assert_eq!(opp.strategy_name, "backrun");
                assert!(opp.estimated_profit_wei > 0);
                assert!(opp.confidence_score > 0.0);
            }
            _ => panic!("Expected opportunity for good transaction"),
        }
        
        // Test with a poor candidate
        let poor_tx = create_test_transaction(0.01, TargetType::Unknown);
        let result = strategy.evaluate_transaction(&poor_tx).await.unwrap();
        
        match result {
            StrategyResult::NoOpportunity => {}, // Expected
            _ => panic!("Expected no opportunity for poor transaction"),
        }
    }
}