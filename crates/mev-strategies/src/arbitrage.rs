//! Arbitrage strategy implementation for MEV bot

use crate::strategy_types::*;
use anyhow::Result;
use mev_core::{ParsedTransaction, SimulationBundle, SimulationResult, TargetType};
use serde_json::json;
use std::collections::HashMap;
use tracing::{debug, info};

/// Arbitrage strategy that finds price differences across DEXs
pub struct ArbitrageStrategy {
    config: StrategyConfig,
    stats: StrategyStats,
}

impl ArbitrageStrategy {
    /// Create new arbitrage strategy
    pub fn new() -> Self {
        let mut config = StrategyConfig {
            name: "arbitrage".to_string(),
            enabled: true,
            priority: 200, // High priority for arbitrage
            min_profit_wei: 10_000_000_000_000_000, // 0.01 ETH
            max_gas_price_gwei: 200,
            risk_tolerance: 0.8,
            parameters: HashMap::new(),
        };

        // Add arbitrage-specific parameters
        config.parameters.insert(
            "min_price_difference_percent".to_string(),
            json!(0.5), // Minimum 0.5% price difference
        );
        config.parameters.insert(
            "max_trade_size_eth".to_string(),
            json!(10.0), // Maximum trade size
        );
        config.parameters.insert(
            "supported_tokens".to_string(),
            json!(["WETH", "USDC", "USDT", "DAI", "WBTC"]),
        );

        Self {
            config,
            stats: StrategyStats::default(),
        }
    }

    /// Check if transaction creates arbitrage opportunity
    fn is_arbitrage_candidate(&self, tx: &ParsedTransaction) -> bool {
        // Look for large swaps that might create price imbalances
        match tx.target_type {
            TargetType::UniswapV2 | TargetType::UniswapV3 | TargetType::SushiSwap => {
                if let Ok(value_wei) = tx.transaction.value.parse::<u128>() {
                    let value_eth = value_wei as f64 / 1e18;
                    
                    // Look for significant trades
                    if value_eth >= 1.0 {
                        debug!(
                            tx_hash = %tx.transaction.hash,
                            value_eth = value_eth,
                            target_type = ?tx.target_type,
                            "Found potential arbitrage trigger"
                        );
                        return true;
                    }
                }
            }
            _ => {}
        }

        false
    }

    /// Find arbitrage opportunities across different DEXs
    fn find_arbitrage_opportunities(&self, tx: &ParsedTransaction) -> Result<Vec<ArbitrageOpportunity>> {
        // This is a simplified implementation
        // Real arbitrage detection would:
        // 1. Parse the swap details from transaction data
        // 2. Query prices across multiple DEXs
        // 3. Calculate optimal arbitrage paths
        // 4. Account for gas costs and slippage

        let mut opportunities = Vec::new();

        // Simulate finding a cross-DEX arbitrage opportunity
        if tx.target_type == TargetType::UniswapV2 {
            // Assume we found a price difference with SushiSwap
            let opportunity = ArbitrageOpportunity {
                token_in: "WETH".to_string(),
                token_out: "USDC".to_string(),
                amount_in: 1_000_000_000_000_000_000, // 1 ETH
                dex_path: vec!["UniswapV2".to_string(), "SushiSwap".to_string()],
                expected_profit: 50_000_000_000_000_000, // 0.05 ETH
                price_difference_percent: 2.5,
            };
            
            opportunities.push(opportunity);
        }

        Ok(opportunities)
    }

    /// Calculate arbitrage profit after costs
    fn calculate_arbitrage_profit(&self, opportunity: &ArbitrageOpportunity) -> Result<u128> {
        // Estimate gas costs for arbitrage transactions
        // Typically need 2 swaps + some overhead
        let gas_cost = strategy_utils::estimate_gas_cost(400_000, 80); // 400k gas at 80 gwei
        
        // Subtract gas costs from expected profit
        Ok(opportunity.expected_profit.saturating_sub(gas_cost))
    }

