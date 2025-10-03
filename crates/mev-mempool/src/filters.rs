//! Transaction filtering system for MEV bot mempool processing

use anyhow::Result;
use chrono::Timelike;
use mev_core::ParsedTransaction;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

/// Filter rule configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterRule {
    pub name: String,
    pub enabled: bool,
    pub priority: i32, // Higher priority = processed first
    pub conditions: Vec<FilterCondition>,
    pub action: FilterAction,
    pub description: String,
}

/// Filter condition types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FilterCondition {
    /// Filter by transaction value
    ValueRange {
        min_wei: Option<String>,
        max_wei: Option<String>,
    },
    /// Filter by gas price
    GasPriceRange {
        min_gwei: Option<u64>,
        max_gwei: Option<u64>,
    },
    /// Filter by target contract type
    TargetType {
        types: Vec<String>, // "UniswapV2", "UniswapV3", etc.
    },
    /// Filter by function signature
    FunctionSignature {
        signatures: Vec<String>, // "0x38ed1739", etc.
    },
    /// Filter by contract address
    ContractAddress {
        addresses: Vec<String>,
        whitelist: bool, // true = only these addresses, false = exclude these
    },
    /// Filter by sender address
    SenderAddress {
        addresses: Vec<String>,
        whitelist: bool,
    },
    /// Filter by transaction size (calldata)
    DataSize {
        min_bytes: Option<usize>,
        max_bytes: Option<usize>,
    },
    /// Filter by time of day (UTC hours)
    TimeOfDay {
        start_hour: u8, // 0-23
        end_hour: u8,   // 0-23
    },
    /// Custom regex pattern on calldata
    DataPattern {
        pattern: String,
        case_sensitive: bool,
    },
}

/// Filter action to take when conditions match
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FilterAction {
    /// Accept transaction (high priority)
    Accept,
    /// Reject transaction (drop it)
    Reject,
    /// Tag transaction with metadata
    Tag { tags: Vec<String> },
    /// Set priority level
    SetPriority { level: u8 }, // 0-255, higher = more important
    /// Rate limit (max per second)
    RateLimit { max_per_second: u32 },
}

/// Transaction with filter metadata
#[derive(Debug, Clone)]
pub struct FilteredTransaction {
    pub transaction: ParsedTransaction,
    pub priority: u8,
    pub tags: Vec<String>,
    pub matched_rules: Vec<String>,
    pub filter_time_ms: f64,
}

impl FilteredTransaction {
    pub fn new(transaction: ParsedTransaction) -> Self {
        Self {
            transaction,
            priority: 128, // Default medium priority
            tags: Vec::new(),
            matched_rules: Vec::new(),
            filter_time_ms: 0.0,
        }
    }

    pub fn add_tag(&mut self, tag: String) {
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
    }

    pub fn set_priority(&mut self, priority: u8) {
        self.priority = priority;
    }

    pub fn add_matched_rule(&mut self, rule_name: String) {
        if !self.matched_rules.contains(&rule_name) {
            self.matched_rules.push(rule_name);
        }
    }
}

/// Filter engine for processing transactions
pub struct FilterEngine {
    rules: Vec<FilterRule>,
    rate_limiters: HashMap<String, RateLimiter>,
    stats: FilterStats,
}

impl FilterEngine {
    pub fn new() -> Self {
        Self {
            rules: Vec::new(),
            rate_limiters: HashMap::new(),
            stats: FilterStats::default(),
        }
    }

    /// Load filter rules from configuration
    pub fn load_rules(&mut self, rules: Vec<FilterRule>) -> Result<()> {
        // Sort rules by priority (highest first)
        let mut sorted_rules = rules;
        sorted_rules.sort_by(|a, b| b.priority.cmp(&a.priority));

        self.rules = sorted_rules;
        
        info!(
            rule_count = self.rules.len(),
            "Loaded filter rules"
        );

        // Initialize rate limiters for rules that need them
        for rule in &self.rules {
            if let FilterAction::RateLimit { max_per_second } = &rule.action {
                self.rate_limiters.insert(
                    rule.name.clone(),
                    RateLimiter::new(*max_per_second),
                );
            }
        }

        Ok(())
    }

