use ethers::types::{Address, Bytes, U256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Blockchain state snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockchainState {
    /// Current block number
    pub block_number: u64,
    
    /// Block timestamp
    pub timestamp: u64,
    
    /// Token prices (address -> price)
    pub token_prices: HashMap<Address, U256>,
    
    /// Contract states (address -> data)
    pub contract_states: HashMap<Address, Bytes>,
}

impl BlockchainState {
    pub fn new(block_number: u64, timestamp: u64) -> Self {
        Self {
            block_number,
            timestamp,
            token_prices: HashMap::new(),
            contract_states: HashMap::new(),
        }
    }
}

/// State update events from blockchain polling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StateUpdateEvent {
    /// New block number detected
    BlockNumber(u64),
    
    /// Token price update
    TokenPrice { token: Address, price: U256 },
    
    /// Contract state update
    ContractState { address: Address, data: Bytes },
    
    /// Balance update
    Balance { address: Address, balance: U256 },
    
    /// Full state snapshot
    StateSnapshot(BlockchainState),
    
    /// Transaction confirmation success
    TransactionConfirmed {
        tx_hash: ethers::types::TxHash,
        block_number: u64,
        gas_used: U256,
        status: bool,
    },
    
    /// Transaction confirmation failed or timed out
    TransactionFailed {
        tx_hash: ethers::types::TxHash,
        reason: String,
    },
}

/// Market data events from WebSocket (wrapper for existing types)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MarketDataEvent {
    /// Trade data received
    Trade(TradeData),
    
    /// Order book update received
    OrderBook(OrderBookData),
}

impl MarketDataEvent {
    /// Get the coin symbol from the event
    pub fn coin(&self) -> &str {
        match self {
            MarketDataEvent::Trade(trade) => &trade.coin,
            MarketDataEvent::OrderBook(orderbook) => &orderbook.coin,
        }
    }
    
    /// Get the timestamp from the event
    pub fn timestamp(&self) -> u64 {
        match self {
            MarketDataEvent::Trade(trade) => trade.timestamp,
            MarketDataEvent::OrderBook(orderbook) => orderbook.timestamp,
        }
    }
}

/// Type of MEV opportunity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OpportunityType {
    /// Arbitrage opportunity between venues
    Arbitrage {
        /// Venue to buy from
        buy_venue: String,
        
        /// Venue to sell to
        sell_venue: String,
        
        /// Token address
        token: Address,
        
        /// Spread in basis points (1 bps = 0.01%)
        spread_bps: u64,
    },
    
    /// Liquidation opportunity
    Liquidation {
        /// Position address to liquidate
        position: Address,
        
        /// Collateral token
        collateral_token: Address,
        
        /// Debt token
        debt_token: Address,
        
        /// Expected profit
        profit: U256,
    },
    
    /// Front-running opportunity
    FrontRun {
        /// Target transaction hash
        target_tx: ethers::types::TxHash,
        
        /// Token being traded
        token: Address,
        
        /// Expected price impact
        price_impact_bps: u64,
    },
}

/// Detected MEV opportunity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opportunity {
    /// Type of opportunity
    pub opportunity_type: OpportunityType,
    
    /// Expected profit in wei
    pub expected_profit: U256,
    
    /// Market data that triggered the opportunity
    pub market_data: MarketDataEvent,
    
    /// Blockchain state at detection time
    pub blockchain_state: BlockchainState,
    
    /// Whether the opportunity has been verified via RPC
    pub verified: bool,
    
    /// Timestamp when opportunity was detected (milliseconds)
    pub detected_at: u64,
    
    /// Optional expiration timestamp (milliseconds)
    pub expires_at: Option<u64>,
}

impl Opportunity {
    /// Create a new opportunity
    pub fn new(
        opportunity_type: OpportunityType,
        expected_profit: U256,
        market_data: MarketDataEvent,
        blockchain_state: BlockchainState,
    ) -> Self {
        Self {
            opportunity_type,
            expected_profit,
            market_data,
            blockchain_state,
            verified: false,
            detected_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            expires_at: None,
        }
    }
    
    /// Mark the opportunity as verified
    pub fn mark_verified(&mut self) {
        self.verified = true;
    }
    
    /// Set expiration time
    pub fn set_expiration(&mut self, expires_at: u64) {
        self.expires_at = Some(expires_at);
    }
    
    /// Check if the opportunity has expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            now >= expires_at
        } else {
            false
        }
    }
    
    /// Get the coin/token symbol from the opportunity
    pub fn coin(&self) -> &str {
        self.market_data.coin()
    }
}

