//! Bundle type definitions

use ethers::types::{Address, Bytes, H256, U256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A complete MEV bundle with pre, target, and post transactions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    /// Unique bundle identifier
    pub id: String,
    
    /// Transactions to execute before the target transaction
    pub pre_transactions: Vec<SignedTransaction>,
    
    /// The target transaction we're MEV-ing against
    pub target_transaction: SignedTransaction,
    
    /// Transactions to execute after the target transaction
    pub post_transactions: Vec<SignedTransaction>,
    
    /// Target block number for inclusion
    pub target_block: u64,
    
    /// Maximum gas price willing to pay
    pub max_gas_price: U256,
    
    /// Estimated profit from this bundle
    pub estimated_profit: U256,
    
    /// Bundle creation timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
    
    /// Bundle metadata
    pub metadata: HashMap<String, String>,
}

/// A signed transaction ready for submission
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedTransaction {
    /// Transaction hash
    pub hash: H256,
    
    /// From address
    pub from: Address,
    
    /// To address (None for contract creation)
    pub to: Option<Address>,
    
    /// Transaction value in wei
    pub value: U256,
    
    /// Gas limit
    pub gas_limit: U256,
    
    /// Max fee per gas (EIP-1559)
    pub max_fee_per_gas: U256,
    
    /// Max priority fee per gas (EIP-1559)
    pub max_priority_fee_per_gas: U256,
    
    /// Transaction nonce
    pub nonce: U256,
    
    /// Transaction data
    pub data: Bytes,
    
    /// Chain ID
    pub chain_id: u64,
    
    /// Raw signed transaction bytes
    pub raw_transaction: Bytes,
    
    /// Transaction type (0 = legacy, 1 = EIP-2930, 2 = EIP-1559)
    pub transaction_type: u8,
}

/// Transaction template for pre-signing optimization
#[derive(Debug, Clone)]
pub struct TransactionTemplate {
    /// To address (None for contract creation)
    pub to: Option<Address>,
    
    /// Transaction value in wei
    pub value: U256,
    
    /// Gas limit
    pub gas_limit: U256,
    
    /// Transaction data
    pub data: Bytes,
    
    /// Transaction type (0 = legacy, 1 = EIP-2930, 2 = EIP-1559)
    pub transaction_type: u8,
}

/// Bundle construction parameters
#[derive(Debug, Clone)]
pub struct BundleParams {
    /// Strategy that generated this bundle
    pub strategy_name: String,
    
    /// Target block for inclusion
    pub target_block: u64,
    
    /// Maximum gas price willing to pay
    pub max_gas_price: U256,
    
    /// Maximum priority fee per gas
    pub max_priority_fee: U256,
    
    /// Gas price multiplier for competitive bidding
    pub gas_multiplier: f64,
    
    /// Minimum profit threshold
    pub min_profit_wei: U256,
    
    /// Bundle expiry time
    pub expiry_seconds: u64,
}

impl Default for BundleParams {
    fn default() -> Self {
        Self {
            strategy_name: "unknown".to_string(),
            target_block: 0,
            max_gas_price: U256::from(200_000_000_000u64), // 200 gwei
            max_priority_fee: U256::from(2_000_000_000u64), // 2 gwei
            gas_multiplier: 1.1,
            min_profit_wei: U256::from(1_000_000_000_000_000u64), // 0.001 ETH
            expiry_seconds: 30,
        }
    }
}

/// Nonce management strategy
#[derive(Debug, Clone, Copy)]
pub enum NonceStrategy {
    /// Use the next available nonce from the network
    Network,
    /// Use a specific nonce value
    Fixed(u64),
    /// Use the next nonce in sequence from local tracking
    Sequential,
}

/// Bundle validation result
#[derive(Debug, Clone)]
pub struct BundleValidation {
    pub is_valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub estimated_gas_cost: U256,
    pub estimated_profit: U256,
}

