//! Integration tests for the complete bundle construction and signing system

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::bundle::builder::{ArbitrageBundleBuilder, BackrunBundleBuilder, SandwichBundleBuilder, StrategyBundleBuilder};
    use crate::strategy_types::{Opportunity, OpportunityType};
    use crate::types::{ParsedTransaction, Transaction, TargetType};
    use ethers::signers::Signer;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tempfile::tempdir;

    /// Create a complete test setup with key manager, signer, and bundle builder
    async fn create_test_setup() -> (Arc<KeyManager>, Arc<TransactionSigner>, BundleBuilder) {
        let key_manager = Arc::new(KeyManager::new());
        
        // Load test private key
        let private_key = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        key_manager.load_private_key(private_key).await.unwrap();
        
        let signer = Arc::new(TransactionSigner::new(key_manager.clone(), 998));
        let mut builder = BundleBuilder::new(signer.clone());
        
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
        
        (key_manager, signer, builder)
    }
    
    fn create_test_opportunity(strategy: &str, opportunity_type: OpportunityType) -> Opportunity {
        Opportunity {
            id: format!("test_{}_1", strategy),
            strategy_name: strategy.to_string(),
            target_transaction: ParsedTransaction {
                transaction: Transaction {
                    hash: "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
                    from: "0x742d35Cc6634C0532925a3b8D4C9db96C4b5Da5e".to_string(),
                    to: Some("0x742d35Cc6634C0532925a3b8D4C9db96C4b5Da5e".to_string()),
                    value: "1000000000000000000".to_string(), // 1 ETH
                    gas_price: "50000000000".to_string(), // 50 gwei
                    gas_limit: "200000".to_string(),
                    nonce: 1,
                    input: "0x38ed1739".to_string(),
                    timestamp: chrono::Utc::now(),
                },
                decoded_input: None,
                target_type: TargetType::UniswapV2,
                processing_time_ms: 1,
            },
            opportunity_type,
            estimated_profit_wei: 50000000000000000, // 0.05 ETH
            estimated_gas_cost_wei: 5000000000000000, // 0.005 ETH
            confidence_score: 0.85,
            priority: 150,
            expiry_timestamp: chrono::Utc::now().timestamp() as u64 + 30,
            metadata: HashMap::new(),
        }
    }
    
    #[tokio::test]
    async fn test_complete_arbitrage_bundle_flow() {
        let (_key_manager, _signer, builder) = create_test_setup().await;
        
        let opportunity = create_test_opportunity(
            "arbitrage",
            OpportunityType::Arbitrage {
                token_in: "WETH".to_string(),
                token_out: "USDC".to_string(),
                amount_in: 1000000000000000000,
                expected_profit: 50000000000000000,
                dex_path: vec!["UniswapV2".to_string(), "SushiSwap".to_string()],
            },
        );
        
        // Build the bundle
        let bundle = builder.build_bundle(&opportunity).await.unwrap();
        
        // Verify bundle structure
        assert_eq!(bundle.pre_transactions.len(), 0);
        assert_eq!(bundle.post_transactions.len(), 1);
        assert_eq!(bundle.estimated_profit, ethers::types::U256::from(50000000000000000u64));
        
        // Verify metadata
        assert_eq!(bundle.metadata.get("strategy").unwrap(), "arbitrage");
        assert_eq!(bundle.metadata.get("opportunity_id").unwrap(), "test_arbitrage_1");
        
        // Verify transaction signing
        let post_tx = &bundle.post_transactions[0];
        assert_eq!(post_tx.transaction_type, 2); // EIP-1559
        assert_eq!(post_tx.chain_id, 998); // HyperEVM
        assert!(post_tx.max_fee_per_gas > ethers::types::U256::zero());
        
        // Verify bundle is not expired
        assert!(!bundle.is_expired());
    }
    
    #[tokio::test]
    async fn test_complete_sandwich_bundle_flow() {
        let (_key_manager, _signer, builder) = create_test_setup().await;
        
        let opportunity = create_test_opportunity(
            "sandwich",
            OpportunityType::Sandwich {
                victim_tx_hash: "0xvictim".to_string(),
                token_pair: ("WETH".to_string(), "USDC".to_string()),
                front_run_amount: 500000000000000000,
                back_run_amount: 500000000000000000,
                expected_profit: 25000000000000000,
            },
        );
        
        // Build the bundle
        let bundle = builder.build_bundle(&opportunity).await.unwrap();
        
        // Verify sandwich structure (front-run + victim + back-run)
        assert_eq!(bundle.pre_transactions.len(), 1);  // Front-run
        assert_eq!(bundle.post_transactions.len(), 1); // Back-run
        
        // Verify nonce sequencing
        let front_run_nonce = bundle.pre_transactions[0].nonce;
        let back_run_nonce = bundle.post_transactions[0].nonce;
        assert!(back_run_nonce > front_run_nonce);
        
        // Verify all transactions are properly signed
        for tx in bundle.all_transactions() {
            assert_eq!(tx.transaction_type, 2); // EIP-1559
            assert_eq!(tx.chain_id, 998);
            assert!(tx.max_fee_per_gas > ethers::types::U256::zero());
        }
    }
    
    #[tokio::test]
    async fn test_key_management_integration() {
        let temp_dir = tempdir().unwrap();
        let key_file = temp_dir.path().join("test_key.json");
        
        let key_manager = Arc::new(KeyManager::with_key_dir(&temp_dir));
        let password = "test_password_123";
        
        // Create encrypted key
        let address = key_manager
            .create_encrypted_key(&key_file, password)
            .await
            .unwrap();
        
        // Create signer with encrypted key
        let signer = Arc::new(TransactionSigner::new(key_manager.clone(), 998));
        
        // Test transaction signing
        let template = TransactionSigner::create_eth_transfer_template(
            ethers::types::Address::random(),
            ethers::types::U256::from(1000000000000000000u64), // 1 ETH
            None,
        );
        
        let signed_tx = signer.sign_transaction(
            &template,
            NonceStrategy::Sequential,
            ethers::types::U256::from(50000000000u64), // 50 gwei
            ethers::types::U256::from(2000000000u64),  // 2 gwei
        ).await.unwrap();
        
        // Verify the transaction was signed by the correct address
        assert_eq!(signed_tx.from, address);
        assert_eq!(signed_tx.value, ethers::types::U256::from(1000000000000000000u64));
        
        // Test loading the key in a new key manager
        let key_manager2 = Arc::new(KeyManager::new());
        let loaded_address = key_manager2
            .load_encrypted_key(&key_file, password)
            .await
            .unwrap();
        
        assert_eq!(address, loaded_address);
    }
    
    #[tokio::test]
    async fn test_transaction_template_optimization() {
        let (_key_manager, signer, _builder) = create_test_setup().await;
        
        // Create a template for optimization
        let template = TransactionSigner::create_contract_call_template(
            ethers::types::Address::random(),
            ethers::types::Bytes::from(hex::decode("a9059cbb").unwrap()), // transfer function
            None,
            None,
        );
        
        // Pre-sign template
        let template_id = signer.create_template(
            template,
            ethers::types::U256::from(50000000000u64),
        ).await.unwrap();
        
        // Sign multiple transactions from template with different gas multipliers
        let tx1 = signer.sign_from_template(
            &template_id,
            NonceStrategy::Sequential,
            1.0, // No multiplier
        ).await.unwrap();
        
        let tx2 = signer.sign_from_template(
            &template_id,
            NonceStrategy::Sequential,
            1.5, // 50% increase
        ).await.unwrap();
        
        // Verify gas price scaling
        assert_eq!(tx1.max_fee_per_gas, ethers::types::U256::from(50000000000u64));
        assert_eq!(tx2.max_fee_per_gas, ethers::types::U256::from(75000000000u64));
        
        // Verify nonce sequencing
        assert_eq!(tx1.nonce, ethers::types::U256::from(0));
        assert_eq!(tx2.nonce, ethers::types::U256::from(1));
        
        // Test template cleanup
        let (total_before, _) = signer.get_template_stats().await;
        assert_eq!(total_before, 1);
        
        signer.cleanup_templates(0).await; // Remove all templates
        
        let (total_after, _) = signer.get_template_stats().await;
        assert_eq!(total_after, 0);
    }
    
    #[tokio::test]
    async fn test_bundle_validation_and_error_handling() {
        let (_key_manager, _signer, builder) = create_test_setup().await;
        
        // Test with invalid opportunity (missing strategy builder)
        let invalid_opportunity = create_test_opportunity(
            "unknown_strategy",
            OpportunityType::Arbitrage {
                token_in: "WETH".to_string(),
                token_out: "USDC".to_string(),
                amount_in: 1000000000000000000,
                expected_profit: 50000000000000000,
                dex_path: vec!["UniswapV2".to_string()],
            },
        );
        
        let result = builder.build_bundle(&invalid_opportunity).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No builder registered"));
        
        // Test bundle validation
        let arbitrage_builder = ArbitrageBundleBuilder;
        let valid_opportunity = create_test_opportunity(
            "arbitrage",
            OpportunityType::Arbitrage {
                token_in: "WETH".to_string(),
                token_out: "USDC".to_string(),
                amount_in: 1000000000000000000,
                expected_profit: 50000000000000000,
                dex_path: vec!["UniswapV2".to_string()],
            },
        );
        
        // Create a valid bundle for validation
        let bundle = builder.build_bundle(&valid_opportunity).await.unwrap();
        let validation = arbitrage_builder.validate_bundle(&bundle, &valid_opportunity);
        
        assert!(validation.is_valid);
        assert!(validation.errors.is_empty());
        assert!(validation.estimated_gas_cost > ethers::types::U256::zero());
    }
    
    #[tokio::test]
    async fn test_nonce_management_strategies() {
        let (_key_manager, signer, _builder) = create_test_setup().await;
        
        let template = TransactionSigner::create_eth_transfer_template(
            ethers::types::Address::random(),
            ethers::types::U256::from(1000000000000000000u64),
            None,
        );
        
        // Test fixed nonce strategy
        let tx_fixed = signer.sign_transaction(
            &template,
            NonceStrategy::Fixed(42),
            ethers::types::U256::from(50000000000u64),
            ethers::types::U256::from(2000000000u64),
        ).await.unwrap();
        
        assert_eq!(tx_fixed.nonce, ethers::types::U256::from(42));
        
        // Test sequential nonce strategy
        let tx_seq1 = signer.sign_transaction(
            &template,
            NonceStrategy::Sequential,
            ethers::types::U256::from(50000000000u64),
            ethers::types::U256::from(2000000000u64),
        ).await.unwrap();
        
        let tx_seq2 = signer.sign_transaction(
            &template,
            NonceStrategy::Sequential,
            ethers::types::U256::from(50000000000u64),
            ethers::types::U256::from(2000000000u64),
        ).await.unwrap();
        
        // Sequential nonces should increment
        assert_eq!(tx_seq2.nonce, tx_seq1.nonce + 1);
        
        // Test nonce reset
        let wallet = signer.key_manager.get_default_wallet().await.unwrap();
        let address = wallet.address();
        
        signer.reset_nonce(address, ethers::types::U256::from(100)).await;
        
        let tx_after_reset = signer.sign_transaction(
            &template,
            NonceStrategy::Sequential,
            ethers::types::U256::from(50000000000u64),
            ethers::types::U256::from(2000000000u64),
        ).await.unwrap();
        
        assert_eq!(tx_after_reset.nonce, ethers::types::U256::from(100));
    }
}