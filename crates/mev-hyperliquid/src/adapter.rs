use crate::metrics::HyperLiquidMetrics;
use crate::types::{TradeData, TradeSide};
use anyhow::{anyhow, Result};
use ethers::types::{Address, U256};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

/// Adapter to convert HyperLiquid trade data into a format compatible with the strategy engine
#[derive(Clone)]
pub struct TradeDataAdapter {
    /// Chain ID for the target network
    #[allow(dead_code)]
    chain_id: u64,
    
    /// Maps HyperLiquid coin symbols to EVM token addresses
    token_mapping: HashMap<String, Address>,
    
    /// Default decimals for tokens (most use 18, but some like USDC use 6)
    token_decimals: HashMap<String, u8>,
    
    /// Prometheus metrics
    metrics: Option<Arc<HyperLiquidMetrics>>,
}

/// Adapted trade event compatible with the strategy engine
#[derive(Debug, Clone)]
pub struct AdaptedTradeEvent {
    /// Unique identifier for this trade (prefixed with "hl_trade_")
    pub hash: String,
    
    /// Source address (HyperLiquid exchange)
    pub from: Address,
    
    /// Destination address (synthetic buyer/seller)
    pub to: Address,
    
    /// Value transferred (typically zero for token swaps)
    pub value: U256,
    
    /// Gas price (synthetic, set to zero for HyperLiquid)
    pub gas_price: U256,
    
    /// Timestamp in seconds
    pub timestamp: u64,
    
    /// Swap data extracted from the trade
    pub swap_data: Option<SwapData>,
}

/// Swap data representing a DEX trade
#[derive(Debug, Clone)]
pub struct SwapData {
    /// DEX name (always "HyperLiquid" for this adapter)
    pub dex: String,
    
    /// Input token address (what was sold)
    pub token_in: Address,
    
    /// Output token address (what was bought)
    pub token_out: Address,
    
    /// Amount of input token
    pub amount_in: U256,
    
    /// Amount of output token
    pub amount_out: U256,
    
    /// Price of the trade
    pub price: f64,
}

impl TradeDataAdapter {
    /// Create a new TradeDataAdapter
    ///
    /// # Arguments
    /// * `chain_id` - The chain ID for the target network
    /// * `token_mapping` - Map of coin symbols to EVM addresses (e.g., "BTC" -> WBTC address)
    pub fn new(chain_id: u64, token_mapping: HashMap<String, String>) -> Result<Self> {
        // Convert string addresses to Address type
        let mut parsed_mapping = HashMap::new();
        for (coin, addr_str) in token_mapping {
            let address = Address::from_str(&addr_str)
                .map_err(|e| anyhow!("Invalid address for {}: {}", coin, e))?;
            parsed_mapping.insert(coin, address);
        }
        
        // Set up default decimals (can be extended based on config)
        let mut token_decimals = HashMap::new();
        token_decimals.insert("USDC".to_string(), 6);
        token_decimals.insert("USDT".to_string(), 6);
        // Most other tokens use 18 decimals
        
        Ok(Self {
            chain_id,
            token_mapping: parsed_mapping,
            token_decimals,
            metrics: None,
        })
    }
    
    /// Create a new TradeDataAdapter with metrics
    ///
    /// # Arguments
    /// * `chain_id` - The chain ID for the target network
    /// * `token_mapping` - Map of coin symbols to EVM addresses (e.g., "BTC" -> WBTC address)
    /// * `metrics` - Prometheus metrics collector
    pub fn new_with_metrics(
        chain_id: u64,
        token_mapping: HashMap<String, String>,
        metrics: Arc<HyperLiquidMetrics>,
    ) -> Result<Self> {
        let mut adapter = Self::new(chain_id, token_mapping)?;
        adapter.metrics = Some(metrics);
        Ok(adapter)
    }
    
