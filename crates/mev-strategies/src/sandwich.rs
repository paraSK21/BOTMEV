//! Sandwich strategy implementation for MEV bot

use crate::strategy_types::*;
use anyhow::Result;
use mev_core::{ParsedTransaction, SimulationBundle, SimulationResult, TargetType};
use serde_json::json;
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// Sandwich strategy that front-runs and back-runs victim transactions
pub struct SandwichStrategy {
    config: StrategyConfig,
    stats: StrategyStats,
}

impl SandwichStrategy {
    /// Create new sandwich strategy
    pub fn new() -> Self {
        let mut config = StrategyConfig {
            name: "sandwich".to_string(),
            enabled: true,
            priority: 200,
            min_profit_wei: 10_000_000_000_000_000, // 0.01 ETH
            max_gas_price_gwei: 200,
            risk_tolerance: 0.5,
            parameters: HashMap::new(),
        };

        // Add sandwich-specific parameters
        config.parameters.insert(
            "max_slippage_tolerance".to_string(),
            json!(5.0), // Maximum victim slippage tolerance to target
        );
        config.parameters.insert(
            "min_victim_value_eth".to_string(),
            json!(1.0), // Minimum victim transaction value
        );
        config.parameters.insert(
            "max_front_run_multiple".to_string(),
            json!(3.0), // Maximum multiple of victim size for front-run
        );
        config.parameters.insert(
            "gas_price_premium_percent".to_string(),
            json!(10.0), // Gas price premium over victim
        );

        Self {
            config,
            stats: StrategyStats::default(),
        }
    }