    /// Create arbitrage transactions
    fn create_arbitrage_transactions(&self, opportunity: &ArbitrageOpportunity) -> Result<Vec<PlannedTransaction>> {
        let mut transactions = Vec::new();

        // First transaction: Buy on cheaper DEX
        let buy_tx = PlannedTransaction {
            transaction_type: TransactionType::Arbitrage,
            to: self.get_dex_router(&opportunity.dex_path[0])?,
            value: opportunity.amount_in,
            gas_limit: 200_000,
            gas_price: self.calculate_competitive_gas_price()?,
            data: self.build_buy_calldata(opportunity)?,
            description: format!("Buy {} on {}", opportunity.token_out, opportunity.dex_path[0]),
        };
        transactions.push(buy_tx);

        // Second transaction: Sell on more expensive DEX
        let sell_tx = PlannedTransaction {
            transaction_type: TransactionType::Arbitrage,
            to: self.get_dex_router(&opportunity.dex_path[1])?,
            value: 0, // No ETH value for token-to-token swap
            gas_limit: 200_000,
            gas_price: self.calculate_competitive_gas_price()?,
            data: self.build_sell_calldata(opportunity)?,
            description: format!("Sell {} on {}", opportunity.token_out, opportunity.dex_path[1]),
        };
        transactions.push(sell_tx);

        Ok(transactions)
    }

    /// Get DEX router address
    fn get_dex_router(&self, dex_name: &str) -> Result<String> {
        let address = match dex_name {
            "UniswapV2" => "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D",
            "UniswapV3" => "0xE592427A0AEce92De3Edee1F18E0157C05861564",
            "SushiSwap" => "0xd9e1cE17f2641f24aE83637ab66a2cca9C378B9F",
            _ => return Err(anyhow::anyhow!("Unsupported DEX: {}", dex_name)),
        };
        
        Ok(address.to_string())
    }

    /// Calculate competitive gas price
    fn calculate_competitive_gas_price(&self) -> Result<u128> {
        // Use a competitive gas price for arbitrage
        let base_gas_price = 60_000_000_000; // 60 gwei
        let max_gas_price = (self.config.max_gas_price_gwei as u128) * 1_000_000_000;
        
        Ok(base_gas_price.min(max_gas_price))
    }

    /// Build buy transaction calldata
    fn build_buy_calldata(&self, _opportunity: &ArbitrageOpportunity) -> Result<String> {
        // Placeholder - real implementation would build proper ABI-encoded calldata
        Ok("0x38ed1739".to_string()) // swapExactTokensForTokens
    }

    /// Build sell transaction calldata
    fn build_sell_calldata(&self, _opportunity: &ArbitrageOpportunity) -> Result<String> {
        // Placeholder - real implementation would build proper ABI-encoded calldata
        Ok("0x38ed1739".to_string()) // swapExactTokensForTokens
    }
}

/// Arbitrage opportunity details
#[derive(Debug, Clone)]
struct ArbitrageOpportunity {
    token_in: String,
    token_out: String,
    amount_in: u128,
    dex_path: Vec<String>,
    expected_profit: u128,
    price_difference_percent: f64,
}

#[async_trait::async_trait]
impl Strategy for ArbitrageStrategy {
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

        // Check if this transaction might create arbitrage opportunities
        if !self.is_arbitrage_candidate(tx) {
            return Ok(StrategyResult::NoOpportunity);
        }

        // Find arbitrage opportunities
        let opportunities = self.find_arbitrage_opportunities(tx)?;
        if opportunities.is_empty() {
            return Ok(StrategyResult::NoOpportunity);
        }

        // Take the best opportunity
        let best_opportunity = opportunities.into_iter()
            .max_by_key(|opp| opp.expected_profit)
            .unwrap();

        // Calculate net profit after gas costs
        let net_profit = self.calculate_arbitrage_profit(&best_opportunity)?;
        if net_profit < self.config.min_profit_wei {
            debug!(
                tx_hash = %tx.transaction.hash,
                net_profit_wei = net_profit,
                min_profit_wei = self.config.min_profit_wei,
                "Arbitrage profit too low"
            );
            return Ok(StrategyResult::NoOpportunity);
        }