    /// Apply filters to a transaction
    pub fn filter_transaction(&mut self, transaction: ParsedTransaction) -> Result<Option<FilteredTransaction>> {
        let start_time = std::time::Instant::now();
        let mut filtered_tx = FilteredTransaction::new(transaction);
        
        self.stats.total_processed += 1;

        // Apply each rule in priority order
        for rule in &self.rules {
            if !rule.enabled {
                continue;
            }

            if self.evaluate_rule(rule, &filtered_tx.transaction)? {
                filtered_tx.add_matched_rule(rule.name.clone());
                
                match &rule.action {
                    FilterAction::Accept => {
                        filtered_tx.set_priority(255); // Highest priority
                        debug!(
                            rule = %rule.name,
                            tx_hash = %filtered_tx.transaction.transaction.hash,
                            "Transaction accepted by rule"
                        );
                    }
                    FilterAction::Reject => {
                        self.stats.rejected += 1;
                        debug!(
                            rule = %rule.name,
                            tx_hash = %filtered_tx.transaction.transaction.hash,
                            "Transaction rejected by rule"
                        );
                        return Ok(None); // Drop the transaction
                    }
                    FilterAction::Tag { tags } => {
                        for tag in tags {
                            filtered_tx.add_tag(tag.clone());
                        }
                        debug!(
                            rule = %rule.name,
                            tags = ?tags,
                            tx_hash = %filtered_tx.transaction.transaction.hash,
                            "Transaction tagged by rule"
                        );
                    }
                    FilterAction::SetPriority { level } => {
                        filtered_tx.set_priority(*level);
                        debug!(
                            rule = %rule.name,
                            priority = level,
                            tx_hash = %filtered_tx.transaction.transaction.hash,
                            "Transaction priority set by rule"
                        );
                    }
                    FilterAction::RateLimit { max_per_second: _ } => {
                        if let Some(rate_limiter) = self.rate_limiters.get_mut(&rule.name) {
                            if !rate_limiter.allow() {
                                self.stats.rate_limited += 1;
                                debug!(
                                    rule = %rule.name,
                                    tx_hash = %filtered_tx.transaction.transaction.hash,
                                    "Transaction rate limited by rule"
                                );
                                return Ok(None); // Drop due to rate limit
                            }
                        }
                    }
                }
            }
        }

        filtered_tx.filter_time_ms = start_time.elapsed().as_secs_f64() * 1000.0;
        self.stats.accepted += 1;
        self.stats.total_filter_time_ms += filtered_tx.filter_time_ms;

        Ok(Some(filtered_tx))
    }