    /// Adapt a HyperLiquid trade into an AdaptedTradeEvent
    ///
    /// # Arguments
    /// * `trade` - The trade data from HyperLiquid
    ///
    /// # Returns
    /// An AdaptedTradeEvent that can be processed by the strategy engine
    pub fn adapt_trade(&self, trade: &TradeData) -> Result<AdaptedTradeEvent> {
        // Generate a unique hash for this trade
        let hash = format!("hl_trade_{}", trade.trade_id);
        
        // HyperLiquid exchange address (synthetic)
        let hyperliquid_exchange = Address::from_str("0x0000000000000000000000000000000000000001")
            .expect("Valid address");
        
        // Synthetic buyer/seller address
        let counterparty = Address::from_str("0x0000000000000000000000000000000000000002")
            .expect("Valid address");
        
        // Convert timestamp from milliseconds to seconds
        let timestamp = trade.timestamp / 1000;
        
        // Create swap data
        let swap_data = self.create_swap_data(trade)?;
        
        Ok(AdaptedTradeEvent {
            hash,
            from: hyperliquid_exchange,
            to: counterparty,
            value: U256::zero(),
            gas_price: U256::zero(),
            timestamp,
            swap_data: Some(swap_data),
        })
    }
    
    /// Create swap data from a trade
    fn create_swap_data(&self, trade: &TradeData) -> Result<SwapData> {
        // Map the coin to an EVM address
        let coin_address = self.map_coin_to_address(&trade.coin)
            .ok_or_else(|| {
                // Track adaptation error
                if let Some(ref metrics) = self.metrics {
                    metrics.inc_adaptation_errors(&trade.coin, "unknown_coin");
                }
                anyhow!("Unknown coin: {}", trade.coin)
            })?;
        
        // Assume USDC as the quote currency
        let usdc_address = self.map_coin_to_address("USDC")
            .ok_or_else(|| {
                // Track adaptation error
                if let Some(ref metrics) = self.metrics {
                    metrics.inc_adaptation_errors("USDC", "missing_usdc_mapping");
                }
                anyhow!("USDC not configured in token mapping")
            })?;
        
        // Determine token_in and token_out based on trade side
        let (token_in, token_out) = match trade.side {
            TradeSide::Buy => {
                // Buy: USDC -> Coin
                (usdc_address, coin_address)
            }
            TradeSide::Sell => {
                // Sell: Coin -> USDC
                (coin_address, usdc_address)
            }
        };
        
        // Calculate swap amounts based on trade side
        let (amount_in, amount_out) = self.calculate_swap_amounts(trade)?;
        
        Ok(SwapData {
            dex: "HyperLiquid".to_string(),
            token_in,
            token_out,
            amount_in,
            amount_out,
            price: trade.price,
        })
    }
    
    /// Calculate swap amounts from trade data
    ///
    /// For a BUY: user pays USDC (token_in) to get BTC (token_out)
    /// For a SELL: user pays BTC (token_in) to get USDC (token_out)
    ///
    /// # Arguments
    /// * `trade` - The trade data
    ///
    /// # Returns
    /// Tuple of (amount_in, amount_out) in their respective token decimals
    pub fn calculate_swap_amounts(&self, trade: &TradeData) -> Result<(U256, U256)> {
        let coin_decimals = self.get_token_decimals(&trade.coin);
        let usdc_decimals = self.get_token_decimals("USDC");
        
        // Calculate the total value in USDC
        let usdc_value = trade.price * trade.size;
        
        match trade.side {
            TradeSide::Buy => {
                // Buy: USDC -> Coin
                // amount_in = USDC value
                // amount_out = coin size
                let amount_in = self.float_to_u256(usdc_value, usdc_decimals)?;
                let amount_out = self.float_to_u256(trade.size, coin_decimals)?;
                Ok((amount_in, amount_out))
            }
            TradeSide::Sell => {
                // Sell: Coin -> USDC
                // amount_in = coin size
                // amount_out = USDC value
                let amount_in = self.float_to_u256(trade.size, coin_decimals)?;
                let amount_out = self.float_to_u256(usdc_value, usdc_decimals)?;
                Ok((amount_in, amount_out))
            }
        }
    }
    
