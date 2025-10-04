//! Comprehensive unit tests for OpportunityDetector
//!
//! This module contains unit tests for the OpportunityDetector component,
//! covering market data processing, state updates, opportunity detection,
//! RPC verification, and error handling.

#[cfg(test)]
mod tests {
    use crate::opportunity_detector::{OpportunityDetector, OpportunityDetectorConfig};
    use crate::rpc_service::{HyperLiquidRpcService, RpcConfig};
    use crate::types::*;
    use ethers::types::{Address, U256};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    /// Helper function to create a test RPC service
    fn create_test_rpc_service() -> Arc<HyperLiquidRpcService> {
        let config = RpcConfig::new("http://localhost:8545".to_string(), 1000);
        Arc::new(HyperLiquidRpcService::new(config, None))
    }

    /// Helper function to create test channels and detector
    fn create_test_detector() -> (
        mpsc::UnboundedSender<MarketDataEvent>,
        mpsc::UnboundedSender<StateUpdateEvent>,
        mpsc::UnboundedReceiver<Opportunity>,
        OpportunityDetector,
    ) {
        let (market_data_tx, market_data_rx) = mpsc::unbounded_channel();
        let (state_update_tx, state_update_rx) = mpsc::unbounded_channel();
        let (opportunity_tx, opportunity_rx) = mpsc::unbounded_channel();
        let rpc_service = create_test_rpc_service();

        let detector = OpportunityDetector::new(
            market_data_rx,
            state_update_rx,
            rpc_service,
            opportunity_tx,
        );

        (market_data_tx, state_update_tx, opportunity_rx, detector)
    }

    /// Helper function to create test detector with custom config
    fn create_test_detector_with_config(
        config: OpportunityDetectorConfig,
    ) -> (
        mpsc::UnboundedSender<MarketDataEvent>,
        mpsc::UnboundedSender<StateUpdateEvent>,
        mpsc::UnboundedReceiver<Opportunity>,
        OpportunityDetector,
    ) {
        let (market_data_tx, market_data_rx) = mpsc::unbounded_channel();
        let (state_update_tx, state_update_rx) = mpsc::unbounded_channel();
        let (opportunity_tx, opportunity_rx) = mpsc::unbounded_channel();
        let rpc_service = create_test_rpc_service();

        let detector = OpportunityDetector::with_config(
            market_data_rx,
            state_update_rx,
            rpc_service,
            opportunity_tx,
            config,
        );

        (market_data_tx, state_update_tx, opportunity_rx, detector)
    }

    /// Helper to get current timestamp in milliseconds
    fn current_timestamp_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    // ========================================================================
    // Market Data Event Processing Tests
    // ========================================================================