    /// Evaluate if a rule matches a transaction
    fn evaluate_rule(&self, rule: &FilterRule, transaction: &ParsedTransaction) -> Result<bool> {
        // All conditions must be true for the rule to match (AND logic)
        for condition in &rule.conditions {
            if !self.evaluate_condition(condition, transaction)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Evaluate a single condition
    fn evaluate_condition(&self, condition: &FilterCondition, transaction: &ParsedTransaction) -> Result<bool> {
        match condition {
            FilterCondition::ValueRange { min_wei, max_wei } => {
                let value = transaction.transaction.value.parse::<u128>().unwrap_or(0);
                
                if let Some(min) = min_wei {
                    let min_val = min.parse::<u128>().unwrap_or(0);
                    if value < min_val {
                        return Ok(false);
                    }
                }
                
                if let Some(max) = max_wei {
                    let max_val = max.parse::<u128>().unwrap_or(u128::MAX);
                    if value > max_val {
                        return Ok(false);
                    }
                }
                
                Ok(true)
            }
            
            FilterCondition::GasPriceRange { min_gwei, max_gwei } => {
                let gas_price = transaction.transaction.gas_price.parse::<u64>().unwrap_or(0);
                let gas_price_gwei = gas_price / 1_000_000_000; // Convert wei to gwei
                
                if let Some(min) = min_gwei {
                    if gas_price_gwei < *min {
                        return Ok(false);
                    }
                }
                
                if let Some(max) = max_gwei {
                    if gas_price_gwei > *max {
                        return Ok(false);
                    }
                }
                
                Ok(true)
            }
            
            FilterCondition::TargetType { types } => {
                let target_type_str = format!("{:?}", transaction.target_type);
                Ok(types.contains(&target_type_str))
            }
            
            FilterCondition::FunctionSignature { signatures } => {
                if let Some(decoded) = &transaction.decoded_input {
                    Ok(signatures.contains(&decoded.function_signature))
                } else {
                    Ok(false)
                }
            }
            
            FilterCondition::ContractAddress { addresses, whitelist } => {
                if let Some(to_address) = &transaction.transaction.to {
                    let matches = addresses.iter().any(|addr| {
                        addr.to_lowercase() == to_address.to_lowercase()
                    });
                    Ok(if *whitelist { matches } else { !matches })
                } else {
                    Ok(!whitelist) // Contract creation - only pass if not whitelist
                }
            }
            
            FilterCondition::SenderAddress { addresses, whitelist } => {
                let matches = addresses.iter().any(|addr| {
                    addr.to_lowercase() == transaction.transaction.from.to_lowercase()
                });
                Ok(if *whitelist { matches } else { !matches })
            }
            
            FilterCondition::DataSize { min_bytes, max_bytes } => {
                let data_size = transaction.transaction.input.len() / 2 - 1; // Remove 0x prefix and convert hex to bytes
                
                if let Some(min) = min_bytes {
                    if data_size < *min {
                        return Ok(false);
                    }
                }
                
                if let Some(max) = max_bytes {
                    if data_size > *max {
                        return Ok(false);
                    }
                }
                
                Ok(true)
            }
            
            FilterCondition::TimeOfDay { start_hour, end_hour } => {
                let now = chrono::Utc::now();
                let current_hour = now.hour() as u8;
                
                if start_hour <= end_hour {
                    // Normal range (e.g., 9-17)
                    Ok(current_hour >= *start_hour && current_hour <= *end_hour)
                } else {
                    // Overnight range (e.g., 22-6)
                    Ok(current_hour >= *start_hour || current_hour <= *end_hour)
                }
            }
            
            FilterCondition::DataPattern { pattern, case_sensitive } => {
                let data = if *case_sensitive {
                    transaction.transaction.input.clone()
                } else {
                    transaction.transaction.input.to_lowercase()
                };
                
                let search_pattern = if *case_sensitive {
                    pattern.clone()
                } else {
                    pattern.to_lowercase()
                };
                
                Ok(data.contains(&search_pattern))
            }
        }
    }

    /// Get filter statistics
    pub fn get_stats(&self) -> FilterStats {
        let mut stats = self.stats.clone();
        
        if stats.total_processed > 0 {
            stats.average_filter_time_ms = stats.total_filter_time_ms / stats.total_processed as f64;
            stats.acceptance_rate = stats.accepted as f64 / stats.total_processed as f64;
            stats.rejection_rate = stats.rejected as f64 / stats.total_processed as f64;
        }
        
        stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = FilterStats::default();
    }

    /// Get active rules
    pub fn get_active_rules(&self) -> Vec<&FilterRule> {
        self.rules.iter().filter(|rule| rule.enabled).collect()
    }

    /// Enable/disable a rule
    pub fn set_rule_enabled(&mut self, rule_name: &str, enabled: bool) -> Result<()> {
        if let Some(rule) = self.rules.iter_mut().find(|r| r.name == rule_name) {
            rule.enabled = enabled;
            info!(rule = rule_name, enabled = enabled, "Rule status changed");
            Ok(())
        } else {
            Err(anyhow::anyhow!("Rule '{}' not found", rule_name))
        }
    }
}

/// Rate limiter for filter actions
struct RateLimiter {
    max_per_second: u32,
    tokens: u32,
    last_refill: std::time::Instant,
}

impl RateLimiter {
    fn new(max_per_second: u32) -> Self {
        Self {
            max_per_second,
            tokens: max_per_second,
            last_refill: std::time::Instant::now(),
        }
    }

    fn allow(&mut self) -> bool {
        self.refill_tokens();
        
        if self.tokens > 0 {
            self.tokens -= 1;
            true
        } else {
            false
        }
    }

    fn refill_tokens(&mut self) {
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        
        if elapsed >= 1.0 {
            self.tokens = self.max_per_second;
            self.last_refill = now;
        }
    }
}

/// Filter engine statistics
#[derive(Debug, Clone, Default)]
pub struct FilterStats {
    pub total_processed: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub rate_limited: u64,
    pub total_filter_time_ms: f64,
    pub average_filter_time_ms: f64,
    pub acceptance_rate: f64,
    pub rejection_rate: f64,
}

impl FilterStats {
    pub fn print_summary(&self) {
        info!(
            total_processed = self.total_processed,
            accepted = self.accepted,
            rejected = self.rejected,
            rate_limited = self.rate_limited,
            acceptance_rate = format!("{:.2}%", self.acceptance_rate * 100.0),
            rejection_rate = format!("{:.2}%", self.rejection_rate * 100.0),
            avg_filter_time_ms = format!("{:.3}", self.average_filter_time_ms),
            "Filter engine statistics"
        );
    }
}

/// Predefined filter rule templates
pub struct FilterRuleTemplates;

impl FilterRuleTemplates {
    /// High-value transaction filter
    pub fn high_value_transactions() -> FilterRule {
        FilterRule {
            name: "high_value_transactions".to_string(),
            enabled: true,
            priority: 100,
            conditions: vec![FilterCondition::ValueRange {
                min_wei: Some("1000000000000000000".to_string()), // 1 ETH
                max_wei: None,
            }],
            action: FilterAction::Tag {
                tags: vec!["high_value".to_string()],
            },
            description: "Tag transactions with value >= 1 ETH".to_string(),
        }
    }

    /// DEX transaction filter
    pub fn dex_transactions() -> FilterRule {
        FilterRule {
            name: "dex_transactions".to_string(),
            enabled: true,
            priority: 90,
            conditions: vec![FilterCondition::TargetType {
                types: vec![
                    "UniswapV2".to_string(),
                    "UniswapV3".to_string(),
                    "SushiSwap".to_string(),
                    "Curve".to_string(),
                ],
            }],
            action: FilterAction::Accept,
            description: "Accept all DEX transactions".to_string(),
        }
    }

    /// High gas price filter
    pub fn high_gas_price() -> FilterRule {
        FilterRule {
            name: "high_gas_price".to_string(),
            enabled: true,
            priority: 80,
            conditions: vec![FilterCondition::GasPriceRange {
                min_gwei: Some(100), // 100 gwei
                max_gwei: None,
            }],
            action: FilterAction::Tag {
                tags: vec!["high_gas".to_string()],
            },
            description: "Tag transactions with gas price >= 100 gwei".to_string(),
        }
    }

    /// Spam filter
    pub fn spam_filter() -> FilterRule {
        FilterRule {
            name: "spam_filter".to_string(),
            enabled: true,
            priority: 200, // High priority to reject early
            conditions: vec![
                FilterCondition::ValueRange {
                    min_wei: None,
                    max_wei: Some("1000000000000000".to_string()), // 0.001 ETH
                },
                FilterCondition::GasPriceRange {
                    min_gwei: None,
                    max_gwei: Some(10), // 10 gwei
                },
            ],
            action: FilterAction::Reject,
            description: "Reject low-value, low-gas transactions (likely spam)".to_string(),
        }
    }

    /// Rate limit filter
    pub fn rate_limit_per_address() -> FilterRule {
        FilterRule {
            name: "rate_limit_per_address".to_string(),
            enabled: true,
            priority: 150,
            conditions: vec![], // Apply to all transactions
            action: FilterAction::RateLimit {
                max_per_second: 10,
            },
            description: "Rate limit transactions to 10 per second".to_string(),
        }
    }

    /// Get all default templates
    pub fn get_default_rules() -> Vec<FilterRule> {
        vec![
            Self::spam_filter(),
            Self::dex_transactions(),
            Self::high_value_transactions(),
            Self::high_gas_price(),
            Self::rate_limit_per_address(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mev_core::{DecodedInput, Transaction};
    use chrono::Utc;

    fn create_test_transaction(value_eth: f64, gas_price_gwei: u64, target_type: TargetType) -> ParsedTransaction {
        let value_wei = (value_eth * 1e18) as u128;
        let gas_price_wei = gas_price_gwei * 1_000_000_000;

        ParsedTransaction {
            transaction: Transaction {
                hash: "0x123".to_string(),
                from: "0xfrom".to_string(),
                to: Some("0xto".to_string()),
                value: value_wei.to_string(),
                gas_price: gas_price_wei.to_string(),
                gas_limit: "21000".to_string(),
                nonce: 1,
                input: "0x38ed1739".to_string(),
                timestamp: Utc::now(),
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
    fn test_value_range_filter() {
        let mut engine = FilterEngine::new();
        let rule = FilterRuleTemplates::high_value_transactions();
        engine.load_rules(vec![rule]).unwrap();

        // High value transaction should be tagged
        let high_value_tx = create_test_transaction(2.0, 20, TargetType::UniswapV2);
        let result = engine.filter_transaction(high_value_tx).unwrap();
        assert!(result.is_some());
        assert!(result.unwrap().tags.contains(&"high_value".to_string()));

        // Low value transaction should not be tagged
        let low_value_tx = create_test_transaction(0.1, 20, TargetType::UniswapV2);
        let result = engine.filter_transaction(low_value_tx).unwrap();
        assert!(result.is_some());
        assert!(!result.unwrap().tags.contains(&"high_value".to_string()));
    }

    #[test]
    fn test_dex_filter() {
        let mut engine = FilterEngine::new();
        let rule = FilterRuleTemplates::dex_transactions();
        engine.load_rules(vec![rule]).unwrap();

        // DEX transaction should be accepted with high priority
        let dex_tx = create_test_transaction(1.0, 20, TargetType::UniswapV2);
        let result = engine.filter_transaction(dex_tx).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().priority, 255);

        // Non-DEX transaction should have default priority
        let other_tx = create_test_transaction(1.0, 20, TargetType::Unknown);
        let result = engine.filter_transaction(other_tx).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().priority, 128);
    }

    #[test]
    fn test_spam_filter() {
        let mut engine = FilterEngine::new();
        let rule = FilterRuleTemplates::spam_filter();
        engine.load_rules(vec![rule]).unwrap();

        // Spam transaction should be rejected
        let spam_tx = create_test_transaction(0.0001, 5, TargetType::Unknown);
        let result = engine.filter_transaction(spam_tx).unwrap();
        assert!(result.is_none());

        // Normal transaction should pass
        let normal_tx = create_test_transaction(1.0, 20, TargetType::UniswapV2);
        let result = engine.filter_transaction(normal_tx).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_filter_stats() {
        let mut engine = FilterEngine::new();
        let rules = FilterRuleTemplates::get_default_rules();
        engine.load_rules(rules).unwrap();

        // Process some transactions
        for i in 0..10 {
            let tx = create_test_transaction(
                if i % 2 == 0 { 2.0 } else { 0.1 },
                20,
                TargetType::UniswapV2,
            );
            let _ = engine.filter_transaction(tx);
        }

        let stats = engine.get_stats();
        assert_eq!(stats.total_processed, 10);
        assert!(stats.accepted > 0);
        assert!(stats.average_filter_time_ms > 0.0);
    }
}