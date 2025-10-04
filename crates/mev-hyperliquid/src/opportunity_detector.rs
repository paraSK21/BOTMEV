//! Opportunity detector for combining market data and blockchain state
//!
//! This module provides the `OpportunityDetector` which combines real-time market data
//! from WebSocket with blockchain state from RPC polling to identify and verify MEV
//! opportunities such as arbitrage, liquidations, and front-running.
//!
//! ## Architecture
//!
//! The OpportunityDetector receives two streams of data:
//! 1. **Market Data**: Real-time trades and order books from WebSocket
//! 2. **State Updates**: Blockchain state from RPC polling (block numbers, prices, etc.)
//!
//! It combines these data sources to:
//! - Detect potential opportunities based on market data
//! - Verify opportunities against on-chain state via RPC
//! - Emit confirmed opportunities to the strategy engine

use anyhow::{Context, Result};
use ethers::types::{Address, U256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::rpc_service::HyperLiquidRpcService;
use crate::types::{
    BlockchainState, MarketDataEvent, Opportunity, OpportunityType, OrderBookData, StateUpdateEvent,
    TradeData,
};

/// Minimum spread in basis points to consider an arbitrage opportunity
const MIN_ARBITRAGE_SPREAD_BPS: u64 = 30; // 0.3%

/// Maximum age of market data in milliseconds before it's considered stale
const MAX_MARKET_DATA_AGE_MS: u64 = 5000; // 5 seconds

/// Opportunity detector configuration
#[derive(Debug, Clone)]
pub struct OpportunityDetectorConfig {
    /// Minimum spread in basis points for arbitrage opportunities
    pub min_arbitrage_spread_bps: u64,
    
    /// Maximum age of market data in milliseconds
    pub max_market_data_age_ms: u64,
    
    /// Whether to verify opportunities via RPC before emitting
    pub verify_via_rpc: bool,
}

impl Default for OpportunityDetectorConfig {
    fn default() -> Self {
        Self {
            min_arbitrage_spread_bps: MIN_ARBITRAGE_SPREAD_BPS,
            max_market_data_age_ms: MAX_MARKET_DATA_AGE_MS,
            verify_via_rpc: true,
        }
    }
}

/// Opportunity detector for combining market data and blockchain state
pub struct OpportunityDetector {
    /// Receiver for market data events from WebSocket
    market_data_rx: mpsc::UnboundedReceiver<MarketDataEvent>,
    
    /// Receiver for state update events from RPC
    state_update_rx: mpsc::UnboundedReceiver<StateUpdateEvent>,
    
    /// RPC service for opportunity verification
    rpc_service: Arc<HyperLiquidRpcService>,
    
    /// Sender for confirmed opportunities
    opportunity_tx: mpsc::UnboundedSender<Opportunity>,
    
    /// Configuration
    config: OpportunityDetectorConfig,
    
    /// Current blockchain state (cached)
    current_state: BlockchainState,
    
    /// Latest market prices by coin symbol
    market_prices: HashMap<String, f64>,
    
    /// Latest order books by coin symbol
    order_books: HashMap<String, OrderBookData>,
}

impl OpportunityDetector {
    /// Create a new OpportunityDetector
    ///
    /// # Arguments
    /// * `market_data_rx` - Receiver for market data events from WebSocket
    /// * `state_update_rx` - Receiver for state update events from RPC
    /// * `rpc_service` - RPC service for opportunity verification
    /// * `opportunity_tx` - Sender for confirmed opportunities
    ///
    /// # Returns
    /// A new OpportunityDetector instance
    pub fn new(
        market_data_rx: mpsc::UnboundedReceiver<MarketDataEvent>,
        state_update_rx: mpsc::UnboundedReceiver<StateUpdateEvent>,
        rpc_service: Arc<HyperLiquidRpcService>,
        opportunity_tx: mpsc::UnboundedSender<Opportunity>,
    ) -> Self {
        Self::with_config(
            market_data_rx,
            state_update_rx,
            rpc_service,
            opportunity_tx,
            OpportunityDetectorConfig::default(),
        )
    }
    
    /// Create a new OpportunityDetector with custom configuration
    pub fn with_config(
        market_data_rx: mpsc::UnboundedReceiver<MarketDataEvent>,
        state_update_rx: mpsc::UnboundedReceiver<StateUpdateEvent>,
        rpc_service: Arc<HyperLiquidRpcService>,
        opportunity_tx: mpsc::UnboundedSender<Opportunity>,
        config: OpportunityDetectorConfig,
    ) -> Self {
        info!("Creating OpportunityDetector with config: {:?}", config);
        
        Self {
            market_data_rx,
            state_update_rx,
            rpc_service,
            opportunity_tx,
            config,
            current_state: BlockchainState::new(0, 0),
            market_prices: HashMap::new(),
            order_books: HashMap::new(),
        }
    }
    
    /// Start the opportunity detection event loop
    ///
    /// This method runs indefinitely, processing market data and state updates
    /// to detect and verify opportunities. It will continue until both channels
    /// are closed.
    ///
    /// # Returns
    /// * `Ok(())` when both channels are closed
    /// * `Err` if there's a critical error in the event loop
    pub async fn start(mut self) -> Result<()> {
        info!("Starting OpportunityDetector event loop");
        
        loop {
            tokio::select! {
                // Process market data events
                Some(market_data) = self.market_data_rx.recv() => {
                    if let Err(e) = self.process_market_data(market_data).await {
                        error!("Error processing market data: {:#}", e);
                        // Continue processing despite errors
                    }
                }
                
                // Process state update events
                Some(state_update) = self.state_update_rx.recv() => {
                    if let Err(e) = self.process_state_update(state_update).await {
                        error!("Error processing state update: {:#}", e);
                        // Continue processing despite errors
                    }
                }
                
                // Both channels closed
                else => {
                    info!("Both market data and state update channels closed, stopping OpportunityDetector");
                    break;
                }
            }
        }
        
        Ok(())
    }
    
    /// Process market data event from WebSocket
    ///
    /// This method updates internal state with the latest market data and
    /// checks for potential opportunities.
    ///
    /// # Arguments
    /// * `data` - Market data event (trade or order book update)
    ///
    /// # Returns
    /// * `Ok(())` if processing succeeds
    /// * `Err` if there's an error during processing
    async fn process_market_data(&mut self, data: MarketDataEvent) -> Result<()> {
        debug!("Processing market data: {:?}", data);
        
        match &data {
            MarketDataEvent::Trade(trade) => {
                self.process_trade(trade, &data).await?;
            }
            MarketDataEvent::OrderBook(orderbook) => {
                self.process_orderbook(orderbook, &data).await?;
            }
        }
        
        Ok(())
    }
    
    /// Process trade data
    async fn process_trade(&mut self, trade: &TradeData, market_data: &MarketDataEvent) -> Result<()> {
        debug!(
            "Processing trade for {}: price={}",
            trade.coin, trade.price
        );
        
        // Check for arbitrage opportunities BEFORE updating the price
        // This allows us to compare the new price with the previous price
        self.check_arbitrage_opportunity(market_data).await?;
        
        // Update latest market price AFTER checking for opportunities
        self.market_prices.insert(trade.coin.clone(), trade.price);
        
        Ok(())
    }
    
    /// Process order book data
    async fn process_orderbook(
        &mut self,
        orderbook: &OrderBookData,
        market_data: &MarketDataEvent,
    ) -> Result<()> {
        // Update cached order book
        self.order_books.insert(orderbook.coin.clone(), orderbook.clone());
        
        debug!(
            "Updated order book for {}: best_bid={:?}, best_ask={:?}",
            orderbook.coin,
            orderbook.best_bid(),
            orderbook.best_ask()
        );
        
        // Check for arbitrage opportunities using order book data
        self.check_arbitrage_opportunity(market_data).await?;
        
        Ok(())
    }
    
    /// Process state update event from RPC
    ///
    /// This method updates the cached blockchain state with the latest data
    /// from RPC polling.
    ///
    /// # Arguments
    /// * `update` - State update event
    ///
    /// # Returns
    /// * `Ok(())` if processing succeeds
    /// * `Err` if there's an error during processing
    async fn process_state_update(&mut self, update: StateUpdateEvent) -> Result<()> {
        debug!("Processing state update: {:?}", update);
        
        match update {
            StateUpdateEvent::BlockNumber(block_number) => {
                self.current_state.block_number = block_number;
                debug!("Updated block number: {}", block_number);
            }
            StateUpdateEvent::TokenPrice { token, price } => {
                self.current_state.token_prices.insert(token, price);
                debug!("Updated token price: {:?} = {}", token, price);
            }
            StateUpdateEvent::ContractState { address, data } => {
                self.current_state.contract_states.insert(address, data);
                debug!("Updated contract state: {:?}", address);
            }
            StateUpdateEvent::Balance { address, balance } => {
                debug!("Balance update: {:?} = {}", address, balance);
                // Could be used for liquidation detection in the future
            }
            StateUpdateEvent::StateSnapshot(state) => {
                self.current_state = state;
                debug!(
                    "Updated full state snapshot: block {}",
                    self.current_state.block_number
                );
            }
            StateUpdateEvent::TransactionConfirmed { .. } => {
                // Transaction confirmations are handled elsewhere
                debug!("Transaction confirmed (handled by executor)");
            }
            StateUpdateEvent::TransactionFailed { .. } => {
                // Transaction failures are handled elsewhere
                debug!("Transaction failed (handled by executor)");
            }
        }
        
        Ok(())
    }
    
    /// Check for arbitrage opportunities
    ///
    /// This method compares market data from HyperLiquid with on-chain prices
    /// to identify potential arbitrage opportunities.
    async fn check_arbitrage_opportunity(&mut self, market_data: &MarketDataEvent) -> Result<()> {
        let coin = market_data.coin();
        
        // Check if market data is stale
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        
        let data_age = now.saturating_sub(market_data.timestamp());
        if data_age > self.config.max_market_data_age_ms {
            debug!(
                "Market data for {} is stale ({}ms old), skipping",
                coin, data_age
            );
            return Ok(());
        }
        
        // Get HyperLiquid price from market data
        let hl_price = match market_data {
            MarketDataEvent::Trade(trade) => Some(trade.price),
            MarketDataEvent::OrderBook(orderbook) => {
                // Use mid price from order book
                match (orderbook.best_bid(), orderbook.best_ask()) {
                    (Some(bid), Some(ask)) => Some((bid + ask) / 2.0),
                    _ => None,
                }
            }
        };
        
        let hl_price = match hl_price {
            Some(price) => price,
            None => {
                debug!("Could not determine HyperLiquid price for {}", coin);
                return Ok(());
            }
        };
        
        // For now, we'll use a simple heuristic: compare with the last known price
        // In a real implementation, you would compare with prices from other DEXes/CEXes
        
        // Check if we have a previous price to compare
        if let Some(&prev_price) = self.market_prices.get(coin) {
            if prev_price == 0.0 || hl_price == 0.0 {
                return Ok(());
            }
            
            // Calculate spread in basis points
            let spread_bps = ((hl_price - prev_price).abs() / prev_price * 10000.0) as u64;
            
            if spread_bps >= self.config.min_arbitrage_spread_bps {
                debug!(
                    "Potential arbitrage opportunity detected for {}: spread={}bps",
                    coin, spread_bps
                );
                
                // Determine buy/sell venues based on price difference
                let (buy_venue, sell_venue) = if hl_price < prev_price {
                    ("HyperLiquid".to_string(), "External".to_string())
                } else {
                    ("External".to_string(), "HyperLiquid".to_string())
                };
                
                // Calculate expected profit (simplified)
                let profit_estimate = U256::from((spread_bps as f64 * 1000.0) as u64);
                
                // Create opportunity
                let mut opportunity = Opportunity::new(
                    OpportunityType::Arbitrage {
                        buy_venue,
                        sell_venue,
                        token: Address::zero(), // Would need token mapping
                        spread_bps,
                    },
                    profit_estimate,
                    market_data.clone(),
                    self.current_state.clone(),
                );
                
                // Set expiration (opportunities expire quickly)
                let expires_at = now + 10_000; // 10 seconds
                opportunity.set_expiration(expires_at);
                
                // Verify via RPC if configured
                if self.config.verify_via_rpc {
                    match self.verify_opportunity_via_rpc(&opportunity).await {
                        Ok(true) => {
                            opportunity.mark_verified();
                            info!(
                                "Verified arbitrage opportunity for {}: spread={}bps, profit={}",
                                coin, spread_bps, profit_estimate
                            );
                            
                            // Emit confirmed opportunity
                            if let Err(e) = self.opportunity_tx.send(opportunity) {
                                error!("Failed to send opportunity: {}", e);
                            }
                        }
                        Ok(false) => {
                            debug!(
                                "Opportunity verification failed for {}: conditions no longer valid",
                                coin
                            );
                        }
                        Err(e) => {
                            warn!("Error verifying opportunity for {}: {:#}", coin, e);
                            // Could still emit unverified opportunity if desired
                        }
                    }
                } else {
                    // Emit without verification
                    info!(
                        "Detected arbitrage opportunity for {} (unverified): spread={}bps",
                        coin, spread_bps
                    );
                    
                    if let Err(e) = self.opportunity_tx.send(opportunity) {
                        error!("Failed to send opportunity: {}", e);
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Verify opportunity via RPC
    ///
    /// This method performs on-chain verification of the opportunity to ensure
    /// the conditions are still valid before execution.
    ///
    /// # Arguments
    /// * `opportunity` - The opportunity to verify
    ///
    /// # Returns
    /// * `Ok(true)` if opportunity is verified and still valid
    /// * `Ok(false)` if opportunity is no longer valid
    /// * `Err` if verification fails due to RPC errors
    async fn verify_opportunity_via_rpc(&self, opportunity: &Opportunity) -> Result<bool> {
        debug!("Verifying opportunity via RPC: {:?}", opportunity.opportunity_type);
        
        // Check if opportunity has expired
        if opportunity.is_expired() {
            debug!("Opportunity has expired, skipping verification");
            return Ok(false);
        }
        
        match &opportunity.opportunity_type {
            OpportunityType::Arbitrage {
                token,
                spread_bps,
                ..
            } => {
                // Verify the arbitrage opportunity by checking on-chain state
                
                // 1. Check current block number to ensure we're not too far behind
                let current_state = self.rpc_service
                    .poll_blockchain_state()
                    .await
                    .context("Failed to poll blockchain state for verification")?;
                
                let block_diff = current_state.block_number.saturating_sub(
                    opportunity.blockchain_state.block_number
                );
                
                if block_diff > 5 {
                    debug!(
                        "Opportunity is stale: detected at block {}, current block {}",
                        opportunity.blockchain_state.block_number,
                        current_state.block_number
                    );
                    return Ok(false);
                }
                
                // 2. Verify token price on-chain (if available)
                if let Some(&on_chain_price) = current_state.token_prices.get(token) {
                    debug!("On-chain price for token {:?}: {}", token, on_chain_price);
                    
                    // Check if the spread is still profitable
                    // This is a simplified check - real implementation would be more sophisticated
                    if on_chain_price == U256::zero() {
                        debug!("On-chain price is zero, opportunity may not be valid");
                        return Ok(false);
                    }
                }
                
                // 3. Additional checks could include:
                // - Gas price estimation
                // - Liquidity checks
                // - Slippage tolerance
                // - Contract state validation
                
                debug!(
                    "Opportunity verified: spread={}bps, block_diff={}",
                    spread_bps, block_diff
                );
                
                Ok(true)
            }
            OpportunityType::Liquidation { .. } => {
                // Liquidation verification would check position health, collateral ratios, etc.
                debug!("Liquidation opportunity verification not yet implemented");
                Ok(false)
            }
            OpportunityType::FrontRun { .. } => {
                // Front-running verification would check if target tx is still pending
                debug!("Front-run opportunity verification not yet implemented");
                Ok(false)
            }
        }
    }
    
    /// Get current blockchain state
    pub fn current_state(&self) -> &BlockchainState {
        &self.current_state
    }
    
    /// Get current market prices
    pub fn market_prices(&self) -> &HashMap<String, f64> {
        &self.market_prices
    }
    
    /// Get current order books
    pub fn order_books(&self) -> &HashMap<String, OrderBookData> {
        &self.order_books
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc_service::RpcConfig;
    use crate::types::TradeSide;
    
    fn create_test_rpc_service() -> Arc<HyperLiquidRpcService> {
        let config = RpcConfig::new("http://localhost:8545".to_string(), 1000);
        Arc::new(HyperLiquidRpcService::new(config, None))
    }
    
    #[tokio::test]
    async fn test_opportunity_detector_creation() {
        let (market_data_tx, market_data_rx) = mpsc::unbounded_channel();
        let (state_update_tx, state_update_rx) = mpsc::unbounded_channel();
        let (opportunity_tx, _opportunity_rx) = mpsc::unbounded_channel();
        
        let rpc_service = create_test_rpc_service();
        
        let detector = OpportunityDetector::new(
            market_data_rx,
            state_update_rx,
            rpc_service,
            opportunity_tx,
        );
        
        assert_eq!(detector.current_state.block_number, 0);
        assert!(detector.market_prices.is_empty());
        assert!(detector.order_books.is_empty());
        
        // Clean up channels
        drop(market_data_tx);
        drop(state_update_tx);
    }
    
    #[tokio::test]
    async fn test_process_trade_data() {
        let (market_data_tx, market_data_rx) = mpsc::unbounded_channel();
        let (_state_update_tx, state_update_rx) = mpsc::unbounded_channel();
        let (opportunity_tx, _opportunity_rx) = mpsc::unbounded_channel();
        
        let rpc_service = create_test_rpc_service();
        
        let mut detector = OpportunityDetector::new(
            market_data_rx,
            state_update_rx,
            rpc_service,
            opportunity_tx,
        );
        
        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 45000.0,
            size: 1.0,
            timestamp: 1696348800000,
            trade_id: "123".to_string(),
        };
        
        let market_data = MarketDataEvent::Trade(trade.clone());
        
        detector.process_market_data(market_data).await.unwrap();
        
        assert_eq!(detector.market_prices.get("BTC"), Some(&45000.0));
        
        drop(market_data_tx);
    }
    
    #[tokio::test]
    async fn test_process_state_update() {
        let (_market_data_tx, market_data_rx) = mpsc::unbounded_channel();
        let (state_update_tx, state_update_rx) = mpsc::unbounded_channel();
        let (opportunity_tx, _opportunity_rx) = mpsc::unbounded_channel();
        
        let rpc_service = create_test_rpc_service();
        
        let mut detector = OpportunityDetector::new(
            market_data_rx,
            state_update_rx,
            rpc_service,
            opportunity_tx,
        );
        
        let update = StateUpdateEvent::BlockNumber(1000);
        detector.process_state_update(update).await.unwrap();
        
        assert_eq!(detector.current_state.block_number, 1000);
        
        drop(state_update_tx);
    }
    
    #[tokio::test]
    async fn test_custom_config() {
        let (_market_data_tx, market_data_rx) = mpsc::unbounded_channel();
        let (_state_update_tx, state_update_rx) = mpsc::unbounded_channel();
        let (opportunity_tx, _opportunity_rx) = mpsc::unbounded_channel();
        
        let rpc_service = create_test_rpc_service();
        
        let config = OpportunityDetectorConfig {
            min_arbitrage_spread_bps: 50,
            max_market_data_age_ms: 10000,
            verify_via_rpc: false,
        };
        
        let detector = OpportunityDetector::with_config(
            market_data_rx,
            state_update_rx,
            rpc_service,
            opportunity_tx,
            config.clone(),
        );
        
        assert_eq!(detector.config.min_arbitrage_spread_bps, 50);
        assert_eq!(detector.config.max_market_data_age_ms, 10000);
        assert!(!detector.config.verify_via_rpc);
    }
    
    #[tokio::test]
    async fn test_arbitrage_detection_without_verification() {
        let (market_data_tx, market_data_rx) = mpsc::unbounded_channel();
        let (_state_update_tx, state_update_rx) = mpsc::unbounded_channel();
        let (opportunity_tx, mut opportunity_rx) = mpsc::unbounded_channel();
        
        let rpc_service = create_test_rpc_service();
        
        // Configure without RPC verification for testing
        let config = OpportunityDetectorConfig {
            min_arbitrage_spread_bps: 30,
            max_market_data_age_ms: 10000,
            verify_via_rpc: false,
        };
        
        let mut detector = OpportunityDetector::with_config(
            market_data_rx,
            state_update_rx,
            rpc_service,
            opportunity_tx,
            config,
        );
        
        // Manually set a baseline price in the detector's state
        detector.market_prices.insert("BTC".to_string(), 45000.0);
        
        // Send trade with significant price difference (should trigger arbitrage)
        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 45200.0, // ~0.44% increase = 44 bps
            size: 1.0,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            trade_id: "124".to_string(),
        };
        
        let market_data = MarketDataEvent::Trade(trade);
        detector.process_market_data(market_data).await.unwrap();
        
        // Check if opportunity was detected
        let opportunity = opportunity_rx.try_recv();
        assert!(opportunity.is_ok(), "Expected arbitrage opportunity to be detected");
        
        let opp = opportunity.unwrap();
        assert!(!opp.verified); // Should not be verified since we disabled RPC verification
        assert!(matches!(opp.opportunity_type, OpportunityType::Arbitrage { .. }));
        
        drop(market_data_tx);
    }
    
    #[tokio::test]
    async fn test_orderbook_processing() {
        let (market_data_tx, market_data_rx) = mpsc::unbounded_channel();
        let (_state_update_tx, state_update_rx) = mpsc::unbounded_channel();
        let (opportunity_tx, _opportunity_rx) = mpsc::unbounded_channel();
        
        let rpc_service = create_test_rpc_service();
        
        let mut detector = OpportunityDetector::new(
            market_data_rx,
            state_update_rx,
            rpc_service,
            opportunity_tx,
        );
        
        let orderbook = crate::types::OrderBookData {
            coin: "ETH".to_string(),
            bids: vec![
                crate::types::OrderBookLevel::new(3000.0, 10.0),
                crate::types::OrderBookLevel::new(2999.0, 5.0),
            ],
            asks: vec![
                crate::types::OrderBookLevel::new(3001.0, 8.0),
                crate::types::OrderBookLevel::new(3002.0, 12.0),
            ],
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        };
        
        let market_data = MarketDataEvent::OrderBook(orderbook.clone());
        detector.process_market_data(market_data).await.unwrap();
        
        // Verify orderbook was cached
        assert!(detector.order_books.contains_key("ETH"));
        let cached_orderbook = detector.order_books.get("ETH").unwrap();
        assert_eq!(cached_orderbook.best_bid(), Some(3000.0));
        assert_eq!(cached_orderbook.best_ask(), Some(3001.0));
        
        drop(market_data_tx);
    }
    
    #[tokio::test]
    async fn test_state_snapshot_update() {
        let (_market_data_tx, market_data_rx) = mpsc::unbounded_channel();
        let (state_update_tx, state_update_rx) = mpsc::unbounded_channel();
        let (opportunity_tx, _opportunity_rx) = mpsc::unbounded_channel();
        
        let rpc_service = create_test_rpc_service();
        
        let mut detector = OpportunityDetector::new(
            market_data_rx,
            state_update_rx,
            rpc_service,
            opportunity_tx,
        );
        
        let mut state = BlockchainState::new(5000, 1696348800);
        state.token_prices.insert(Address::zero(), U256::from(45000));
        
        let update = StateUpdateEvent::StateSnapshot(state.clone());
        detector.process_state_update(update).await.unwrap();
        
        assert_eq!(detector.current_state.block_number, 5000);
        assert_eq!(detector.current_state.timestamp, 1696348800);
        assert_eq!(
            detector.current_state.token_prices.get(&Address::zero()),
            Some(&U256::from(45000))
        );
        
        drop(state_update_tx);
    }
}