/// Configuration for WebSocket reconnection behavior
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// Minimum backoff time in seconds
    pub min_backoff_secs: u64,
    
    /// Maximum backoff time in seconds
    pub max_backoff_secs: u64,
    
    /// Maximum number of consecutive connection failures before entering degraded state
    pub max_consecutive_failures: u32,
}

impl ReconnectConfig {
    pub fn new(min_backoff_secs: u64, max_backoff_secs: u64, max_consecutive_failures: u32) -> Self {
        Self {
            min_backoff_secs,
            max_backoff_secs,
            max_consecutive_failures,
        }
    }
    
    /// Calculate backoff duration for a given attempt number using exponential backoff
    pub fn calculate_backoff(&self, attempt: u32) -> u64 {
        let backoff = self.min_backoff_secs * 2_u64.pow(attempt.saturating_sub(1));
        backoff.min(self.max_backoff_secs)
    }
}

/// Trading pair configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingPair {
    /// Coin symbol (e.g., "BTC", "ETH")
    pub coin: String,
    
    /// Subscribe to trade feed for this pair
    pub subscribe_trades: bool,
    
    /// Subscribe to order book updates for this pair
    pub subscribe_orderbook: bool,
}

impl TradingPair {
    pub fn new(coin: String, subscribe_trades: bool, subscribe_orderbook: bool) -> Self {
        Self {
            coin,
            subscribe_trades,
            subscribe_orderbook,
        }
    }
    
    /// Create a trading pair with only trade subscription
    pub fn trades_only(coin: String) -> Self {
        Self::new(coin, true, false)
    }
    
    /// Create a trading pair with both trade and order book subscriptions
    pub fn with_orderbook(coin: String) -> Self {
        Self::new(coin, true, true)
    }
}

/// Side of a trade (buy or sell)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeSide {
    /// Buy order (bid)
    Buy,
    
    /// Sell order (ask)
    Sell,
}

impl TradeSide {
    /// Parse from HyperLiquid's format ("B" for buy, "A" for ask/sell)
    pub fn from_hyperliquid(s: &str) -> Option<Self> {
        match s {
            "B" => Some(TradeSide::Buy),
            "A" => Some(TradeSide::Sell),
            _ => None,
        }
    }
}

/// Trade data from HyperLiquid
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeData {
    /// Coin symbol (e.g., "BTC")
    pub coin: String,
    
    /// Side of the trade (buy or sell)
    pub side: TradeSide,
    
    /// Price of the trade
    pub price: f64,
    
    /// Size/amount of the trade
    pub size: f64,
    
    /// Timestamp in milliseconds
    pub timestamp: u64,
    
    /// Unique trade ID
    pub trade_id: String,
}

/// Order book level (price and size)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookLevel {
    /// Price level
    pub price: f64,
    
    /// Total size at this price level
    pub size: f64,
}

impl OrderBookLevel {
    pub fn new(price: f64, size: f64) -> Self {
        Self { price, size }
    }
}

/// Order book data from HyperLiquid
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookData {
    /// Coin symbol
    pub coin: String,
    
    /// Bid levels (buy orders)
    pub bids: Vec<OrderBookLevel>,
    
    /// Ask levels (sell orders)
    pub asks: Vec<OrderBookLevel>,
    
    /// Timestamp in milliseconds
    pub timestamp: u64,
}

