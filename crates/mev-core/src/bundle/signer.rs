//! Transaction signing infrastructure with EIP-1559 support

use super::types::{SignedTransaction, TransactionTemplate, NonceStrategy};
use super::key_manager::KeyManager;
use anyhow::{anyhow, Result};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{
    transaction::{eip1559::Eip1559TransactionRequest, eip2718::TypedTransaction},
    Address, Bytes, U256,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Transaction signer with EIP-1559 support and pre-signing optimization
pub struct TransactionSigner {
    /// Key manager for wallet access
    pub key_manager: Arc<KeyManager>,
    /// Nonce tracking by address
    nonce_tracker: Arc<RwLock<HashMap<Address, U256>>>,
    /// Chain ID for transaction signing
    chain_id: u64,
    /// Pre-signed transaction templates for optimization
    template_cache: Arc<RwLock<HashMap<String, SignedTransactionTemplate>>>,
}

/// Pre-signed transaction template for optimization
#[derive(Debug, Clone)]
struct SignedTransactionTemplate {
    /// Template transaction
    template: TransactionTemplate,
    /// Signer address
    signer: Address,
    /// Base gas price for template
    base_gas_price: U256,
    /// Template creation time
    created_at: chrono::DateTime<chrono::Utc>,
}

impl TransactionSigner {
    /// Create a new transaction signer
    pub fn new(key_manager: Arc<KeyManager>, chain_id: u64) -> Self {
        Self {
            key_manager,
            nonce_tracker: Arc::new(RwLock::new(HashMap::new())),
            chain_id,
            template_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Sign a transaction using the default wallet
    pub async fn sign_transaction(
        &self,
        template: &TransactionTemplate,
        nonce_strategy: NonceStrategy,
        max_fee_per_gas: U256,
        max_priority_fee_per_gas: U256,
    ) -> Result<SignedTransaction> {
        let wallet = self.key_manager.get_default_wallet().await?;
        self.sign_transaction_with_wallet(
            &wallet,
            template,
            nonce_strategy,
            max_fee_per_gas,
            max_priority_fee_per_gas,
        ).await
    }
    
    /// Sign a transaction using a specific wallet
    pub async fn sign_transaction_with_wallet(
        &self,
        wallet: &LocalWallet,
        template: &TransactionTemplate,
        nonce_strategy: NonceStrategy,
        max_fee_per_gas: U256,
        max_priority_fee_per_gas: U256,
    ) -> Result<SignedTransaction> {
        let from = wallet.address();
        let nonce = self.get_nonce(from, nonce_strategy).await?;
        
        // Create EIP-1559 transaction
        let mut tx = Eip1559TransactionRequest::new()
            .to(template.to.unwrap_or_default())
            .value(template.value)
            .gas(template.gas_limit)
            .max_fee_per_gas(max_fee_per_gas)
            .max_priority_fee_per_gas(max_priority_fee_per_gas)
            .nonce(nonce)
            .data(template.data.clone())
            .chain_id(self.chain_id);
        
        // Handle contract creation (to = None)
        if template.to.is_none() {
            tx = tx.to(Address::zero()); // Will be converted to None during signing
        }
        
        // Sign the transaction
        let typed_tx = TypedTransaction::Eip1559(tx);
        let signature = wallet.sign_transaction(&typed_tx).await?;
        
        // Create signed transaction
        let signed_tx = SignedTransaction {
            hash: typed_tx.hash(&signature),
            from,
            to: template.to,
            value: template.value,
            gas_limit: template.gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            nonce,
            data: template.data.clone(),
            chain_id: self.chain_id,
            raw_transaction: typed_tx.rlp_signed(&signature),
            transaction_type: 2, // EIP-1559
        };
        
        // Update nonce tracker
        self.update_nonce(from, nonce + 1).await;
        
        Ok(signed_tx)
    }
    
    /// Sign a transaction using a specific address
    pub async fn sign_transaction_with_address(
        &self,
        signer_address: &Address,
        template: &TransactionTemplate,
        nonce_strategy: NonceStrategy,
        max_fee_per_gas: U256,
        max_priority_fee_per_gas: U256,
    ) -> Result<SignedTransaction> {
        let wallet = self.key_manager.get_wallet(signer_address).await?;
        self.sign_transaction_with_wallet(
            &wallet,
            template,
            nonce_strategy,
            max_fee_per_gas,
            max_priority_fee_per_gas,
        ).await
    }
    
    /// Pre-sign a transaction template for optimization
    pub async fn create_template(
        &self,
        template: TransactionTemplate,
        base_gas_price: U256,
    ) -> Result<String> {
        let wallet = self.key_manager.get_default_wallet().await?;
        let template_id = format!("{}_{}", 
            hex::encode(template.data.as_ref()),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        );
        
        let signed_template = SignedTransactionTemplate {
            template,
            signer: wallet.address(),
            base_gas_price,
            created_at: chrono::Utc::now(),
        };
        
        let mut cache = self.template_cache.write().await;
        cache.insert(template_id.clone(), signed_template);
        
        Ok(template_id)
    }
    
    /// Sign a transaction from a pre-created template
    pub async fn sign_from_template(
        &self,
        template_id: &str,
        nonce_strategy: NonceStrategy,
        gas_multiplier: f64,
    ) -> Result<SignedTransaction> {
        let template = {
            let cache = self.template_cache.read().await;
            cache.get(template_id)
                .ok_or_else(|| anyhow!("Template not found: {}", template_id))?
                .clone()
        };
        
        // Calculate gas prices with multiplier
        let max_fee_per_gas = U256::from(
            (template.base_gas_price.as_u128() as f64 * gas_multiplier) as u128
        );
        let max_priority_fee_per_gas = U256::from(2_000_000_000u64); // 2 gwei
        
        let wallet = self.key_manager.get_wallet(&template.signer).await?;
        self.sign_transaction_with_wallet(
            &wallet,
            &template.template,
            nonce_strategy,
            max_fee_per_gas,
            max_priority_fee_per_gas,
        ).await
    }
    
    /// Get nonce for an address based on strategy
    async fn get_nonce(&self, address: Address, strategy: NonceStrategy) -> Result<U256> {
        match strategy {
            NonceStrategy::Fixed(nonce) => Ok(U256::from(nonce)),
            NonceStrategy::Sequential => {
                let mut tracker = self.nonce_tracker.write().await;
                let current_nonce = tracker.get(&address).copied().unwrap_or(U256::zero());
                tracker.insert(address, current_nonce + 1);
                Ok(current_nonce)
            }
            NonceStrategy::Network => {
                // In a real implementation, this would query the network
                // For now, use sequential as fallback
                let mut tracker = self.nonce_tracker.write().await;
                let current_nonce = tracker.get(&address).copied().unwrap_or(U256::zero());
                tracker.insert(address, current_nonce + 1);
                Ok(current_nonce)
            }
        }
    }
    
    /// Update nonce tracker
    async fn update_nonce(&self, address: Address, nonce: U256) {
        let mut tracker = self.nonce_tracker.write().await;
        tracker.insert(address, nonce);
    }
    
    /// Reset nonce for an address
    pub async fn reset_nonce(&self, address: Address, nonce: U256) {
        self.update_nonce(address, nonce).await;
    }
    
    /// Get current tracked nonce for an address
    pub async fn get_tracked_nonce(&self, address: Address) -> Option<U256> {
        let tracker = self.nonce_tracker.read().await;
        tracker.get(&address).copied()
    }
    
    /// Clear expired templates from cache
    pub async fn cleanup_templates(&self, max_age_seconds: i64) {
        let mut cache = self.template_cache.write().await;
        let now = chrono::Utc::now();
        
        cache.retain(|_, template| {
            now.signed_duration_since(template.created_at).num_seconds() < max_age_seconds
        });
    }
    
    /// Get template cache statistics
    pub async fn get_template_stats(&self) -> (usize, usize) {
        let cache = self.template_cache.read().await;
        let total = cache.len();
        let expired = cache.values()
            .filter(|template| {
                let now = chrono::Utc::now();
                now.signed_duration_since(template.created_at).num_seconds() > 300 // 5 minutes
            })
            .count();
        
        (total, expired)
    }
}

/// Helper functions for creating common transaction templates
impl TransactionSigner {
    /// Create a simple ETH transfer template
    pub fn create_eth_transfer_template(
        to: Address,
        value: U256,
        gas_limit: Option<U256>,
    ) -> TransactionTemplate {
        TransactionTemplate {
            to: Some(to),
            value,
            gas_limit: gas_limit.unwrap_or(U256::from(21000)),
            data: Bytes::default(),
            transaction_type: 2, // EIP-1559
        }
    }
    
    /// Create a contract call template
    pub fn create_contract_call_template(
        to: Address,
        data: Bytes,
        value: Option<U256>,
        gas_limit: Option<U256>,
    ) -> TransactionTemplate {
        TransactionTemplate {
            to: Some(to),
            value: value.unwrap_or(U256::zero()),
            gas_limit: gas_limit.unwrap_or(U256::from(200000)),
            data,
            transaction_type: 2, // EIP-1559
        }
    }
    
    /// Create a contract deployment template
    pub fn create_contract_deployment_template(
        bytecode: Bytes,
        value: Option<U256>,
        gas_limit: Option<U256>,
    ) -> TransactionTemplate {
        TransactionTemplate {
            to: None, // Contract creation
            value: value.unwrap_or(U256::zero()),
            gas_limit: gas_limit.unwrap_or(U256::from(1000000)),
            data: bytecode,
            transaction_type: 2, // EIP-1559
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    
    async fn create_test_signer() -> TransactionSigner {
        let key_manager = Arc::new(KeyManager::new());
        
        // Load test private key
        let private_key = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        key_manager.load_private_key(private_key).await.unwrap();
        
        TransactionSigner::new(key_manager, 998) // HyperEVM chain ID
    }
    
    #[tokio::test]
    async fn test_sign_eth_transfer() {
        let signer = create_test_signer().await;
        
        let template = TransactionSigner::create_eth_transfer_template(
            Address::random(),
            U256::from(1000000000000000000u64), // 1 ETH
            None,
        );
        
        let signed_tx = signer.sign_transaction(
            &template,
            NonceStrategy::Sequential,
            U256::from(50000000000u64), // 50 gwei
            U256::from(2000000000u64),  // 2 gwei
        ).await.unwrap();
        
        assert_eq!(signed_tx.transaction_type, 2);
        assert_eq!(signed_tx.chain_id, 998);
        assert_eq!(signed_tx.value, U256::from(1000000000000000000u64));
        assert_eq!(signed_tx.nonce, U256::zero());
    }
    
    #[tokio::test]
    async fn test_nonce_tracking() {
        let signer = create_test_signer().await;
        let wallet = signer.key_manager.get_default_wallet().await.unwrap();
        let address = wallet.address();
        
        let template = TransactionSigner::create_eth_transfer_template(
            Address::random(),
            U256::from(1000000000000000000u64),
            None,
        );
        
        // Sign multiple transactions
        for i in 0..3 {
            let signed_tx = signer.sign_transaction(
                &template,
                NonceStrategy::Sequential,
                U256::from(50000000000u64),
                U256::from(2000000000u64),
            ).await.unwrap();
            
            assert_eq!(signed_tx.nonce, U256::from(i));
        }
        
        // Check tracked nonce
        let tracked_nonce = signer.get_tracked_nonce(address).await.unwrap();
        assert_eq!(tracked_nonce, U256::from(3));
    }
    
    #[tokio::test]
    async fn test_template_caching() {
        let signer = create_test_signer().await;
        
        let template = TransactionSigner::create_contract_call_template(
            Address::random(),
            Bytes::from(hex::decode("a9059cbb").unwrap()), // transfer function
            None,
            None,
        );
        
        // Create template
        let template_id = signer.create_template(
            template,
            U256::from(50000000000u64),
        ).await.unwrap();
        
        // Sign from template
        let signed_tx = signer.sign_from_template(
            &template_id,
            NonceStrategy::Sequential,
            1.2, // 20% gas multiplier
        ).await.unwrap();
        
        assert_eq!(signed_tx.transaction_type, 2);
        assert_eq!(signed_tx.max_fee_per_gas, U256::from(60000000000u64)); // 50 * 1.2
    }
    
    #[tokio::test]
    async fn test_template_cleanup() {
        let signer = create_test_signer().await;
        
        let template = TransactionSigner::create_eth_transfer_template(
            Address::random(),
            U256::from(1000000000000000000u64),
            None,
        );
        
        // Create multiple templates
        for _ in 0..5 {
            signer.create_template(
                template.clone(),
                U256::from(50000000000u64),
            ).await.unwrap();
        }
        
        let (total, _) = signer.get_template_stats().await;
        assert_eq!(total, 5);
        
        // Cleanup with very short max age (should remove all)
        signer.cleanup_templates(0).await;
        
        let (total_after, _) = signer.get_template_stats().await;
        assert_eq!(total_after, 0);
    }
}