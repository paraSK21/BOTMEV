//! Strategy engine for coordinating multiple MEV strategies

use crate::strategy_types::*;
use anyhow::Result;
use mev_core::{ParsedTransaction, PrometheusMetrics, SimulationBundle, SimulationResult};
use std::{
    collections::HashMap,
    sync::Arc,
    time::Instant,
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Strategy engine configuration
#[derive(Debug, Clone)]
pub struct StrategyEngineConfig {
    pub max_concurrent_evaluations: usize,
    pub evaluation_timeout_ms: u64,
    pub opportunity_cache_size: usize,
    pub bundle_cache_ttl_seconds: u64,
    pub enable_parallel_evaluation: bool,
}

impl Default for StrategyEngineConfig {
    fn default() -> Self {
        Self {
            max_concurrent_evaluations: 10,
            evaluation_timeout_ms: 50,
            opportunity_cache_size: 1000,
            bundle_cache_ttl_seconds: 30,
            enable_parallel_evaluation: true,
        }
    }
}

/// Strategy engine that coordinates multiple MEV strategies
pub struct StrategyEngine {
    config: StrategyEngineConfig,
    strategies: Arc<RwLock<HashMap<String, Box<dyn Strategy>>>>,
    opportunities: Arc<RwLock<HashMap<String, Opportunity>>>,
    bundle_plans: Arc<RwLock<HashMap<String, BundlePlan>>>,
    metrics: Arc<PrometheusMetrics>,
    stats: Arc<RwLock<StrategyEngineStats>>,
}

/// Strategy engine statistics
#[derive(Debug, Clone, Default)]
pub struct StrategyEngineStats {
    pub transactions_evaluated: u64,
    pub opportunities_found: u64,
    pub bundles_created: u64,
    pub total_evaluation_time_ms: f64,
    pub average_evaluation_time_ms: f64,
    pub strategy_performance: HashMap<String, StrategyStats>,
}

impl StrategyEngine {
    /// Create new strategy engine
    pub fn new(config: StrategyEngineConfig, metrics: Arc<PrometheusMetrics>) -> Self {
        Self {
            config,
            strategies: Arc::new(RwLock::new(HashMap::new())),
            opportunities: Arc::new(RwLock::new(HashMap::new())),
            bundle_plans: Arc::new(RwLock::new(HashMap::new())),
            metrics,
            stats: Arc::new(RwLock::new(StrategyEngineStats::default())),
        }
    }

    /// Register a strategy with the engine
    pub async fn register_strategy(&self, strategy: Box<dyn Strategy>) -> Result<()> {
        let name = strategy.name().to_string();
        let mut strategies = self.strategies.write().await;
        strategies.insert(name.clone(), strategy);
        
        info!(strategy_name = %name, "Strategy registered with engine");
        Ok(())
    }

    /// Remove a strategy from the engine
    pub async fn unregister_strategy(&self, name: &str) -> Result<()> {
        let mut strategies = self.strategies.write().await;
        if strategies.remove(name).is_some() {
            info!(strategy_name = %name, "Strategy unregistered from engine");
            Ok(())
        } else {
            Err(anyhow::anyhow!("Strategy '{}' not found", name))
        }
    }

    /// Evaluate a transaction across all enabled strategies
    pub async fn evaluate_transaction(&self, tx: &ParsedTransaction) -> Result<Vec<Opportunity>> {
        let start_time = Instant::now();
        let mut opportunities = Vec::new();

        // Update stats
        {
            let mut stats = self.stats.write().await;
            stats.transactions_evaluated += 1;
        }

        // Get enabled strategies
        let strategies = self.strategies.read().await;
        let enabled_strategies: Vec<_> = strategies
            .values()
            .filter(|s| s.config().enabled)
            .collect();

        if enabled_strategies.is_empty() {
            debug!("No enabled strategies for transaction evaluation");
            return Ok(opportunities);
        }

        debug!(
            tx_hash = %tx.transaction.hash,
            strategy_count = enabled_strategies.len(),
            "Evaluating transaction across strategies"
        );

        // Evaluate strategies in parallel or sequentially based on config
        if self.config.enable_parallel_evaluation {
            opportunities = self.evaluate_strategies_parallel(&enabled_strategies, tx).await?;
        } else {
            opportunities = self.evaluate_strategies_sequential(&enabled_strategies, tx).await?;
        }

        // Cache opportunities
        self.cache_opportunities(&opportunities).await;

        // Update metrics
        let evaluation_time = start_time.elapsed().as_secs_f64() * 1000.0;
        {
            let mut stats = self.stats.write().await;
            stats.opportunities_found += opportunities.len() as u64;
            stats.total_evaluation_time_ms += evaluation_time;
            stats.average_evaluation_time_ms = 
                stats.total_evaluation_time_ms / stats.transactions_evaluated as f64;
        }

        // Record metrics
        self.metrics.record_decision_latency(
            start_time.elapsed(),
            "strategy_evaluation",
            if opportunities.is_empty() { "no_opportunity" } else { "opportunity_found" },
        );

        info!(
            tx_hash = %tx.transaction.hash,
            opportunities_found = opportunities.len(),
            evaluation_time_ms = format!("{:.2}", evaluation_time),
            "Transaction evaluation completed"
        );

        Ok(opportunities)
    }

    /// Evaluate strategies in parallel
    async fn evaluate_strategies_parallel(
        &self,
        strategies: &[&Box<dyn Strategy>],
        tx: &ParsedTransaction,
    ) -> Result<Vec<Opportunity>> {
        let mut handles = Vec::new();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.max_concurrent_evaluations));

        // Create evaluation tasks
        for strategy in strategies {
            let permit = semaphore.clone().acquire_owned().await?;
            let strategy_name = strategy.name().to_string();
            let tx_clone = tx.clone();
            let timeout = self.config.evaluation_timeout_ms;

            // Note: In a real implementation, you'd need to handle the strategy reference properly
            // This is a simplified version that shows the pattern
            let handle = tokio::spawn(async move {
                let _permit = permit; // Hold permit until task completes
                
                // Simulate strategy evaluation with timeout
                let result = tokio::time::timeout(
                    tokio::time::Duration::from_millis(timeout),
                    async {
                        // In real implementation, call strategy.evaluate_transaction(&tx_clone)
                        // For now, return no opportunity
                        Ok::<StrategyResult, anyhow::Error>(StrategyResult::NoOpportunity)
                    }
                ).await;

                match result {
                    Ok(Ok(StrategyResult::Opportunity(opp))) => Some(opp),
                    Ok(Ok(_)) => None,
                    Ok(Err(e)) => {
                        warn!(strategy = %strategy_name, error = %e, "Strategy evaluation failed");
                        None
                    }
                    Err(_) => {
                        warn!(strategy = %strategy_name, "Strategy evaluation timed out");
                        None
                    }
                }
            });

            handles.push(handle);
        }

        // Collect results
        let mut opportunities = Vec::new();
        for handle in handles {
            if let Ok(Some(opportunity)) = handle.await {
                opportunities.push(opportunity);
            }
        }

        Ok(opportunities)
    }

    /// Evaluate strategies sequentially
    async fn evaluate_strategies_sequential(
        &self,
        strategies: &[&Box<dyn Strategy>],
        tx: &ParsedTransaction,
    ) -> Result<Vec<Opportunity>> {
        let mut opportunities = Vec::new();

        for strategy in strategies {
            let start_time = Instant::now();
            
            match tokio::time::timeout(
                tokio::time::Duration::from_millis(self.config.evaluation_timeout_ms),
                strategy.evaluate_transaction(tx),
            ).await {
                Ok(Ok(StrategyResult::Opportunity(opp))) => {
                    debug!(
                        strategy = %strategy.name(),
                        opportunity_id = %opp.id,
                        evaluation_time_ms = start_time.elapsed().as_millis(),
                        "Strategy found opportunity"
                    );
                    opportunities.push(opp);
                }
                Ok(Ok(_)) => {
                    debug!(
                        strategy = %strategy.name(),
                        evaluation_time_ms = start_time.elapsed().as_millis(),
                        "Strategy found no opportunity"
                    );
                }
                Ok(Err(e)) => {
                    warn!(
                        strategy = %strategy.name(),
                        error = %e,
                        "Strategy evaluation failed"
                    );
                }
                Err(_) => {
                    warn!(
                        strategy = %strategy.name(),
                        timeout_ms = self.config.evaluation_timeout_ms,
                        "Strategy evaluation timed out"
                    );
                }
            }
        }

        Ok(opportunities)
    }

    /// Cache opportunities for later retrieval
    async fn cache_opportunities(&self, opportunities: &[Opportunity]) {
        let mut cache = self.opportunities.write().await;
        
        for opportunity in opportunities {
            cache.insert(opportunity.id.clone(), opportunity.clone());
        }

        // Cleanup expired opportunities
        let current_time = chrono::Utc::now().timestamp() as u64;
        cache.retain(|_, opp| opp.expiry_timestamp > current_time);

        // Limit cache size
        if cache.len() > self.config.opportunity_cache_size {
            let excess = cache.len() - self.config.opportunity_cache_size;
            let keys_to_remove: Vec<_> = cache.keys().take(excess).cloned().collect();
            for key in keys_to_remove {
                cache.remove(&key);
            }
        }
    }

    /// Create bundle plan from opportunity
    pub async fn create_bundle_plan(&self, opportunity_id: &str) -> Result<Option<BundlePlan>> {
        // Get opportunity from cache
        let opportunity = {
            let opportunities = self.opportunities.read().await;
            opportunities.get(opportunity_id).cloned()
        };

        let opportunity = match opportunity {
            Some(opp) => opp,
            None => {
                warn!(opportunity_id = %opportunity_id, "Opportunity not found in cache");
                return Ok(None);
            }
        };

        // Check if opportunity is still valid
        if !strategy_utils::is_opportunity_valid(&opportunity) {
            debug!(opportunity_id = %opportunity_id, "Opportunity expired");
            return Ok(None);
        }

        // Get the strategy that created this opportunity
        let strategies = self.strategies.read().await;
        let strategy = match strategies.get(&opportunity.strategy_name) {
            Some(s) => s,
            None => {
                error!(
                    opportunity_id = %opportunity_id,
                    strategy_name = %opportunity.strategy_name,
                    "Strategy not found for opportunity"
                );
                return Ok(None);
            }
        };

        // Create bundle plan
        match strategy.create_bundle_plan(&opportunity).await {
            Ok(bundle_plan) => {
                // Cache the bundle plan
                {
                    let mut bundles = self.bundle_plans.write().await;
                    bundles.insert(bundle_plan.id.clone(), bundle_plan.clone());
                }

                // Update stats
                {
                    let mut stats = self.stats.write().await;
                    stats.bundles_created += 1;
                }

                info!(
                    bundle_id = %bundle_plan.id,
                    opportunity_id = %opportunity_id,
                    strategy = %opportunity.strategy_name,
                    "Bundle plan created"
                );

                Ok(Some(bundle_plan))
            }
            Err(e) => {
                error!(
                    opportunity_id = %opportunity_id,
                    strategy = %opportunity.strategy_name,
                    error = %e,
                    "Failed to create bundle plan"
                );
                Err(e)
            }
        }
    }

    /// Get all active opportunities
    pub async fn get_active_opportunities(&self) -> Vec<Opportunity> {
        let opportunities = self.opportunities.read().await;
        let current_time = chrono::Utc::now().timestamp() as u64;
        
        opportunities
            .values()
            .filter(|opp| opp.expiry_timestamp > current_time)
            .cloned()
            .collect()
    }

    /// Get opportunities by strategy
    pub async fn get_opportunities_by_strategy(&self, strategy_name: &str) -> Vec<Opportunity> {
        let opportunities = self.opportunities.read().await;
        
        opportunities
            .values()
            .filter(|opp| opp.strategy_name == strategy_name)
            .filter(|opp| strategy_utils::is_opportunity_valid(opp))
            .cloned()
            .collect()
    }

    /// Get bundle plan by ID
    pub async fn get_bundle_plan(&self, bundle_id: &str) -> Option<BundlePlan> {
        let bundles = self.bundle_plans.read().await;
        bundles.get(bundle_id).cloned()
    }

    /// Get strategy statistics
    pub async fn get_strategy_stats(&self, strategy_name: &str) -> Option<StrategyStats> {
        let strategies = self.strategies.read().await;
        strategies.get(strategy_name).map(|s| s.get_stats())
    }

    /// Get engine statistics
    pub async fn get_engine_stats(&self) -> StrategyEngineStats {
        let stats = self.stats.read().await;
        stats.clone()
    }

    /// Reset all statistics
    pub async fn reset_stats(&self) -> Result<()> {
        // Reset engine stats
        {
            let mut stats = self.stats.write().await;
            *stats = StrategyEngineStats::default();
        }

        // Reset strategy stats
        let mut strategies = self.strategies.write().await;
        for strategy in strategies.values_mut() {
            strategy.reset_stats();
        }

        info!("All strategy engine statistics reset");
        Ok(())
    }

    /// Cleanup expired opportunities and bundle plans
    pub async fn cleanup_expired(&self) {
        let current_time = chrono::Utc::now().timestamp() as u64;
        
        // Cleanup opportunities
        {
            let mut opportunities = self.opportunities.write().await;
            let initial_count = opportunities.len();
            opportunities.retain(|_, opp| opp.expiry_timestamp > current_time);
            let removed = initial_count - opportunities.len();
            if removed > 0 {
                debug!(removed_opportunities = removed, "Cleaned up expired opportunities");
            }
        }

        // Cleanup bundle plans
        {
            let mut bundles = self.bundle_plans.write().await;
            let initial_count = bundles.len();
            bundles.retain(|_, bundle| bundle.execution_deadline > current_time);
            let removed = initial_count - bundles.len();
            if removed > 0 {
                debug!(removed_bundles = removed, "Cleaned up expired bundle plans");
            }
        }
    }

    /// Start background cleanup task
    pub async fn start_cleanup_task(&self) {
        let engine = self.clone();
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
            
            loop {
                interval.tick().await;
                engine.cleanup_expired().await;
            }
        });

        info!("Started strategy engine cleanup task");
    }
}

