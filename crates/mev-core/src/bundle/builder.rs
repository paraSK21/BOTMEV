//! Bundle builder with comprehensive construction logic

use super::types::{Bundle, BundleParams, BundleValidation, NonceStrategy, SignedTransaction, TransactionTemplate};
use super::signer::TransactionSigner;
use crate::strategy_types::{Opportunity, OpportunityType};
use anyhow::{anyhow, Result};
use ethers::types::{Address, Bytes, U256};
use ethers::signers::Signer;
use std::collections::HashMap;
use std::sync::Arc;

/// Bundle builder for constructing MEV bundles
pub struct BundleBuilder {
    /// Transaction signer
    signer: Arc<TransactionSigner>,
    /// Default bundle parameters
    default_params: BundleParams,
    /// Strategy-specific bundle builders
    strategy_builders: HashMap<String, Box<dyn StrategyBundleBuilder>>,
}

/// Strategy-specific bundle building logic
#[async_trait::async_trait]
pub trait StrategyBundleBuilder: Send + Sync {
    /// Build bundle transactions for a specific strategy
    async fn build_bundle_transactions(
        &self,
        opportunity: &Opportunity,
        params: &BundleParams,
    ) -> Result<(Vec<TransactionTemplate>, Vec<TransactionTemplate>)>;
    
    /// Validate bundle for strategy-specific requirements
    fn validate_bundle(&self, bundle: &Bundle, opportunity: &Opportunity) -> BundleValidation;
}

impl BundleBuilder {
    /// Create a new bundle builder
    pub fn new(signer: Arc<TransactionSigner>) -> Self {
        Self {
            signer,
            default_params: BundleParams::default(),
            strategy_builders: HashMap::new(),
        }
    }
    
    /// Create a bundle builder with custom default parameters
    pub fn with_params(signer: Arc<TransactionSigner>, params: BundleParams) -> Self {
        Self {
            signer,
            default_params: params,
            strategy_builders: HashMap::new(),
        }
    }
    
    /// Register a strategy-specific bundle builder
    pub fn register_strategy_builder(
        &mut self,
        strategy_name: String,
        builder: Box<dyn StrategyBundleBuilder>,
    ) {
        self.strategy_builders.insert(strategy_name, builder);
    }
    
    /// Build a complete bundle from an opportunity
    pub async fn build_bundle(&self, opportunity: &Opportunity) -> Result<Bundle> {
        self.build_bundle_with_params(opportunity, &self.default_params).await
    }
    
    /// Build a bundle with custom parameters
    pub async fn build_bundle_with_params(
        &self,
        opportunity: &Opportunity,
        params: &BundleParams,
    ) -> Result<Bundle> {
        // Get strategy-specific builder
        let strategy_builder = self.strategy_builders
            .get(&opportunity.strategy_name)
            .ok_or_else(|| anyhow!("No builder registered for strategy: {}", opportunity.strategy_name))?;
        
        // Build pre and post transaction templates
        let (pre_templates, post_templates) = strategy_builder
            .build_bundle_transactions(opportunity, params)
            .await?;
        
        // Create target transaction template from opportunity
        let _target_template = self.create_target_template(opportunity)?;
        
        // Calculate gas prices
        let (max_fee_per_gas, max_priority_fee_per_gas) = self.calculate_gas_prices(params)?;
        
        // Sign all transactions with proper nonce sequencing
        let mut _signed_transactions: Vec<SignedTransaction> = Vec::new();
        let mut current_nonce = self.get_starting_nonce().await?;
        
        // Sign pre-transactions
        let mut pre_transactions = Vec::new();
        for template in pre_templates {
            let signed_tx = self.signer.sign_transaction(
                &template,
                NonceStrategy::Fixed(current_nonce.as_u64()),
                max_fee_per_gas,
                max_priority_fee_per_gas,
            ).await?;
            
            pre_transactions.push(signed_tx);
            current_nonce += U256::one();
        }
        
        // Sign target transaction (this is the victim's transaction, so we don't sign it)
        // Instead, we create a placeholder signed transaction from the opportunity data
        let target_transaction = self.create_target_signed_transaction(opportunity)?;
        
        // Sign post-transactions
        let mut post_transactions = Vec::new();
        for template in post_templates {
            let signed_tx = self.signer.sign_transaction(
                &template,
                NonceStrategy::Fixed(current_nonce.as_u64()),
                max_fee_per_gas,
                max_priority_fee_per_gas,
            ).await?;
            
            post_transactions.push(signed_tx);
            current_nonce += U256::one();
        }
        
        // Create bundle
        let bundle = Bundle {
            id: format!("bundle_{}", uuid::Uuid::new_v4()),
            pre_transactions,
            target_transaction,
            post_transactions,
            target_block: params.target_block,
            max_gas_price: params.max_gas_price,
            estimated_profit: U256::from(opportunity.estimated_profit_wei),
            created_at: chrono::Utc::now(),
            metadata: self.create_bundle_metadata(opportunity, params),
        };
        
        // Validate bundle
        let validation = strategy_builder.validate_bundle(&bundle, opportunity);
        if !validation.is_valid {
            return Err(anyhow!("Bundle validation failed: {:?}", validation.errors));
        }
        
        Ok(bundle)
    }
    