impl OrderBookData {
    /// Get the best bid price (highest buy price)
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.first().map(|level| level.price)
    }
    
    /// Get the best ask price (lowest sell price)
    pub fn best_ask(&self) -> Option<f64> {
        self.asks.first().map(|level| level.price)
    }
    
    /// Calculate the spread between best bid and ask
    pub fn spread(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(ask - bid),
            _ => None,
        }
    }
    
    /// Calculate the spread as a percentage of the mid price
    pub fn spread_percentage(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => {
                let mid = (bid + ask) / 2.0;
                if mid > 0.0 {
                    Some(((ask - bid) / mid) * 100.0)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Subscription response from HyperLiquid
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionResponse {
    /// Type of subscription (e.g., "trades", "l2Book")
    pub subscription_type: String,
    
    /// Coin symbol
    pub coin: String,
    
    /// Whether the subscription was successful
    pub success: bool,
    
    /// Optional error message if subscription failed
    pub error: Option<String>,
}

/// Error data from HyperLiquid
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorData {
    /// Error code
    pub code: Option<i32>,
    
    /// Error message
    pub message: String,
}

/// Message data variants
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessageData {
    /// Subscription confirmation response
    SubscriptionResponse(SubscriptionResponse),
    
    /// Trade data
    Trade(TradeData),
    
    /// Order book update
    OrderBook(OrderBookData),
    
    /// Error message
    Error(ErrorData),
}

/// Complete message from HyperLiquid WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperLiquidMessage {
    /// Channel name (e.g., "trades", "l2Book")
    pub channel: String,
    
    /// Message data
    pub data: MessageData,
}

impl HyperLiquidMessage {
    pub fn new(channel: String, data: MessageData) -> Self {
        Self { channel, data }
    }
    
    /// Check if this is a trade message
    pub fn is_trade(&self) -> bool {
        matches!(self.data, MessageData::Trade(_))
    }
    
    /// Check if this is an order book message
    pub fn is_orderbook(&self) -> bool {
        matches!(self.data, MessageData::OrderBook(_))
    }
    
    /// Check if this is an error message
    pub fn is_error(&self) -> bool {
        matches!(self.data, MessageData::Error(_))
    }
    
    /// Check if this is a subscription response
    pub fn is_subscription_response(&self) -> bool {
        matches!(self.data, MessageData::SubscriptionResponse(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconnect_config_backoff_calculation() {
        let config = ReconnectConfig::new(1, 60, 10);
        
        assert_eq!(config.calculate_backoff(1), 1);  // 1 * 2^0 = 1
        assert_eq!(config.calculate_backoff(2), 2);  // 1 * 2^1 = 2
        assert_eq!(config.calculate_backoff(3), 4);  // 1 * 2^2 = 4
        assert_eq!(config.calculate_backoff(4), 8);  // 1 * 2^3 = 8
        assert_eq!(config.calculate_backoff(5), 16); // 1 * 2^4 = 16
        assert_eq!(config.calculate_backoff(6), 32); // 1 * 2^5 = 32
        assert_eq!(config.calculate_backoff(7), 60); // 1 * 2^6 = 64, capped at 60
        assert_eq!(config.calculate_backoff(10), 60); // Capped at max
    }

    #[test]
    fn test_trade_side_from_hyperliquid() {
        assert_eq!(TradeSide::from_hyperliquid("B"), Some(TradeSide::Buy));
        assert_eq!(TradeSide::from_hyperliquid("A"), Some(TradeSide::Sell));
        assert_eq!(TradeSide::from_hyperliquid("X"), None);
        assert_eq!(TradeSide::from_hyperliquid(""), None);
    }

    #[test]
    fn test_trading_pair_constructors() {
        let pair = TradingPair::trades_only("BTC".to_string());
        assert_eq!(pair.coin, "BTC");
        assert!(pair.subscribe_trades);
        assert!(!pair.subscribe_orderbook);

        let pair = TradingPair::with_orderbook("ETH".to_string());
        assert_eq!(pair.coin, "ETH");
        assert!(pair.subscribe_trades);
        assert!(pair.subscribe_orderbook);
    }

    #[test]
    fn test_orderbook_best_prices() {
        let orderbook = OrderBookData {
            coin: "BTC".to_string(),
            bids: vec![
                OrderBookLevel::new(45000.0, 1.0),
                OrderBookLevel::new(44999.0, 2.0),
            ],
            asks: vec![
                OrderBookLevel::new(45001.0, 1.5),
                OrderBookLevel::new(45002.0, 2.5),
            ],
            timestamp: 1696348800000,
        };

        assert_eq!(orderbook.best_bid(), Some(45000.0));
        assert_eq!(orderbook.best_ask(), Some(45001.0));
        assert_eq!(orderbook.spread(), Some(1.0));
        
        let spread_pct = orderbook.spread_percentage().unwrap();
        assert!((spread_pct - 0.00222).abs() < 0.001); // ~0.00222%
    }

    #[test]
    fn test_orderbook_empty() {
        let orderbook = OrderBookData {
            coin: "BTC".to_string(),
            bids: vec![],
            asks: vec![],
            timestamp: 1696348800000,
        };

        assert_eq!(orderbook.best_bid(), None);
        assert_eq!(orderbook.best_ask(), None);
        assert_eq!(orderbook.spread(), None);
        assert_eq!(orderbook.spread_percentage(), None);
    }

    #[test]
    fn test_message_type_checks() {
        let trade_msg = HyperLiquidMessage::new(
            "trades".to_string(),
            MessageData::Trade(TradeData {
                coin: "BTC".to_string(),
                side: TradeSide::Buy,
                price: 45000.0,
                size: 1.0,
                timestamp: 1696348800000,
                trade_id: "123".to_string(),
            }),
        );

        assert!(trade_msg.is_trade());
        assert!(!trade_msg.is_orderbook());
        assert!(!trade_msg.is_error());
        assert!(!trade_msg.is_subscription_response());
    }

    #[test]
    fn test_market_data_event() {
        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 45000.0,
            size: 1.0,
            timestamp: 1696348800000,
            trade_id: "123".to_string(),
        };
        
        let event = MarketDataEvent::Trade(trade.clone());
        assert_eq!(event.coin(), "BTC");
        assert_eq!(event.timestamp(), 1696348800000);
        
        let orderbook = OrderBookData {
            coin: "ETH".to_string(),
            bids: vec![OrderBookLevel::new(3000.0, 10.0)],
            asks: vec![OrderBookLevel::new(3001.0, 5.0)],
            timestamp: 1696348900000,
        };
        
        let event = MarketDataEvent::OrderBook(orderbook);
        assert_eq!(event.coin(), "ETH");
        assert_eq!(event.timestamp(), 1696348900000);
    }

    #[test]
    fn test_opportunity_creation() {
        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 45000.0,
            size: 1.0,
            timestamp: 1696348800000,
            trade_id: "123".to_string(),
        };
        
        let market_data = MarketDataEvent::Trade(trade);
        let blockchain_state = BlockchainState::new(1000, 1696348800);
        
        let opportunity = Opportunity::new(
            OpportunityType::Arbitrage {
                buy_venue: "HyperLiquid".to_string(),
                sell_venue: "Uniswap".to_string(),
                token: Address::zero(),
                spread_bps: 50,
            },
            U256::from(1000000),
            market_data,
            blockchain_state,
        );
        
        assert!(!opportunity.verified);
        assert_eq!(opportunity.coin(), "BTC");
        assert_eq!(opportunity.expected_profit, U256::from(1000000));
        assert!(!opportunity.is_expired());
    }

    #[test]
    fn test_opportunity_verification() {
        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 45000.0,
            size: 1.0,
            timestamp: 1696348800000,
            trade_id: "123".to_string(),
        };
        
        let market_data = MarketDataEvent::Trade(trade);
        let blockchain_state = BlockchainState::new(1000, 1696348800);
        
        let mut opportunity = Opportunity::new(
            OpportunityType::Arbitrage {
                buy_venue: "HyperLiquid".to_string(),
                sell_venue: "Uniswap".to_string(),
                token: Address::zero(),
                spread_bps: 50,
            },
            U256::from(1000000),
            market_data,
            blockchain_state,
        );
        
        assert!(!opportunity.verified);
        opportunity.mark_verified();
        assert!(opportunity.verified);
    }

    #[test]
    fn test_opportunity_expiration() {
        let trade = TradeData {
            coin: "BTC".to_string(),
            side: TradeSide::Buy,
            price: 45000.0,
            size: 1.0,
            timestamp: 1696348800000,
            trade_id: "123".to_string(),
        };
        
        let market_data = MarketDataEvent::Trade(trade);
        let blockchain_state = BlockchainState::new(1000, 1696348800);
        
        let mut opportunity = Opportunity::new(
            OpportunityType::Liquidation {
                position: Address::zero(),
                collateral_token: Address::zero(),
                debt_token: Address::zero(),
                profit: U256::from(500000),
            },
            U256::from(500000),
            market_data,
            blockchain_state,
        );
        
        // Not expired without expiration set
        assert!(!opportunity.is_expired());
        
        // Set expiration in the past
        opportunity.set_expiration(1000);
        assert!(opportunity.is_expired());
        
        // Set expiration far in the future
        let future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64 + 1000000;
        opportunity.set_expiration(future);
        assert!(!opportunity.is_expired());
    }

    #[test]
    fn test_blockchain_state_serialization() {
        let mut state = BlockchainState::new(1000, 1696348800);
        state.token_prices.insert(Address::zero(), U256::from(45000));
        
        // Test that it can be serialized and deserialized
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: BlockchainState = serde_json::from_str(&json).unwrap();
        
        assert_eq!(deserialized.block_number, 1000);
        assert_eq!(deserialized.timestamp, 1696348800);
        assert_eq!(deserialized.token_prices.get(&Address::zero()), Some(&U256::from(45000)));
    }

    #[test]
    fn test_state_update_event_serialization() {
        let event = StateUpdateEvent::BlockNumber(1000);
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: StateUpdateEvent = serde_json::from_str(&json).unwrap();
        
        match deserialized {
            StateUpdateEvent::BlockNumber(num) => assert_eq!(num, 1000),
            _ => panic!("Wrong variant"),
        }
    }
}