    #[tokio::test]
    async fn test_process_trade_updates_market_price() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 45000.0,
            size: 1.0,
            timestamp: current_timestamp_ms(),
            trade_id: "trade_123".to_string(),
        };

        let market_data = MarketDataEvent::Trade(trade.clone());
        detector.process_market_data(market_data).await.unwrap();

        // Verify price was updated
        assert_eq!(detector.market_prices().get("BTC"), Some(&45000.0));
    }

    #[tokio::test]
    async fn test_process_multiple_trades_for_same_coin() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        // First trade
        let trade1 = TradeData {
            coin: "ETH".to_string(),
            side: TradeSide::Buy,
            price: 3000.0,
            size: 5.0,
            timestamp: current_timestamp_ms(),
            trade_id: "trade_1".to_string(),
        };
        detector
            .process_market_data(MarketDataEvent::Trade(trade1))
            .await
            .unwrap();

        // Second trade with different price
        let trade2 = TradeData {
            coin: "ETH".to_string(),
            side: TradeSide::Sell,
            price: 3050.0,
            size: 2.0,
            timestamp: current_timestamp_ms(),
            trade_id: "trade_2".to_string(),
        };
        detector
            .process_market_data(MarketDataEvent::Trade(trade2))
            .await
            .unwrap();

        // Should have the latest price
        assert_eq!(detector.market_prices().get("ETH"), Some(&3050.0));
    }

    #[tokio::test]
    async fn test_process_orderbook_updates_cache() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let orderbook = OrderBookData {
            coin: "SOL".to_string(),
            bids: vec![
                OrderBookLevel::new(100.0, 50.0),
                OrderBookLevel::new(99.5, 30.0),
            ],
            asks: vec![
                OrderBookLevel::new(100.5, 40.0),
                OrderBookLevel::new(101.0, 60.0),
            ],
            timestamp: current_timestamp_ms(),
        };

        let market_data = MarketDataEvent::OrderBook(orderbook.clone());
        detector.process_market_data(market_data).await.unwrap();

        // Verify orderbook was cached
        assert!(detector.order_books().contains_key("SOL"));
        let cached = detector.order_books().get("SOL").unwrap();
        assert_eq!(cached.best_bid(), Some(100.0));
        assert_eq!(cached.best_ask(), Some(100.5));
    }

    #[tokio::test]
    async fn test_process_orderbook_with_empty_levels() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let orderbook = OrderBookData {
            coin: "AVAX".to_string(),
            bids: vec![],
            asks: vec![],
            timestamp: current_timestamp_ms(),
        };

        let market_data = MarketDataEvent::OrderBook(orderbook);
        detector.process_market_data(market_data).await.unwrap();

        // Should still cache even if empty
        assert!(detector.order_books().contains_key("AVAX"));
        let cached = detector.order_books().get("AVAX").unwrap();
        assert_eq!(cached.best_bid(), None);
        assert_eq!(cached.best_ask(), None);
    }

    #[tokio::test]
    async fn test_process_trades_for_multiple_coins() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let coins = vec![
            ("BTC", 45000.0),
            ("ETH", 3000.0),
            ("SOL", 100.0),
            ("AVAX", 35.0),
        ];

        for (coin, price) in coins.iter() {
            let trade = TradeData {
                coin: coin.to_string(),
                side: TradeSide::Buy,
                price: *price,
                size: 1.0,
                timestamp: current_timestamp_ms(),
                trade_id: format!("trade_{}", coin),
            };
            detector
                .process_market_data(MarketDataEvent::Trade(trade))
                .await
                .unwrap();
        }

        // Verify all prices were stored
        assert_eq!(detector.market_prices().len(), 4);
        assert_eq!(detector.market_prices().get("BTC"), Some(&45000.0));
        assert_eq!(detector.market_prices().get("ETH"), Some(&3000.0));
        assert_eq!(detector.market_prices().get("SOL"), Some(&100.0));
        assert_eq!(detector.market_prices().get("AVAX"), Some(&35.0));
    }

    // ========================================================================
    // State Update Event Processing Tests
    // ========================================================================

    #[tokio::test]
    async fn test_process_block_number_update() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let update = StateUpdateEvent::BlockNumber(12345);
        detector.process_state_update(update).await.unwrap();

        assert_eq!(detector.current_state().block_number, 12345);
    }

    #[tokio::test]
    async fn test_process_token_price_update() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let token_address = Address::from_low_u64_be(1);
        let price = U256::from(45000);

        let update = StateUpdateEvent::TokenPrice {
            token: token_address,
            price,
        };
        detector.process_state_update(update).await.unwrap();

        assert_eq!(
            detector.current_state().token_prices.get(&token_address),
            Some(&price)
        );
    }

    #[tokio::test]
    async fn test_process_contract_state_update() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let contract_address = Address::from_low_u64_be(2);
        let state_data = ethers::types::Bytes::from(vec![1, 2, 3, 4]);

        let update = StateUpdateEvent::ContractState {
            address: contract_address,
            data: state_data.clone(),
        };
        detector.process_state_update(update).await.unwrap();

        assert_eq!(
            detector.current_state().contract_states.get(&contract_address),
            Some(&state_data)
        );
    }

    #[tokio::test]
    async fn test_process_balance_update() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let address = Address::from_low_u64_be(3);
        let balance = U256::from(1000000);

        let update = StateUpdateEvent::Balance { address, balance };
        
        // Balance updates are logged but don't modify state directly
        detector.process_state_update(update).await.unwrap();
        
        // No assertion needed - just verify it doesn't panic
    }

    #[tokio::test]
    async fn test_process_state_snapshot() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let mut state = BlockchainState::new(5000, 1696348800);
        let token_address = Address::from_low_u64_be(1);
        state.token_prices.insert(token_address, U256::from(50000));

        let update = StateUpdateEvent::StateSnapshot(state.clone());
        detector.process_state_update(update).await.unwrap();

        assert_eq!(detector.current_state().block_number, 5000);
        assert_eq!(detector.current_state().timestamp, 1696348800);
        assert_eq!(
            detector.current_state().token_prices.get(&token_address),
            Some(&U256::from(50000))
        );
    }

    #[tokio::test]
    async fn test_process_multiple_state_updates() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        // Update block number
        detector
            .process_state_update(StateUpdateEvent::BlockNumber(1000))
            .await
            .unwrap();

        // Update token prices
        let token1 = Address::from_low_u64_be(1);
        let token2 = Address::from_low_u64_be(2);
        detector
            .process_state_update(StateUpdateEvent::TokenPrice {
                token: token1,
                price: U256::from(45000),
            })
            .await
            .unwrap();
        detector
            .process_state_update(StateUpdateEvent::TokenPrice {
                token: token2,
                price: U256::from(3000),
            })
            .await
            .unwrap();

        // Verify all updates were applied
        assert_eq!(detector.current_state().block_number, 1000);
        assert_eq!(detector.current_state().token_prices.len(), 2);
    }

    #[tokio::test]
    async fn test_process_transaction_confirmed_event() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let tx_hash = ethers::types::TxHash::zero();
        let update = StateUpdateEvent::TransactionConfirmed {
            tx_hash,
            block_number: 1000,
            gas_used: U256::from(21000),
            status: true,
        };

        // Should not panic - these events are handled elsewhere
        detector.process_state_update(update).await.unwrap();
    }

    #[tokio::test]
    async fn test_process_transaction_failed_event() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let tx_hash = ethers::types::TxHash::zero();
        let update = StateUpdateEvent::TransactionFailed {
            tx_hash,
            reason: "Gas too low".to_string(),
        };

        // Should not panic - these events are handled elsewhere
        detector.process_state_update(update).await.unwrap();
    }

    // ========================================================================
    // Opportunity Detection Logic Tests
    // ========================================================================

    #[tokio::test]
    async fn test_arbitrage_detection_with_sufficient_spread() {
        let config = OpportunityDetectorConfig {
            min_arbitrage_spread_bps: 30,
            max_market_data_age_ms: 10000,
            verify_via_rpc: false, // Disable RPC verification for this test
        };

        let (_market_tx, _state_tx, mut opp_rx, mut detector) =
            create_test_detector_with_config(config);

        // Set baseline price
        detector.market_prices.insert("BTC".to_string(), 45000.0);

        // Send trade with significant price difference (0.44% = 44 bps > 30 bps threshold)
        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 45200.0,
            size: 1.0,
            timestamp: current_timestamp_ms(),
            trade_id: "trade_arb".to_string(),
        };

        detector
            .process_market_data(MarketDataEvent::Trade(trade))
            .await
            .unwrap();

        // Should detect opportunity
        let opportunity = opp_rx.try_recv();
        assert!(
            opportunity.is_ok(),
            "Expected arbitrage opportunity to be detected"
        );

        let opp = opportunity.unwrap();
        assert!(!opp.verified); // Not verified since we disabled RPC verification
        match opp.opportunity_type {
            OpportunityType::Arbitrage { spread_bps, .. } => {
                assert!(spread_bps >= 30, "Spread should be >= 30 bps");
            }
            _ => panic!("Expected Arbitrage opportunity type"),
        }
    }

    #[tokio::test]
    async fn test_arbitrage_detection_with_insufficient_spread() {
        let config = OpportunityDetectorConfig {
            min_arbitrage_spread_bps: 50,
            max_market_data_age_ms: 10000,
            verify_via_rpc: false,
        };

        let (_market_tx, _state_tx, mut opp_rx, mut detector) =
            create_test_detector_with_config(config);

        // Set baseline price
        detector.market_prices.insert("ETH".to_string(), 3000.0);

        // Send trade with small price difference (0.1% = 10 bps < 50 bps threshold)
        let trade = TradeData {
            coin: "ETH".to_string(),
            side: TradeSide::Sell,
            price: 3003.0,
            size: 2.0,
            timestamp: current_timestamp_ms(),
            trade_id: "trade_small".to_string(),
        };

        detector
            .process_market_data(MarketDataEvent::Trade(trade))
            .await
            .unwrap();

        // Should NOT detect opportunity
        let opportunity = opp_rx.try_recv();
        assert!(
            opportunity.is_err(),
            "Should not detect opportunity with insufficient spread"
        );
    }

    #[tokio::test]
    async fn test_arbitrage_detection_with_orderbook() {
        let config = OpportunityDetectorConfig {
            min_arbitrage_spread_bps: 30,
            max_market_data_age_ms: 10000,
            verify_via_rpc: false,
        };

        let (_market_tx, _state_tx, mut opp_rx, mut detector) =
            create_test_detector_with_config(config);

        // Set baseline price
        detector.market_prices.insert("SOL".to_string(), 100.0);

        // Send orderbook with mid-price significantly different from baseline
        let orderbook = OrderBookData {
            coin: "SOL".to_string(),
            bids: vec![OrderBookLevel::new(101.0, 50.0)],
            asks: vec![OrderBookLevel::new(102.0, 40.0)],
            timestamp: current_timestamp_ms(),
        };

        detector
            .process_market_data(MarketDataEvent::OrderBook(orderbook))
            .await
            .unwrap();

        // Mid-price is 101.5, baseline is 100.0 -> 1.5% spread = 150 bps
        let opportunity = opp_rx.try_recv();
        assert!(
            opportunity.is_ok(),
            "Expected arbitrage opportunity from orderbook"
        );
    }

    #[tokio::test]
    async fn test_no_arbitrage_without_baseline_price() {
        let config = OpportunityDetectorConfig {
            min_arbitrage_spread_bps: 30,
            max_market_data_age_ms: 10000,
            verify_via_rpc: false,
        };

        let (_market_tx, _state_tx, mut opp_rx, mut detector) =
            create_test_detector_with_config(config);

        // Don't set baseline price - first trade for this coin
        let trade = TradeData {
            coin: "AVAX".to_string(),
            side: TradeSide::Buy,
            price: 35.0,
            size: 10.0,
            timestamp: current_timestamp_ms(),
            trade_id: "trade_first".to_string(),
        };

        detector
            .process_market_data(MarketDataEvent::Trade(trade))
            .await
            .unwrap();

        // Should NOT detect opportunity (no baseline to compare)
        let opportunity = opp_rx.try_recv();
        assert!(
            opportunity.is_err(),
            "Should not detect opportunity without baseline price"
        );
    }

    #[tokio::test]
    async fn test_stale_market_data_ignored() {
        let config = OpportunityDetectorConfig {
            min_arbitrage_spread_bps: 30,
            max_market_data_age_ms: 1000, // Only 1 second max age
            verify_via_rpc: false,
        };

        let (_market_tx, _state_tx, mut opp_rx, mut detector) =
            create_test_detector_with_config(config);

        // Set baseline price
        detector.market_prices.insert("BTC".to_string(), 45000.0);

        // Send trade with old timestamp (6 seconds ago)
        let old_timestamp = current_timestamp_ms() - 6000;
        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 46000.0, // Large price difference
            size: 1.0,
            timestamp: old_timestamp,
            trade_id: "trade_stale".to_string(),
        };

        detector
            .process_market_data(MarketDataEvent::Trade(trade))
            .await
            .unwrap();

        // Should NOT detect opportunity (data too old)
        let opportunity = opp_rx.try_recv();
        assert!(
            opportunity.is_err(),
            "Should not detect opportunity with stale data"
        );
    }

    // ========================================================================
    // RPC Verification Tests
    // ========================================================================

    #[tokio::test]
    async fn test_opportunity_verification_disabled() {
        let config = OpportunityDetectorConfig {
            min_arbitrage_spread_bps: 30,
            max_market_data_age_ms: 10000,
            verify_via_rpc: false, // Verification disabled
        };

        let (_market_tx, _state_tx, mut opp_rx, mut detector) =
            create_test_detector_with_config(config);

        detector.market_prices.insert("BTC".to_string(), 45000.0);

        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 45200.0,
            size: 1.0,
            timestamp: current_timestamp_ms(),
            trade_id: "trade_no_verify".to_string(),
        };

        detector
            .process_market_data(MarketDataEvent::Trade(trade))
            .await
            .unwrap();

        // Should emit opportunity without verification
        let opportunity = opp_rx.try_recv().unwrap();
        assert!(!opportunity.verified);
    }

    // Note: Testing RPC verification with actual RPC calls would require
    // mocking the RPC service, which is complex. The existing tests in
    // opportunity_detector.rs cover the basic verification logic.

    // ========================================================================
    // Error Handling Tests
    // ========================================================================

    #[tokio::test]
    async fn test_process_market_data_with_zero_price() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 0.0, // Zero price
            size: 1.0,
            timestamp: current_timestamp_ms(),
            trade_id: "trade_zero".to_string(),
        };

        // Should not panic
        let result = detector
            .process_market_data(MarketDataEvent::Trade(trade))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_market_data_with_negative_size() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        let trade = TradeData {
            coin: "ETH".to_string(),
            side: TradeSide::Sell,
            price: 3000.0,
            size: -1.0, // Negative size (shouldn't happen but test resilience)
            timestamp: current_timestamp_ms(),
            trade_id: "trade_neg".to_string(),
        };

        // Should not panic
        let result = detector
            .process_market_data(MarketDataEvent::Trade(trade))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_detector_handles_closed_opportunity_channel() {
        let (market_tx, state_tx, opp_rx, mut detector) = create_test_detector();

        // Drop the opportunity receiver to close the channel
        drop(opp_rx);

        // Set up for opportunity detection
        detector.market_prices.insert("BTC".to_string(), 45000.0);

        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 45500.0,
            size: 1.0,
            timestamp: current_timestamp_ms(),
            trade_id: "trade_closed".to_string(),
        };

        // Should not panic even though channel is closed
        let result = detector
            .process_market_data(MarketDataEvent::Trade(trade))
            .await;
        assert!(result.is_ok());

        drop(market_tx);
        drop(state_tx);
    }

    #[tokio::test]
    async fn test_concurrent_market_data_and_state_updates() {
        let (market_tx, state_tx, mut opp_rx, detector) = create_test_detector();

        // Spawn the detector event loop
        let detector_handle = tokio::spawn(async move {
            detector.start().await
        });

        // Send market data
        for i in 0..10 {
            let trade = TradeData {
                coin: "BTC".to_string(),
                side: TradeSide::Buy,
                price: 45000.0 + (i as f64 * 10.0),
                size: 1.0,
                timestamp: current_timestamp_ms(),
                trade_id: format!("trade_{}", i),
            };
            market_tx.send(MarketDataEvent::Trade(trade)).unwrap();
        }

        // Send state updates
        for i in 0..10 {
            state_tx
                .send(StateUpdateEvent::BlockNumber(1000 + i))
                .unwrap();
        }

        // Give some time for processing
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Close channels to stop detector
        drop(market_tx);
        drop(state_tx);

        // Wait for detector to finish
        let result = detector_handle.await;
        assert!(result.is_ok());

        // Drain any opportunities that were detected
        while opp_rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn test_detector_state_accessors() {
        let (_market_tx, _state_tx, _opp_rx, mut detector) = create_test_detector();

        // Process some data
        detector
            .process_state_update(StateUpdateEvent::BlockNumber(5000))
            .await
            .unwrap();

        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 45000.0,
            size: 1.0,
            timestamp: current_timestamp_ms(),
            trade_id: "trade_accessor".to_string(),
        };
        detector
            .process_market_data(MarketDataEvent::Trade(trade))
            .await
            .unwrap();

        // Test accessors
        assert_eq!(detector.current_state().block_number, 5000);
        assert_eq!(detector.market_prices().get("BTC"), Some(&45000.0));
        assert!(detector.order_books().is_empty());
    }

    #[tokio::test]
    async fn test_custom_config_values() {
        let config = OpportunityDetectorConfig {
            min_arbitrage_spread_bps: 100,
            max_market_data_age_ms: 5000,
            verify_via_rpc: true,
        };

        let (_market_tx, _state_tx, _opp_rx, detector) =
            create_test_detector_with_config(config.clone());

        assert_eq!(detector.config.min_arbitrage_spread_bps, 100);
        assert_eq!(detector.config.max_market_data_age_ms, 5000);
        assert!(detector.config.verify_via_rpc);
    }

    #[tokio::test]
    async fn test_default_config_values() {
        let (_market_tx, _state_tx, _opp_rx, detector) = create_test_detector();

        assert_eq!(detector.config.min_arbitrage_spread_bps, 30);
        assert_eq!(detector.config.max_market_data_age_ms, 5000);
        assert!(detector.config.verify_via_rpc);
    }
}