impl Clone for StrategyEngine {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            strategies: self.strategies.clone(),
            opportunities: self.opportunities.clone(),
            bundle_plans: self.bundle_plans.clone(),
            metrics: self.metrics.clone(),
            stats: self.stats.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArbitrageStrategy, BackrunStrategy};
    use mev_core::{Transaction, TargetType};

    fn create_test_transaction() -> ParsedTransaction {
        ParsedTransaction {
            transaction: Transaction {
                hash: "0x1234567890abcdef".to_string(),
                from: "0xfrom".to_string(),
                to: Some("0xto".to_string()),
                value: "2000000000000000000".to_string(), // 2 ETH
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
    async fn test_strategy_registration() {
        let metrics = Arc::new(PrometheusMetrics::new().unwrap());
        let engine = StrategyEngine::new(StrategyEngineConfig::default(), metrics);

        let arbitrage_strategy = Box::new(ArbitrageStrategy::new());
        let backrun_strategy = Box::new(BackrunStrategy::new());

        assert!(engine.register_strategy(arbitrage_strategy).await.is_ok());
        assert!(engine.register_strategy(backrun_strategy).await.is_ok());

        let strategies = engine.strategies.read().await;
        assert_eq!(strategies.len(), 2);
        assert!(strategies.contains_key("arbitrage"));
        assert!(strategies.contains_key("backrun"));
    }

    #[tokio::test]
    async fn test_opportunity_caching() {
        let metrics = Arc::new(PrometheusMetrics::new().unwrap());
        let engine = StrategyEngine::new(StrategyEngineConfig::default(), metrics);

        let opportunity = Opportunity {
            id: "test_opp_1".to_string(),
            strategy_name: "test".to_string(),
            target_transaction: create_test_transaction(),
            opportunity_type: OpportunityType::Arbitrage {
                token_in: "WETH".to_string(),
                token_out: "USDC".to_string(),
                amount_in: 1000000000000000000,
                expected_profit: 50000000000000000,
                dex_path: vec!["UniswapV2".to_string()],
            },
            estimated_profit_wei: 50000000000000000,
            estimated_gas_cost_wei: 5000000000000000,
            confidence_score: 0.8,
            priority: 200,
            expiry_timestamp: chrono::Utc::now().timestamp() as u64 + 60,
            metadata: HashMap::new(),
        };

        engine.cache_opportunities(&[opportunity.clone()]).await;

        let cached_opportunities = engine.get_active_opportunities().await;
        assert_eq!(cached_opportunities.len(), 1);
        assert_eq!(cached_opportunities[0].id, opportunity.id);
    }

    #[tokio::test]
    async fn test_cleanup_expired() {
        let metrics = Arc::new(PrometheusMetrics::new().unwrap());
        let engine = StrategyEngine::new(StrategyEngineConfig::default(), metrics);

        // Create expired opportunity
        let expired_opportunity = Opportunity {
            id: "expired_opp".to_string(),
            strategy_name: "test".to_string(),
            target_transaction: create_test_transaction(),
            opportunity_type: OpportunityType::Arbitrage {
                token_in: "WETH".to_string(),
                token_out: "USDC".to_string(),
                amount_in: 1000000000000000000,
                expected_profit: 50000000000000000,
                dex_path: vec!["UniswapV2".to_string()],
            },
            estimated_profit_wei: 50000000000000000,
            estimated_gas_cost_wei: 5000000000000000,
            confidence_score: 0.8,
            priority: 200,
            expiry_timestamp: chrono::Utc::now().timestamp() as u64 - 60, // Expired
            metadata: HashMap::new(),
        };

        engine.cache_opportunities(&[expired_opportunity]).await;
        
        // Should be removed during caching due to expiry
        let active_opportunities = engine.get_active_opportunities().await;
        assert_eq!(active_opportunities.len(), 0);
    }
}