    /// Map a coin symbol to its EVM address
    ///
    /// # Arguments
    /// * `coin` - The coin symbol (e.g., "BTC", "ETH")
    ///
    /// # Returns
    /// The corresponding EVM address, or None if not found
    pub fn map_coin_to_address(&self, coin: &str) -> Option<Address> {
        self.token_mapping.get(coin).copied()
    }
    
    /// Get the number of decimals for a token
    fn get_token_decimals(&self, coin: &str) -> u8 {
        self.token_decimals.get(coin).copied().unwrap_or(18)
    }
    
    /// Convert a float value to U256 with the specified decimals
    fn float_to_u256(&self, value: f64, decimals: u8) -> Result<U256> {
        if value < 0.0 {
            return Err(anyhow!("Cannot convert negative value to U256"));
        }
        
        // Multiply by 10^decimals to convert to integer representation
        let multiplier = 10_f64.powi(decimals as i32);
        let scaled_value = value * multiplier;
        
        // Check for overflow
        if scaled_value > u128::MAX as f64 {
            return Err(anyhow!("Value too large to convert to U256"));
        }
        
        Ok(U256::from(scaled_value as u128))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_adapter() -> TradeDataAdapter {
        let mut token_mapping = HashMap::new();
        token_mapping.insert("BTC".to_string(), "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599".to_string());
        token_mapping.insert("ETH".to_string(), "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2".to_string());
        token_mapping.insert("USDC".to_string(), "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".to_string());
        
        TradeDataAdapter::new(1, token_mapping).unwrap()
    }

    fn create_test_trade(coin: &str, side: TradeSide, price: f64, size: f64) -> TradeData {
        TradeData {
            coin: coin.to_string(),
            side,
            price,
            size,
            timestamp: 1696348800000,
            trade_id: "test_123".to_string(),
        }
    }

    #[test]
    fn test_adapter_creation() {
        let adapter = create_test_adapter();
        assert_eq!(adapter.chain_id, 1);
        assert_eq!(adapter.token_mapping.len(), 3);
    }

    #[test]
    fn test_adapter_creation_invalid_address() {
        let mut token_mapping = HashMap::new();
        token_mapping.insert("BTC".to_string(), "invalid_address".to_string());
        
        let result = TradeDataAdapter::new(1, token_mapping);
        assert!(result.is_err());
    }

    #[test]
    fn test_map_coin_to_address() {
        let adapter = create_test_adapter();
        
        let btc_addr = adapter.map_coin_to_address("BTC");
        assert!(btc_addr.is_some());
        assert_eq!(
            btc_addr.unwrap(),
            Address::from_str("0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599").unwrap()
        );
        
        let unknown = adapter.map_coin_to_address("UNKNOWN");
        assert!(unknown.is_none());
    }

    #[test]
    fn test_calculate_swap_amounts_buy() {
        let adapter = create_test_adapter();
        let trade = create_test_trade("BTC", TradeSide::Buy, 45000.0, 0.5);
        
        let (amount_in, amount_out) = adapter.calculate_swap_amounts(&trade).unwrap();
        
        // Buy 0.5 BTC at 45000 = 22500 USDC
        // USDC has 6 decimals: 22500 * 10^6 = 22500000000
        assert_eq!(amount_in, U256::from(22500000000u64));
        
        // BTC has 18 decimals: 0.5 * 10^18 = 500000000000000000
        assert_eq!(amount_out, U256::from(500000000000000000u64));
    }

    #[test]
    fn test_calculate_swap_amounts_sell() {
        let adapter = create_test_adapter();
        let trade = create_test_trade("BTC", TradeSide::Sell, 45000.0, 0.5);
        
        let (amount_in, amount_out) = adapter.calculate_swap_amounts(&trade).unwrap();
        
        // Sell 0.5 BTC at 45000 = 22500 USDC
        // BTC has 18 decimals: 0.5 * 10^18 = 500000000000000000
        assert_eq!(amount_in, U256::from(500000000000000000u64));
        
        // USDC has 6 decimals: 22500 * 10^6 = 22500000000
        assert_eq!(amount_out, U256::from(22500000000u64));
    }

    #[test]
    fn test_calculate_swap_amounts_different_decimals() {
        let adapter = create_test_adapter();
        
        // ETH uses 18 decimals (default)
        let trade = create_test_trade("ETH", TradeSide::Buy, 3000.0, 1.0);
        let (amount_in, amount_out) = adapter.calculate_swap_amounts(&trade).unwrap();
        
        // Buy 1 ETH at 3000 = 3000 USDC
        // USDC: 3000 * 10^6 = 3000000000
        assert_eq!(amount_in, U256::from(3000000000u64));
        
        // ETH: 1.0 * 10^18 = 1000000000000000000
        assert_eq!(amount_out, U256::from(1000000000000000000u64));
    }

    #[test]
    fn test_adapt_trade_buy() {
        let adapter = create_test_adapter();
        let trade = create_test_trade("BTC", TradeSide::Buy, 45000.0, 0.5);
        
        let adapted = adapter.adapt_trade(&trade).unwrap();
        
        assert_eq!(adapted.hash, "hl_trade_test_123");
        assert_eq!(adapted.timestamp, 1696348800); // milliseconds to seconds
        assert!(adapted.swap_data.is_some());
        
        let swap = adapted.swap_data.unwrap();
        assert_eq!(swap.dex, "HyperLiquid");
        assert_eq!(swap.price, 45000.0);
        assert_eq!(
            swap.token_out,
            Address::from_str("0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599").unwrap()
        );
        assert_eq!(
            swap.token_in,
            Address::from_str("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").unwrap()
        );
    }

    #[test]
    fn test_adapt_trade_sell() {
        let adapter = create_test_adapter();
        let trade = create_test_trade("ETH", TradeSide::Sell, 3000.0, 2.0);
        
        let adapted = adapter.adapt_trade(&trade).unwrap();
        
        assert!(adapted.swap_data.is_some());
        let swap = adapted.swap_data.unwrap();
        
        // For sell, token_in should be ETH, token_out should be USDC
        assert_eq!(
            swap.token_in,
            Address::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap()
        );
        assert_eq!(
            swap.token_out,
            Address::from_str("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").unwrap()
        );
    }

    #[test]
    fn test_adapt_trade_unknown_coin() {
        let adapter = create_test_adapter();
        let trade = create_test_trade("UNKNOWN", TradeSide::Buy, 100.0, 1.0);
        
        let result = adapter.adapt_trade(&trade);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown coin"));
    }

    #[test]
    fn test_float_to_u256() {
        let adapter = create_test_adapter();
        
        // Test with 18 decimals
        let result = adapter.float_to_u256(1.5, 18).unwrap();
        assert_eq!(result, U256::from(1500000000000000000u64));
        
        // Test with 6 decimals
        let result = adapter.float_to_u256(1000.5, 6).unwrap();
        assert_eq!(result, U256::from(1000500000u64));
        
        // Test with 0 decimals
        let result = adapter.float_to_u256(42.0, 0).unwrap();
        assert_eq!(result, U256::from(42u64));
    }

    #[test]
    fn test_float_to_u256_negative() {
        let adapter = create_test_adapter();
        let result = adapter.float_to_u256(-1.0, 18);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_token_decimals() {
        let adapter = create_test_adapter();
        
        assert_eq!(adapter.get_token_decimals("USDC"), 6);
        assert_eq!(adapter.get_token_decimals("USDT"), 6);
        assert_eq!(adapter.get_token_decimals("BTC"), 18); // default
        assert_eq!(adapter.get_token_decimals("ETH"), 18); // default
        assert_eq!(adapter.get_token_decimals("UNKNOWN"), 18); // default
    }
}