    /// Create target transaction template from opportunity
    fn create_target_template(&self, opportunity: &Opportunity) -> Result<TransactionTemplate> {
        let tx = &opportunity.target_transaction.transaction;
        
        Ok(TransactionTemplate {
            to: tx.to.as_ref().map(|s| s.parse()).transpose().map_err(|e| anyhow!("Invalid to address: {}", e))?,
            value: tx.value.parse().map_err(|e| anyhow!("Invalid value: {}", e))?,
            gas_limit: tx.gas_limit.parse().map_err(|e| anyhow!("Invalid gas limit: {}", e))?,
            data: Bytes::from(hex::decode(tx.input.trim_start_matches("0x")).map_err(|e| anyhow!("Invalid input data: {}", e))?),
            transaction_type: 2, // EIP-1559
        })
    }
    
    /// Create signed transaction from opportunity (for target transaction)
    fn create_target_signed_transaction(&self, opportunity: &Opportunity) -> Result<SignedTransaction> {
        let tx = &opportunity.target_transaction.transaction;
        
        Ok(SignedTransaction {
            hash: tx.hash.parse().map_err(|e| anyhow!("Invalid hash: {}", e))?,
            from: tx.from.parse().map_err(|e| anyhow!("Invalid from address: {}", e))?,
            to: tx.to.as_ref().map(|s| s.parse()).transpose().map_err(|e| anyhow!("Invalid to address: {}", e))?,
            value: tx.value.parse().map_err(|e| anyhow!("Invalid value: {}", e))?,
            gas_limit: tx.gas_limit.parse().map_err(|e| anyhow!("Invalid gas limit: {}", e))?,
            max_fee_per_gas: tx.gas_price.parse().map_err(|e| anyhow!("Invalid gas price: {}", e))?,
            max_priority_fee_per_gas: U256::from(2_000_000_000u64), // 2 gwei default
            nonce: U256::from(tx.nonce),
            data: Bytes::from(hex::decode(tx.input.trim_start_matches("0x")).map_err(|e| anyhow!("Invalid input data: {}", e))?),
            chain_id: 998, // HyperEVM
            raw_transaction: Bytes::default(), // Would be filled by actual transaction
            transaction_type: 2,
        })
    }
    
    /// Calculate gas prices based on parameters
    fn calculate_gas_prices(&self, params: &BundleParams) -> Result<(U256, U256)> {
        let base_fee = params.max_gas_price;
        let priority_fee = params.max_priority_fee;
        
        // Apply gas multiplier
        let max_fee_per_gas = U256::from(
            (base_fee.as_u128() as f64 * params.gas_multiplier) as u128
        );
        
        // Ensure max fee is at least base fee + priority fee
        let max_fee_per_gas = std::cmp::max(max_fee_per_gas, base_fee + priority_fee);
        
        Ok((max_fee_per_gas, priority_fee))
    }
    