impl Bundle {
    /// Get all transactions in execution order
    pub fn all_transactions(&self) -> Vec<&SignedTransaction> {
        let mut txs = Vec::new();
        txs.extend(&self.pre_transactions);
        txs.push(&self.target_transaction);
        txs.extend(&self.post_transactions);
        txs
    }
    
    /// Calculate total gas limit for the bundle
    pub fn total_gas_limit(&self) -> U256 {
        self.all_transactions()
            .iter()
            .map(|tx| tx.gas_limit)
            .fold(U256::zero(), |acc, gas| acc + gas)
    }
    
    /// Calculate total value transferred in the bundle
    pub fn total_value(&self) -> U256 {
        self.all_transactions()
            .iter()
            .map(|tx| tx.value)
            .fold(U256::zero(), |acc, value| acc + value)
    }
    
    /// Check if bundle has expired
    pub fn is_expired(&self) -> bool {
        let now = chrono::Utc::now();
        let expiry_seconds = self.metadata
            .get("expiry_seconds")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(30);
        
        now.signed_duration_since(self.created_at).num_seconds() > expiry_seconds
    }
}

impl SignedTransaction {
    /// Get the transaction hash as a hex string
    pub fn hash_hex(&self) -> String {
        format!("0x{:x}", self.hash)
    }
    
    /// Get the from address as a hex string
    pub fn from_hex(&self) -> String {
        format!("0x{:x}", self.from)
    }
    
    /// Get the to address as a hex string (if present)
    pub fn to_hex(&self) -> Option<String> {
        self.to.map(|addr| format!("0x{:x}", addr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn create_test_signed_transaction() -> SignedTransaction {
        SignedTransaction {
            hash: H256::random(),
            from: Address::random(),
            to: Some(Address::random()),
            value: U256::from(1000000000000000000u64), // 1 ETH
            gas_limit: U256::from(21000),
            max_fee_per_gas: U256::from(50000000000u64), // 50 gwei
            max_priority_fee_per_gas: U256::from(2000000000u64), // 2 gwei
            nonce: U256::from(1),
            data: Bytes::default(),
            chain_id: 998, // HyperEVM
            raw_transaction: Bytes::default(),
            transaction_type: 2, // EIP-1559
        }
    }
    
    #[test]
    fn test_bundle_creation() {
        let pre_tx = create_test_signed_transaction();
        let target_tx = create_test_signed_transaction();
        let post_tx = create_test_signed_transaction();
        
        let bundle = Bundle {
            id: "test_bundle".to_string(),
            pre_transactions: vec![pre_tx],
            target_transaction: target_tx,
            post_transactions: vec![post_tx],
            target_block: 1000,
            max_gas_price: U256::from(100000000000u64),
            estimated_profit: U256::from(50000000000000000u64),
            created_at: chrono::Utc::now(),
            metadata: HashMap::new(),
        };
        
        assert_eq!(bundle.all_transactions().len(), 3);
        assert_eq!(bundle.total_gas_limit(), U256::from(63000)); // 3 * 21000
        assert_eq!(bundle.total_value(), U256::from(3000000000000000000u64)); // 3 ETH
    }
    
    #[test]
    fn test_bundle_expiry() {
        let mut bundle = Bundle {
            id: "test_bundle".to_string(),
            pre_transactions: vec![],
            target_transaction: create_test_signed_transaction(),
            post_transactions: vec![],
            target_block: 1000,
            max_gas_price: U256::from(100000000000u64),
            estimated_profit: U256::from(50000000000000000u64),
            created_at: chrono::Utc::now() - chrono::Duration::seconds(60),
            metadata: HashMap::new(),
        };
        
        // Should be expired with default 30 second expiry
        assert!(bundle.is_expired());
        
        // Set longer expiry
        bundle.metadata.insert("expiry_seconds".to_string(), "120".to_string());
        assert!(!bundle.is_expired());
    }
}