        // Calculate confidence based on price difference and market conditions
        let confidence = strategy_utils::calculate_confidence_score(
            best_opportunity.price_difference_percent / 100.0,
            0.9, // Assume good gas price stability
            0.2, // Assume moderate market volatility
        );

        // Create opportunity
        let opportunity = Opportunity {
            id: strategy_utils::generate_opportunity_id(self.name(), &tx.transaction.hash),
            strategy_name: self.name().to_string(),
            target_transaction: tx.clone(),
            opportunity_type: OpportunityType::Arbitrage {
                token_in: best_opportunity.token_in.clone(),
                token_out: best_opportunity.token_out.clone(),
                amount_in: best_opportunity.amount_in,
                expected_profit: net_profit,
                dex_path: best_opportunity.dex_path.clone(),
            },
            estimated_profit_wei: net_profit,
            estimated_gas_cost_wei: strategy_utils::estimate_gas_cost(400_000, 80),
            confidence_score: confidence,
            priority: strategy_utils::calculate_priority_score(
                net_profit,
                confidence,
                self.config.priority,
            ),
            expiry_timestamp: chrono::Utc::now().timestamp() as u64 + 15, // 15 second expiry
            metadata: {
                let mut meta = HashMap::new();
                meta.insert("price_difference_percent".to_string(), best_opportunity.price_difference_percent.to_string());
                meta.insert("dex_path".to_string(), best_opportunity.dex_path.join(" -> "));
                meta
            },
        };

        info!(
            opportunity_id = %opportunity.id,
            token_pair = format!("{}/{}", best_opportunity.token_in, best_opportunity.token_out),
            expected_profit_eth = net_profit as f64 / 1e18,
            price_difference = format!("{:.2}%", best_opportunity.price_difference_percent),
            confidence = confidence,
            "Arbitrage opportunity detected"
        );

        Ok(StrategyResult::Opportunity(opportunity))
    }

    async fn create_bundle_plan(&self, opportunity: &Opportunity) -> Result<BundlePlan> {
        // Extract arbitrage details
        let arb_opportunity = match &opportunity.opportunity_type {
            OpportunityType::Arbitrage { token_in, token_out, amount_in, dex_path, .. } => {
                ArbitrageOpportunity {
                    token_in: token_in.clone(),
                    token_out: token_out.clone(),
                    amount_in: *amount_in,
                    dex_path: dex_path.clone(),
                    expected_profit: opportunity.estimated_profit_wei,
                    price_difference_percent: 0.0, // Not needed for bundle creation
                }
            }
            _ => return Err(anyhow::anyhow!("Invalid opportunity type for arbitrage strategy")),
        };

        // Create arbitrage transactions
        let transactions = self.create_arbitrage_transactions(&arb_opportunity)?;
        
        let bundle_plan = BundlePlan {
            id: format!("bundle_{}", opportunity.id),
            strategy_name: self.name().to_string(),
            opportunity_id: opportunity.id.clone(),
            transactions,
            estimated_gas_total: 400_000, // Two transactions ~200k each
            estimated_profit_wei: opportunity.estimated_profit_wei,
            risk_score: 1.0 - opportunity.confidence_score,
            execution_deadline: opportunity.expiry_timestamp,
        };

        info!(
            bundle_id = %bundle_plan.id,
            opportunity_id = %opportunity.id,
            transaction_count = bundle_plan.transactions.len(),
            "Created arbitrage bundle plan"
        );

        Ok(bundle_plan)
    }

    async fn validate_bundle(&self, _bundle: &SimulationBundle) -> Result<bool> {
        // Simplified validation - real implementation would:
        // 1. Re-check prices across DEXs
        // 2. Verify liquidity is still available
        // 3. Confirm gas prices are still competitive
        // 4. Check that arbitrage is still profitable
        
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
            efficiency = if bundle_plan.estimated_profit_wei > 0 {
                actual_profit as f64 / bundle_plan.estimated_profit_wei as f64
            } else { 0.0 },
            "Arbitrage execution analysis"
        );

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

impl Default for ArbitrageStrategy {
    fn default() -> Self {
        Self::new()
    }
}