    /// Get starting nonce for bundle transactions
    async fn get_starting_nonce(&self) -> Result<U256> {
        let wallet = self.signer.key_manager.get_default_wallet().await?;
        let address = wallet.address();
        
        // In a real implementation, this would query the network for the current nonce
        // For now, use tracked nonce or start from 0
        Ok(self.signer.get_tracked_nonce(address).await.unwrap_or(U256::zero()))
    }
    
    /// Create bundle metadata
    fn create_bundle_metadata(&self, opportunity: &Opportunity, params: &BundleParams) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        
        metadata.insert("strategy".to_string(), opportunity.strategy_name.clone());
        metadata.insert("opportunity_id".to_string(), opportunity.id.clone());
        metadata.insert("opportunity_type".to_string(), format!("{:?}", opportunity.opportunity_type));
        metadata.insert("confidence_score".to_string(), opportunity.confidence_score.to_string());
        metadata.insert("priority".to_string(), opportunity.priority.to_string());
        metadata.insert("expiry_seconds".to_string(), params.expiry_seconds.to_string());
        metadata.insert("gas_multiplier".to_string(), params.gas_multiplier.to_string());
        
        // Add opportunity-specific metadata
        for (key, value) in &opportunity.metadata {
            metadata.insert(format!("opp_{}", key), value.clone());
        }
        
        metadata
    }
}

/// Arbitrage strategy bundle builder
pub struct ArbitrageBundleBuilder;

#[async_trait::async_trait]
impl StrategyBundleBuilder for ArbitrageBundleBuilder {
    async fn build_bundle_transactions(
        &self,
        opportunity: &Opportunity,
        _params: &BundleParams,
    ) -> Result<(Vec<TransactionTemplate>, Vec<TransactionTemplate>)> {
        match &opportunity.opportunity_type {
            OpportunityType::Arbitrage { token_in, token_out, amount_in, dex_path, .. } => {
                // For arbitrage, we typically don't need pre-transactions
                let pre_transactions = Vec::new();
                
                // Post-transaction: Execute the arbitrage
                let mut post_transactions = Vec::new();
                
                // Create arbitrage execution transaction
                let arbitrage_data = self.encode_arbitrage_call(token_in, token_out, *amount_in, dex_path)?;
                let arbitrage_template = TransactionTemplate {
                    to: Some(self.get_arbitrage_contract_address()), // Would be actual arbitrage contract
                    value: U256::zero(),
                    gas_limit: U256::from(300000),
                    data: arbitrage_data,
                    transaction_type: 2,
                };
                
                post_transactions.push(arbitrage_template);
                
                Ok((pre_transactions, post_transactions))
            }
            _ => Err(anyhow!("Invalid opportunity type for arbitrage builder")),
        }
    }
    
    fn validate_bundle(&self, bundle: &Bundle, _opportunity: &Opportunity) -> BundleValidation {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        // Validate arbitrage-specific requirements
        if bundle.post_transactions.is_empty() {
            errors.push("Arbitrage bundle must have post-transactions".to_string());
        }
        
        // Check profit threshold
        if bundle.estimated_profit < U256::from(1_000_000_000_000_000u64) { // 0.001 ETH
            warnings.push("Low profit margin for arbitrage".to_string());
        }
        
        // Estimate gas cost
        let estimated_gas_cost = bundle.total_gas_limit() * bundle.max_gas_price;
        
        BundleValidation {
            is_valid: errors.is_empty(),
            errors,
            warnings,
            estimated_gas_cost,
            estimated_profit: bundle.estimated_profit,
        }
    }
}

impl ArbitrageBundleBuilder {
    fn encode_arbitrage_call(
        &self,
        _token_in: &str,
        _token_out: &str,
        _amount_in: u128,
        _dex_path: &[String],
    ) -> Result<Bytes> {
        // In a real implementation, this would encode the actual arbitrage function call
        // For now, return a placeholder
        Ok(Bytes::from(hex::decode("deadbeef").unwrap()))
    }
    