    /// Check if transaction is suitable for sandwiching
    fn is_sandwich_victim(&self, tx: &ParsedTransaction) -> bool {
        // Check if it's a DEX swap transaction
        match tx.target_type {
            TargetType::UniswapV2 | TargetType::UniswapV3 | TargetType::SushiSwap => {
                // Check transaction value meets minimum
                if let Ok(value_wei) = tx.transaction.value.parse::<u128>() {
                    let min_value_eth = self.config.parameters
                        .get("min_victim_value_eth")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0);
                    let min_value_wei = (min_value_eth * 1e18) as u128;
                    
                    if value_wei >= min_value_wei {
                        // Check if victim has poor slippage protection
                        if self.has_poor_slippage_protection(tx) {
                            debug!(
                                tx_hash = %tx.transaction.hash,
                                value_eth = value_wei as f64 / 1e18,
                                "Found sandwich victim candidate"
                            );
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }

        false
    }

    /// Check if transaction has poor slippage protection (vulnerable to sandwich)
    fn has_poor_slippage_protection(&self, tx: &ParsedTransaction) -> bool {
        // Analyze decoded parameters to check slippage tolerance
        if let Some(ref decoded) = tx.decoded_input {
            for param in &decoded.parameters {
                if param.name == "amountOutMin" {
                    // If amountOutMin is 0 or very low, victim is vulnerable
                    if let Ok(amount_out_min) = param.value.parse::<u128>() {
                        if let Ok(value_wei) = tx.transaction.value.parse::<u128>() {
                            let slippage_tolerance = if value_wei > 0 {
                                1.0 - (amount_out_min as f64 / value_wei as f64)
                            } else {
                                1.0 // Assume high slippage if we can't calculate
                            };
                            
                            let max_slippage = self.config.parameters
                                .get("max_slippage_tolerance")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(5.0) / 100.0;
                            
                            return slippage_tolerance >= max_slippage;
                        }
                    }
                }
            }
        }
        
        // If we can't determine slippage protection, assume it's vulnerable
        // (conservative approach for testing)
        true
    }

    /// Calculate potential sandwich profit
    fn calculate_sandwich_profit(&self, tx: &ParsedTransaction) -> Result<(u128, u128, u128)> {
        let victim_value_wei = tx.transaction.value.parse::<u128>().unwrap_or(0);
        
        // Calculate optimal front-run size (typically 1-3x victim size)
        let max_multiple = self.config.parameters
            .get("max_front_run_multiple")
            .and_then(|v| v.as_f64())
            .unwrap_or(3.0);
        
        let front_run_size = (victim_value_wei as f64 * max_multiple * 0.5) as u128; // Use 50% of max
        
        // Estimate price impact and profit
        let price_impact = self.estimate_price_impact(victim_value_wei + front_run_size, &tx.target_type)?;
        
        // Calculate profit from price movement
        // Simplified: profit = front_run_size * price_impact * efficiency
        let efficiency = match tx.target_type {
            TargetType::UniswapV2 | TargetType::SushiSwap => 0.4, // 40% efficiency
            TargetType::UniswapV3 => 0.5, // 50% efficiency with concentrated liquidity
            _ => 0.3,
        };
        
        let gross_profit = (front_run_size as f64 * price_impact * efficiency) as u128;
        
        // Estimate gas costs (front-run + back-run transactions)
        let gas_cost = strategy_utils::estimate_gas_cost(400_000, 100); // ~400k gas total
        
        let net_profit = gross_profit.saturating_sub(gas_cost);
        
        Ok((front_run_size, gross_profit, net_profit))
    }

    /// Estimate price impact for a given trade size
    fn estimate_price_impact(&self, trade_size_wei: u128, target_type: &TargetType) -> Result<f64> {
        let trade_size_eth = trade_size_wei as f64 / 1e18;
        
        // Simplified price impact model
        let impact = match target_type {
            TargetType::UniswapV2 | TargetType::SushiSwap => {
                // V2 AMMs: impact = k * sqrt(trade_size)
                (trade_size_eth * 0.005).min(0.15) // 0.5% per ETH, max 15%
            }
            TargetType::UniswapV3 => {
                // V3 concentrated liquidity: lower impact but more complex
                (trade_size_eth * 0.003).min(0.10) // 0.3% per ETH, max 10%
            }
            _ => (trade_size_eth * 0.002).min(0.05), // Conservative estimate
        };
        
        Ok(impact)
    }

    /// Create sandwich bundle (front-run + victim + back-run)
    fn create_sandwich_transactions(&self, opportunity: &Opportunity) -> Result<Vec<PlannedTransaction>> {
        let (front_run_size, back_run_size) = match &opportunity.opportunity_type {
            OpportunityType::Sandwich { front_run_amount, back_run_amount, .. } => {
                (*front_run_amount, *back_run_amount)
            }
            _ => return Err(anyhow::anyhow!("Invalid opportunity type for sandwich strategy")),
        };

        let victim_gas_price = opportunity.target_transaction.transaction.gas_price.parse::<u128>().unwrap_or(20_000_000_000);
        let gas_premium_percent = self.config.parameters
            .get("gas_price_premium_percent")
            .and_then(|v| v.as_f64())
            .unwrap_or(10.0) / 100.0;
        
        let our_gas_price = (victim_gas_price as f64 * (1.0 + gas_premium_percent)) as u128;
        let router_address = self.get_router_address(&opportunity.target_transaction.target_type)?;

        // Front-run transaction (higher gas price)
        let front_run_tx = PlannedTransaction {
            transaction_type: TransactionType::FrontRun,
            to: router_address.clone(),
            value: front_run_size,
            gas_limit: 200_000,
            gas_price: our_gas_price + 1_000_000_000, // +1 gwei over our base
            data: self.build_swap_calldata(front_run_size, true)?,
            description: "Sandwich front-run transaction".to_string(),
        };

        // Back-run transaction (same gas price as front-run)
        let back_run_tx = PlannedTransaction {
            transaction_type: TransactionType::BackRun,
            to: router_address,
            value: 0, // Usually selling tokens back
            gas_limit: 200_000,
            gas_price: our_gas_price,
            data: self.build_swap_calldata(back_run_size, false)?,
            description: "Sandwich back-run transaction".to_string(),
        };

        Ok(vec![front_run_tx, back_run_tx])
    }

    /// Get router address for the target protocol
    fn get_router_address(&self, target_type: &TargetType) -> Result<String> {
        let address = match target_type {
            TargetType::UniswapV2 => "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D",
            TargetType::UniswapV3 => "0xE592427A0AEce92De3Edee1F18E0157C05861564",
            TargetType::SushiSwap => "0xd9e1cE17f2641f24aE83637ab66a2cca9C378B9F",
            _ => return Err(anyhow::anyhow!("Unsupported target type for sandwich")),
        };
        
        Ok(address.to_string())
    }

    /// Build swap calldata for sandwich transactions
    fn build_swap_calldata(&self, _amount: u128, _is_front_run: bool) -> Result<String> {
        // Placeholder - real implementation would build proper ABI-encoded calldata
        Ok("0x38ed1739".to_string()) // swapExactTokensForTokens selector
    }

    /// Validate sandwich opportunity against risk parameters
    fn validate_sandwich_opportunity(&self, opportunity: &Opportunity) -> bool {
        // Check profit meets minimum threshold
        if opportunity.estimated_profit_wei < self.config.min_profit_wei {
            return false;
        }

        // Check confidence score meets risk tolerance
        if opportunity.confidence_score < self.config.risk_tolerance {
            return false;
        }

        // Additional sandwich-specific validations
        match &opportunity.opportunity_type {
            OpportunityType::Sandwich { front_run_amount, expected_profit, .. } => {
                // Ensure front-run size is reasonable
                let victim_value = opportunity.target_transaction.transaction.value.parse::<u128>().unwrap_or(0);
                let max_multiple = self.config.parameters
                    .get("max_front_run_multiple")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(3.0);
                
                if *front_run_amount > (victim_value as f64 * max_multiple) as u128 {
                    return false;
                }

                // Ensure expected profit is reasonable
                if *expected_profit != opportunity.estimated_profit_wei {
                    warn!("Profit mismatch in sandwich opportunity validation");
                }

                true
            }
            _ => false,
        }
    }
}

#[async_trait::async_trait]
impl Strategy for SandwichStrategy {
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

        // Check if this transaction is a sandwich victim
        if !self.is_sandwich_victim(tx) {
            return Ok(StrategyResult::NoOpportunity);
        }

        // Calculate sandwich profit potential
        let (front_run_size, gross_profit, net_profit) = self.calculate_sandwich_profit(tx)?;
        
        if net_profit < self.config.min_profit_wei {
            debug!(
                tx_hash = %tx.transaction.hash,
                net_profit_wei = net_profit,
                min_profit_wei = self.config.min_profit_wei,
                "Sandwich profit too low"
            );
            return Ok(StrategyResult::NoOpportunity);
        }

        // Calculate confidence score based on slippage tolerance and market conditions
        let confidence = strategy_utils::calculate_confidence_score(
            net_profit as f64 / gross_profit as f64,
            0.7, // Assume reasonable gas price stability
            0.3, // Moderate market volatility for sandwich attacks
        );

        // Create sandwich opportunity
        let opportunity = Opportunity {
            id: strategy_utils::generate_opportunity_id(self.name(), &tx.transaction.hash),
            strategy_name: self.name().to_string(),
            target_transaction: tx.clone(),
            opportunity_type: OpportunityType::Sandwich {
                victim_tx_hash: tx.transaction.hash.clone(),
                token_pair: ("ETH".to_string(), "TOKEN".to_string()), // Simplified
                front_run_amount: front_run_size,
                back_run_amount: front_run_size, // Simplified: same size
                expected_profit: net_profit,
            },
            estimated_profit_wei: net_profit,
            estimated_gas_cost_wei: strategy_utils::estimate_gas_cost(400_000, 100),
            confidence_score: confidence,
            priority: strategy_utils::calculate_priority_score(
                net_profit,
                confidence,
                self.config.priority,
            ),
            expiry_timestamp: chrono::Utc::now().timestamp() as u64 + 15, // 15 second expiry (tight timing)
            metadata: {
                let mut meta = HashMap::new();
                meta.insert("front_run_size_eth".to_string(), (front_run_size as f64 / 1e18).to_string());
                meta.insert("victim_value_eth".to_string(), (tx.transaction.value.parse::<u128>().unwrap_or(0) as f64 / 1e18).to_string());
                meta.insert("target_protocol".to_string(), format!("{:?}", tx.target_type));
                meta
            },
        };

        // Validate opportunity
        if !self.validate_sandwich_opportunity(&opportunity) {
            return Ok(StrategyResult::NoOpportunity);
        }

        info!(
            opportunity_id = %opportunity.id,
            victim_tx = %tx.transaction.hash,
            estimated_profit_eth = net_profit as f64 / 1e18,
            confidence = confidence,
            "Sandwich opportunity detected"
        );

        Ok(StrategyResult::Opportunity(opportunity))
    }

    async fn create_bundle_plan(&self, opportunity: &Opportunity) -> Result<BundlePlan> {
        // Create sandwich transactions
        let transactions = self.create_sandwich_transactions(opportunity)?;
        
        let bundle_plan = BundlePlan {
            id: format!("sandwich_{}", opportunity.id),
            strategy_name: self.name().to_string(),
            opportunity_id: opportunity.id.clone(),
            transactions,
            estimated_gas_total: 400_000, // Front-run + back-run
            estimated_profit_wei: opportunity.estimated_profit_wei,
            risk_score: 1.0 - opportunity.confidence_score, // Higher confidence = lower risk
            execution_deadline: opportunity.expiry_timestamp,
        };

        info!(
            bundle_id = %bundle_plan.id,
            opportunity_id = %opportunity.id,
            transaction_count = bundle_plan.transactions.len(),
            "Created sandwich bundle plan"
        );

        Ok(bundle_plan)
    }

    async fn validate_bundle(&self, _bundle: &SimulationBundle) -> Result<bool> {
        // Sandwich-specific validation
        // Real implementation would:
        // 1. Check victim transaction is still in mempool
        // 2. Verify our gas prices are competitive
        // 3. Confirm liquidity is still available
        // 4. Validate slippage calculations
        // 5. Check for competing sandwich attempts
        
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
            "Sandwich execution analysis"
        );

        // Additional sandwich-specific analysis
        if success {
            let profit_accuracy = if bundle_plan.estimated_profit_wei > 0 {
                actual_profit as f64 / bundle_plan.estimated_profit_wei as f64
            } else {
                0.0
            };
            
            debug!(
                bundle_id = %bundle_plan.id,
                profit_accuracy = profit_accuracy,
                "Sandwich profit prediction accuracy"
            );
        }

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

impl Default for SandwichStrategy {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