    fn get_arbitrage_contract_address(&self) -> Address {
        // In a real implementation, this would return the actual arbitrage contract address
        Address::zero()
    }
}

/// Backrun strategy bundle builder
pub struct BackrunBundleBuilder;

#[async_trait::async_trait]
impl StrategyBundleBuilder for BackrunBundleBuilder {
    async fn build_bundle_transactions(
        &self,
        opportunity: &Opportunity,
        _params: &BundleParams,
    ) -> Result<(Vec<TransactionTemplate>, Vec<TransactionTemplate>)> {
        match &opportunity.opportunity_type {
            OpportunityType::Backrun { affected_token, follow_up_action, .. } => {
                // For backrun, we don't need pre-transactions
                let pre_transactions = Vec::new();
                
                // Post-transaction: Execute the backrun action
                let mut post_transactions = Vec::new();
                
                let backrun_data = self.encode_backrun_call(affected_token, follow_up_action)?;
                let backrun_template = TransactionTemplate {
                    to: Some(self.get_backrun_contract_address()),
                    value: U256::zero(),
                    gas_limit: U256::from(250000),
                    data: backrun_data,
                    transaction_type: 2,
                };
                
                post_transactions.push(backrun_template);
                
                Ok((pre_transactions, post_transactions))
            }
            _ => Err(anyhow!("Invalid opportunity type for backrun builder")),
        }
    }
    
    fn validate_bundle(&self, bundle: &Bundle, _opportunity: &Opportunity) -> BundleValidation {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        // Validate backrun-specific requirements
        if bundle.post_transactions.is_empty() {
            errors.push("Backrun bundle must have post-transactions".to_string());
        }
        
        if !bundle.pre_transactions.is_empty() {
            warnings.push("Backrun typically doesn't need pre-transactions".to_string());
        }
        
        let estimated_gas_cost = bundle.total_gas_limit() * bundle.max_gas_price;
        
        BundleValidation {
            is_valid: errors.is_empty(),
            errors,
            warnings,
            estimated_gas_cost,
            estimated_profit: bundle.estimated_profit,
        }
    }
}

impl BackrunBundleBuilder {
    fn encode_backrun_call(&self, _affected_token: &str, _follow_up_action: &str) -> Result<Bytes> {
        // In a real implementation, this would encode the actual backrun function call
        Ok(Bytes::from(hex::decode("cafebabe").unwrap()))
    }
    
    fn get_backrun_contract_address(&self) -> Address {
        // In a real implementation, this would return the actual backrun contract address
        Address::zero()
    }
}

/// Sandwich strategy bundle builder
pub struct SandwichBundleBuilder;

#[async_trait::async_trait]
impl StrategyBundleBuilder for SandwichBundleBuilder {
    async fn build_bundle_transactions(
        &self,
        opportunity: &Opportunity,
        _params: &BundleParams,
    ) -> Result<(Vec<TransactionTemplate>, Vec<TransactionTemplate>)> {
        match &opportunity.opportunity_type {
            OpportunityType::Sandwich { token_pair, front_run_amount, back_run_amount, .. } => {
                // Pre-transaction: Front-run the victim
                let mut pre_transactions = Vec::new();
                let front_run_data = self.encode_front_run_call(token_pair, *front_run_amount)?;
                let front_run_template = TransactionTemplate {
                    to: Some(self.get_sandwich_contract_address()),
                    value: U256::zero(),
                    gas_limit: U256::from(200000),
                    data: front_run_data,
                    transaction_type: 2,
                };
                pre_transactions.push(front_run_template);
                
                // Post-transaction: Back-run the victim
                let mut post_transactions = Vec::new();
                let back_run_data = self.encode_back_run_call(token_pair, *back_run_amount)?;
                let back_run_template = TransactionTemplate {
                    to: Some(self.get_sandwich_contract_address()),
                    value: U256::zero(),
                    gas_limit: U256::from(200000),
                    data: back_run_data,
                    transaction_type: 2,
                };
                post_transactions.push(back_run_template);
                
                Ok((pre_transactions, post_transactions))
            }
            _ => Err(anyhow!("Invalid opportunity type for sandwich builder")),
        }
    }
    
    fn validate_bundle(&self, bundle: &Bundle, _opportunity: &Opportunity) -> BundleValidation {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        
        // Validate sandwich-specific requirements
        if bundle.pre_transactions.is_empty() {
            errors.push("Sandwich bundle must have pre-transactions (front-run)".to_string());
        }
        
        if bundle.post_transactions.is_empty() {
            errors.push("Sandwich bundle must have post-transactions (back-run)".to_string());
        }
        
        // Check for balanced front-run and back-run
        if bundle.pre_transactions.len() != bundle.post_transactions.len() {
            warnings.push("Unbalanced sandwich transactions".to_string());
        }
        
        let estimated_gas_cost = bundle.total_gas_limit() * bundle.max_gas_price;
        
        BundleValidation {
            is_valid: errors.is_empty(),
            errors,
            warnings,
            estimated_gas_cost,
            estimated_profit: bundle.estimated_profit,
        }
    }
}

impl SandwichBundleBuilder {
    fn encode_front_run_call(&self, _token_pair: &(String, String), _amount: u128) -> Result<Bytes> {
        // In a real implementation, this would encode the front-run function call
        Ok(Bytes::from(hex::decode("f00dface").unwrap()))
    }
    
    fn encode_back_run_call(&self, _token_pair: &(String, String), _amount: u128) -> Result<Bytes> {
        // In a real implementation, this would encode the back-run function call
        Ok(Bytes::from(hex::decode("deadc0de").unwrap()))
    }
    
    fn get_sandwich_contract_address(&self) -> Address {
        // In a real implementation, this would return the actual sandwich contract address
        Address::zero()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy_types::{Opportunity, OpportunityType};
    use crate::types::{ParsedTransaction, Transaction, TargetType};
    use crate::bundle::key_manager::KeyManager;
    use std::collections::HashMap;
    
    async fn create_test_bundle_builder() -> BundleBuilder {
        let key_manager = Arc::new(KeyManager::new());
        let private_key = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        key_manager.load_private_key(private_key).await.unwrap();
        
        let signer = Arc::new(TransactionSigner::new(key_manager, 998));
        let mut builder = BundleBuilder::new(signer);
        
        // Register strategy builders
        builder.register_strategy_builder(
            "arbitrage".to_string(),
            Box::new(ArbitrageBundleBuilder),
        );
        builder.register_strategy_builder(
            "backrun".to_string(),
            Box::new(BackrunBundleBuilder),
        );
        builder.register_strategy_builder(
            "sandwich".to_string(),
            Box::new(SandwichBundleBuilder),
        );
        
        builder
    }
    
    fn create_test_arbitrage_opportunity() -> Opportunity {
        Opportunity {
            id: "test_arb_1".to_string(),
            strategy_name: "arbitrage".to_string(),
            target_transaction: ParsedTransaction {
                transaction: Transaction {
                    hash: "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
                    from: "0x742d35Cc6634C0532925a3b8D4C9db96C4b5Da5e".to_string(),
                    to: Some("0x742d35Cc6634C0532925a3b8D4C9db96C4b5Da5e".to_string()),
                    value: "1000000000000000000".to_string(),
                    gas_price: "50000000000".to_string(),
                    gas_limit: "200000".to_string(),
                    nonce: 1,
                    input: "0x38ed1739".to_string(),
                    timestamp: chrono::Utc::now(),
                },
                decoded_input: None,
                target_type: TargetType::UniswapV2,
                processing_time_ms: 1,
            },
            opportunity_type: OpportunityType::Arbitrage {
                token_in: "WETH".to_string(),
                token_out: "USDC".to_string(),
                amount_in: 1000000000000000000,
                expected_profit: 50000000000000000,
                dex_path: vec!["UniswapV2".to_string(), "SushiSwap".to_string()],
            },
            estimated_profit_wei: 50000000000000000,
            estimated_gas_cost_wei: 5000000000000000,
            confidence_score: 0.85,
            priority: 150,
            expiry_timestamp: chrono::Utc::now().timestamp() as u64 + 30,
            metadata: HashMap::new(),
        }
    }
    
    #[tokio::test]
    async fn test_build_arbitrage_bundle() {
        let builder = create_test_bundle_builder().await;
        let opportunity = create_test_arbitrage_opportunity();
        
        let bundle = builder.build_bundle(&opportunity).await.unwrap();
        
        assert_eq!(bundle.pre_transactions.len(), 0);
        assert_eq!(bundle.post_transactions.len(), 1);
        assert_eq!(bundle.estimated_profit, U256::from(50000000000000000u64));
        assert!(bundle.metadata.contains_key("strategy"));
        assert_eq!(bundle.metadata.get("strategy").unwrap(), "arbitrage");
    }
    
    #[tokio::test]
    async fn test_build_sandwich_bundle() {
        let builder = create_test_bundle_builder().await;
        
        let opportunity = Opportunity {
            id: "test_sandwich_1".to_string(),
            strategy_name: "sandwich".to_string(),
            target_transaction: create_test_arbitrage_opportunity().target_transaction,
            opportunity_type: OpportunityType::Sandwich {
                victim_tx_hash: "0xvictim".to_string(),
                token_pair: ("WETH".to_string(), "USDC".to_string()),
                front_run_amount: 500000000000000000,
                back_run_amount: 500000000000000000,
                expected_profit: 25000000000000000,
            },
            estimated_profit_wei: 25000000000000000,
            estimated_gas_cost_wei: 8000000000000000,
            confidence_score: 0.75,
            priority: 200,
            expiry_timestamp: chrono::Utc::now().timestamp() as u64 + 15,
            metadata: HashMap::new(),
        };
        
        let bundle = builder.build_bundle(&opportunity).await.unwrap();
        
        assert_eq!(bundle.pre_transactions.len(), 1);  // Front-run
        assert_eq!(bundle.post_transactions.len(), 1); // Back-run
        assert_eq!(bundle.estimated_profit, U256::from(25000000000000000u64));
    }
    
    #[tokio::test]
    async fn test_bundle_validation() {
        let builder = ArbitrageBundleBuilder;
        let opportunity = create_test_arbitrage_opportunity();
        
        // Create a valid bundle
        let bundle = Bundle {
            id: "test_bundle".to_string(),
            pre_transactions: vec![],
            target_transaction: SignedTransaction {
                hash: ethers::types::H256::random(),
                from: Address::random(),
                to: Some(Address::random()),
                value: U256::from(1000000000000000000u64),
                gas_limit: U256::from(200000),
                max_fee_per_gas: U256::from(50000000000u64),
                max_priority_fee_per_gas: U256::from(2000000000u64),
                nonce: U256::from(1),
                data: Bytes::default(),
                chain_id: 998,
                raw_transaction: Bytes::default(),
                transaction_type: 2,
            },
            post_transactions: vec![SignedTransaction {
                hash: ethers::types::H256::random(),
                from: Address::random(),
                to: Some(Address::random()),
                value: U256::zero(),
                gas_limit: U256::from(300000),
                max_fee_per_gas: U256::from(50000000000u64),
                max_priority_fee_per_gas: U256::from(2000000000u64),
                nonce: U256::from(2),
                data: Bytes::default(),
                chain_id: 998,
                raw_transaction: Bytes::default(),
                transaction_type: 2,
            }],
            target_block: 1000,
            max_gas_price: U256::from(100000000000u64),
            estimated_profit: U256::from(50000000000000000u64),
            created_at: chrono::Utc::now(),
            metadata: HashMap::new(),
        };
        
        let validation = builder.validate_bundle(&bundle, &opportunity);
        assert!(validation.is_valid);
        assert!(validation.errors.is_empty());
    }
}