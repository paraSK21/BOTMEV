use crate::types::{
    ErrorData, HyperLiquidMessage, MessageData, OrderBookData, OrderBookLevel,
    SubscriptionResponse, TradeData, TradeSide,
};
use serde_json::Value;
use tracing::{error, warn};

/// Parser for HyperLiquid WebSocket messages
pub struct TradeMessageParser;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("Invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid field value: {0}")]
    InvalidValue(String),

    #[error("Unknown message type: {0}")]
    UnknownMessageType(String),

    #[error("Validation failed: {0}")]
    ValidationFailed(String),
}

impl TradeMessageParser {
    /// Create a new parser instance
    pub fn new() -> Self {
        Self
    }

    /// Parse a raw WebSocket message into a structured HyperLiquidMessage
    pub fn parse_message(&self, raw: &str) -> Result<HyperLiquidMessage, ParseError> {
        // Parse JSON
        let value: Value = serde_json::from_str(raw).map_err(|e| {
            error!("Failed to parse JSON: {}", e);
            ParseError::InvalidJson(e)
        })?;

        // Extract channel
        let channel = value
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                error!("Missing 'channel' field in message");
                ParseError::MissingField("channel".to_string())
            })?
            .to_string();

        // Extract data object
        let data_value = value.get("data").ok_or_else(|| {
            error!("Missing 'data' field in message");
            ParseError::MissingField("data".to_string())
        })?;

        // Parse based on channel type
        let data = match channel.as_str() {
            "trades" => {
                // HyperLiquid sends trades as an array, so we need to handle multiple trades
                if let Some(trades_array) = data_value.as_array() {
                    // For now, we'll process the first trade in the array
                    // In a production system, you might want to process all trades
                    if let Some(first_trade) = trades_array.first() {
                        let trade = self.parse_trade(first_trade)?;
                        MessageData::Trade(trade)
                    } else {
                        return Err(ParseError::MissingField("empty trades array".to_string()));
                    }
                } else {
                    // Fallback: try to parse as single trade object
                    let trade = self.parse_trade(data_value)?;
                    MessageData::Trade(trade)
                }
            }
            "l2Book" => {
                let orderbook = self.parse_orderbook(data_value)?;
                MessageData::OrderBook(orderbook)
            }
            "subscriptionResponse" => {
                // HyperLiquid subscription response format: {"channel":"subscriptionResponse","data":{"method":"subscribe","subscription":{"type":"trades","coin":"BTC"}}}
                // The data field contains the subscription details, not a direct response
                if let Some(method) = data_value.get("method").and_then(|v| v.as_str()) {
                    if method == "subscribe" {
                        if let Some(subscription) = data_value.get("subscription") {
                            let subscription_type = subscription
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            
                            let coin = subscription
                                .get("coin")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            
                            let response = SubscriptionResponse {
                                subscription_type,
                                coin,
                                success: true,
                                error: None,
                            };
                            MessageData::SubscriptionResponse(response)
                        } else {
                            return Err(ParseError::MissingField("subscription".to_string()));
                        }
                    } else {
                        // Fallback: try to parse as regular subscription response
                        let response = self.parse_subscription_response(data_value)?;
                        MessageData::SubscriptionResponse(response)
                    }
                } else {
                    // Fallback: try to parse as regular subscription response
                    let response = self.parse_subscription_response(data_value)?;
                    MessageData::SubscriptionResponse(response)
                }
            }
            "error" => {
                // HyperLiquid error format: {"channel":"error","data":"Already subscribed: {...}"}
                // The data field is a string, not an object
                let error_message = if let Some(message_str) = data_value.as_str() {
                    message_str.to_string()
                } else {
                    // Fallback: try to parse as error object
                    let error = self.parse_error(data_value)?;
                    return Ok(HyperLiquidMessage::new(channel, MessageData::Error(error)));
                };
                
                let error_data = ErrorData {
                    code: None,
                    message: error_message,
                };
                MessageData::Error(error_data)
            }
            _ => {
                warn!("Unknown channel type: {}", channel);
                return Err(ParseError::UnknownMessageType(channel));
            }
        };

        let message = HyperLiquidMessage::new(channel, data);

        // Validate the parsed message
        self.validate_message(&message)?;

        Ok(message)
    }

    /// Parse trade data from JSON value
    pub fn parse_trade(&self, data: &Value) -> Result<TradeData, ParseError> {
        let coin = data
            .get("coin")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ParseError::MissingField("coin".to_string()))?
            .to_string();

        let side_str = data
            .get("side")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ParseError::MissingField("side".to_string()))?;

        let side = TradeSide::from_hyperliquid(side_str).ok_or_else(|| {
            ParseError::InvalidValue(format!("Invalid trade side: {}", side_str))
        })?;

        let price = self.parse_f64(data, "px")?;
        let size = self.parse_f64(data, "sz")?;

        let timestamp = data
            .get("time")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ParseError::MissingField("time".to_string()))?;

        let trade_id = data
            .get("tid")
            .map(|v| {
                // Handle both string and number trade IDs
                if let Some(s) = v.as_str() {
                    s.to_string()
                } else if let Some(n) = v.as_u64() {
                    n.to_string()
                } else if let Some(n) = v.as_i64() {
                    n.to_string()
                } else {
                    "unknown".to_string()
                }
            })
            .ok_or_else(|| ParseError::MissingField("tid".to_string()))?;

        Ok(TradeData {
            coin,
            side,
            price,
            size,
            timestamp,
            trade_id,
        })
    }

    /// Parse order book data from JSON value
    pub fn parse_orderbook(&self, data: &Value) -> Result<OrderBookData, ParseError> {
        let coin = data
            .get("coin")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ParseError::MissingField("coin".to_string()))?
            .to_string();

        let bids_array = data
            .get("bids")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ParseError::MissingField("bids".to_string()))?;

        let asks_array = data
            .get("asks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ParseError::MissingField("asks".to_string()))?;

        let bids = self.parse_orderbook_levels(bids_array)?;
        let asks = self.parse_orderbook_levels(asks_array)?;

        let timestamp = data
            .get("time")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ParseError::MissingField("time".to_string()))?;

        Ok(OrderBookData {
            coin,
            bids,
            asks,
            timestamp,
        })
    }

    /// Parse order book levels from array
    fn parse_orderbook_levels(&self, levels: &[Value]) -> Result<Vec<OrderBookLevel>, ParseError> {
        levels
            .iter()
            .map(|level| {
                let price = level
                    .get("px")
                    .or_else(|| level.get(0))
                    .and_then(|v| self.value_to_f64(v))
                    .ok_or_else(|| ParseError::MissingField("price in level".to_string()))?;

                let size = level
                    .get("sz")
                    .or_else(|| level.get(1))
                    .and_then(|v| self.value_to_f64(v))
                    .ok_or_else(|| ParseError::MissingField("size in level".to_string()))?;

                Ok(OrderBookLevel::new(price, size))
            })
            .collect()
    }

    /// Parse subscription response from JSON value
    fn parse_subscription_response(
        &self,
        data: &Value,
    ) -> Result<SubscriptionResponse, ParseError> {
        let subscription_type = data
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ParseError::MissingField("type".to_string()))?
            .to_string();

        let coin = data
            .get("coin")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ParseError::MissingField("coin".to_string()))?
            .to_string();

        let success = data
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let error = data
            .get("error")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(SubscriptionResponse {
            subscription_type,
            coin,
            success,
            error,
        })
    }

    /// Parse error data from JSON value
    fn parse_error(&self, data: &Value) -> Result<ErrorData, ParseError> {
        let code = data.get("code").and_then(|v| v.as_i64()).map(|c| c as i32);

        let message = data
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ParseError::MissingField("message".to_string()))?
            .to_string();

        Ok(ErrorData { code, message })
    }

    /// Validate a parsed message
    pub fn validate_message(&self, msg: &HyperLiquidMessage) -> Result<(), ParseError> {
        match &msg.data {
            MessageData::Trade(trade) => {
                if trade.coin.is_empty() {
                    return Err(ParseError::ValidationFailed("Empty coin symbol".to_string()));
                }
                if trade.price <= 0.0 {
                    return Err(ParseError::ValidationFailed(format!(
                        "Invalid price: {}",
                        trade.price
                    )));
                }
                if trade.size <= 0.0 {
                    return Err(ParseError::ValidationFailed(format!(
                        "Invalid size: {}",
                        trade.size
                    )));
                }
                if trade.timestamp == 0 {
                    return Err(ParseError::ValidationFailed(
                        "Invalid timestamp: 0".to_string(),
                    ));
                }
                if trade.trade_id.is_empty() {
                    return Err(ParseError::ValidationFailed(
                        "Empty trade ID".to_string(),
                    ));
                }
            }
            MessageData::OrderBook(orderbook) => {
                if orderbook.coin.is_empty() {
                    return Err(ParseError::ValidationFailed("Empty coin symbol".to_string()));
                }
                if orderbook.timestamp == 0 {
                    return Err(ParseError::ValidationFailed(
                        "Invalid timestamp: 0".to_string(),
                    ));
                }
                // Validate bid/ask levels
                for bid in &orderbook.bids {
                    if bid.price <= 0.0 || bid.size <= 0.0 {
                        return Err(ParseError::ValidationFailed(format!(
                            "Invalid bid level: price={}, size={}",
                            bid.price, bid.size
                        )));
                    }
                }
                for ask in &orderbook.asks {
                    if ask.price <= 0.0 || ask.size <= 0.0 {
                        return Err(ParseError::ValidationFailed(format!(
                            "Invalid ask level: price={}, size={}",
                            ask.price, ask.size
                        )));
                    }
                }
            }
            MessageData::SubscriptionResponse(response) => {
                if response.subscription_type.is_empty() {
                    return Err(ParseError::ValidationFailed(
                        "Empty subscription type".to_string(),
                    ));
                }
                if response.coin.is_empty() {
                    return Err(ParseError::ValidationFailed("Empty coin symbol".to_string()));
                }
            }
            MessageData::Error(error) => {
                if error.message.is_empty() {
                    return Err(ParseError::ValidationFailed(
                        "Empty error message".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Helper to parse f64 from JSON value with field name
    fn parse_f64(&self, data: &Value, field: &str) -> Result<f64, ParseError> {
        data.get(field)
            .and_then(|v| self.value_to_f64(v))
            .ok_or_else(|| ParseError::MissingField(field.to_string()))
    }

    /// Helper to convert JSON value to f64 (handles both number and string)
    fn value_to_f64(&self, value: &Value) -> Option<f64> {
        match value {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.parse::<f64>().ok(),
            _ => None,
        }
    }
}

impl Default for TradeMessageParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_trade_message() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "channel": "trades",
            "data": {
                "coin": "BTC",
                "side": "B",
                "px": "45000.5",
                "sz": "0.5",
                "time": 1696348800000,
                "tid": "12345"
            }
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_ok());

        let msg = result.unwrap();
        assert_eq!(msg.channel, "trades");
        assert!(msg.is_trade());

        if let MessageData::Trade(trade) = msg.data {
            assert_eq!(trade.coin, "BTC");
            assert_eq!(trade.side, TradeSide::Buy);
            assert_eq!(trade.price, 45000.5);
            assert_eq!(trade.size, 0.5);
            assert_eq!(trade.timestamp, 1696348800000);
            assert_eq!(trade.trade_id, "12345");
        } else {
            panic!("Expected Trade message");
        }
    }

    #[test]
    fn test_parse_trade_with_sell_side() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "channel": "trades",
            "data": {
                "coin": "ETH",
                "side": "A",
                "px": "3000.25",
                "sz": "2.0",
                "time": 1696348800000,
                "tid": "67890"
            }
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_ok());

        let msg = result.unwrap();
        if let MessageData::Trade(trade) = msg.data {
            assert_eq!(trade.coin, "ETH");
            assert_eq!(trade.side, TradeSide::Sell);
            assert_eq!(trade.price, 3000.25);
            assert_eq!(trade.size, 2.0);
        } else {
            panic!("Expected Trade message");
        }
    }

    #[test]
    fn test_parse_orderbook_message() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "channel": "l2Book",
            "data": {
                "coin": "BTC",
                "bids": [
                    {"px": "45000.0", "sz": "1.0"},
                    {"px": "44999.0", "sz": "2.0"}
                ],
                "asks": [
                    {"px": "45001.0", "sz": "1.5"},
                    {"px": "45002.0", "sz": "2.5"}
                ],
                "time": 1696348800000
            }
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_ok());

        let msg = result.unwrap();
        assert_eq!(msg.channel, "l2Book");
        assert!(msg.is_orderbook());

        if let MessageData::OrderBook(orderbook) = msg.data {
            assert_eq!(orderbook.coin, "BTC");
            assert_eq!(orderbook.bids.len(), 2);
            assert_eq!(orderbook.asks.len(), 2);
            assert_eq!(orderbook.bids[0].price, 45000.0);
            assert_eq!(orderbook.bids[0].size, 1.0);
            assert_eq!(orderbook.asks[0].price, 45001.0);
            assert_eq!(orderbook.asks[0].size, 1.5);
            assert_eq!(orderbook.timestamp, 1696348800000);
        } else {
            panic!("Expected OrderBook message");
        }
    }

    #[test]
    fn test_parse_orderbook_with_array_format() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "channel": "l2Book",
            "data": {
                "coin": "ETH",
                "bids": [
                    ["3000.0", "5.0"],
                    ["2999.0", "10.0"]
                ],
                "asks": [
                    ["3001.0", "3.0"]
                ],
                "time": 1696348800000
            }
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_ok());

        let msg = result.unwrap();
        if let MessageData::OrderBook(orderbook) = msg.data {
            assert_eq!(orderbook.coin, "ETH");
            assert_eq!(orderbook.bids.len(), 2);
            assert_eq!(orderbook.bids[0].price, 3000.0);
            assert_eq!(orderbook.bids[0].size, 5.0);
        } else {
            panic!("Expected OrderBook message");
        }
    }

    #[test]
    fn test_parse_malformed_json() {
        let parser = TradeMessageParser::new();
        let raw = r#"{ invalid json }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::InvalidJson(_)));
    }

    #[test]
    fn test_parse_missing_channel_field() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "data": {
                "coin": "BTC"
            }
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ParseError::MissingField(ref field) if field == "channel"
        ));
    }

    #[test]
    fn test_parse_missing_data_field() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "channel": "trades"
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ParseError::MissingField(ref field) if field == "data"
        ));
    }

    #[test]
    fn test_parse_missing_trade_fields() {
        let parser = TradeMessageParser::new();
        
        // Missing coin
        let raw = r#"{
            "channel": "trades",
            "data": {
                "side": "B",
                "px": "45000.5",
                "sz": "0.5",
                "time": 1696348800000,
                "tid": "12345"
            }
        }"#;
        assert!(parser.parse_message(raw).is_err());

        // Missing side
        let raw = r#"{
            "channel": "trades",
            "data": {
                "coin": "BTC",
                "px": "45000.5",
                "sz": "0.5",
                "time": 1696348800000,
                "tid": "12345"
            }
        }"#;
        assert!(parser.parse_message(raw).is_err());

        // Missing price
        let raw = r#"{
            "channel": "trades",
            "data": {
                "coin": "BTC",
                "side": "B",
                "sz": "0.5",
                "time": 1696348800000,
                "tid": "12345"
            }
        }"#;
        assert!(parser.parse_message(raw).is_err());
    }

    #[test]
    fn test_parse_invalid_trade_side() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "channel": "trades",
            "data": {
                "coin": "BTC",
                "side": "X",
                "px": "45000.5",
                "sz": "0.5",
                "time": 1696348800000,
                "tid": "12345"
            }
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::InvalidValue(_)));
    }

    #[test]
    fn test_parse_unknown_message_type() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "channel": "unknown_channel",
            "data": {}
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ParseError::UnknownMessageType(_)
        ));
    }

    #[test]
    fn test_validate_trade_message() {
        let parser = TradeMessageParser::new();

        // Valid trade
        let valid_trade = HyperLiquidMessage::new(
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
        assert!(parser.validate_message(&valid_trade).is_ok());

        // Invalid: empty coin
        let invalid_trade = HyperLiquidMessage::new(
            "trades".to_string(),
            MessageData::Trade(TradeData {
                coin: "".to_string(),
                side: TradeSide::Buy,
                price: 45000.0,
                size: 1.0,
                timestamp: 1696348800000,
                trade_id: "123".to_string(),
            }),
        );
        assert!(parser.validate_message(&invalid_trade).is_err());

        // Invalid: zero price
        let invalid_trade = HyperLiquidMessage::new(
            "trades".to_string(),
            MessageData::Trade(TradeData {
                coin: "BTC".to_string(),
                side: TradeSide::Buy,
                price: 0.0,
                size: 1.0,
                timestamp: 1696348800000,
                trade_id: "123".to_string(),
            }),
        );
        assert!(parser.validate_message(&invalid_trade).is_err());

        // Invalid: negative size
        let invalid_trade = HyperLiquidMessage::new(
            "trades".to_string(),
            MessageData::Trade(TradeData {
                coin: "BTC".to_string(),
                side: TradeSide::Buy,
                price: 45000.0,
                size: -1.0,
                timestamp: 1696348800000,
                trade_id: "123".to_string(),
            }),
        );
        assert!(parser.validate_message(&invalid_trade).is_err());

        // Invalid: zero timestamp
        let invalid_trade = HyperLiquidMessage::new(
            "trades".to_string(),
            MessageData::Trade(TradeData {
                coin: "BTC".to_string(),
                side: TradeSide::Buy,
                price: 45000.0,
                size: 1.0,
                timestamp: 0,
                trade_id: "123".to_string(),
            }),
        );
        assert!(parser.validate_message(&invalid_trade).is_err());

        // Invalid: empty trade_id
        let invalid_trade = HyperLiquidMessage::new(
            "trades".to_string(),
            MessageData::Trade(TradeData {
                coin: "BTC".to_string(),
                side: TradeSide::Buy,
                price: 45000.0,
                size: 1.0,
                timestamp: 1696348800000,
                trade_id: "".to_string(),
            }),
        );
        assert!(parser.validate_message(&invalid_trade).is_err());
    }

    #[test]
    fn test_validate_orderbook_message() {
        let parser = TradeMessageParser::new();

        // Valid orderbook
        let valid_orderbook = HyperLiquidMessage::new(
            "l2Book".to_string(),
            MessageData::OrderBook(OrderBookData {
                coin: "BTC".to_string(),
                bids: vec![OrderBookLevel::new(45000.0, 1.0)],
                asks: vec![OrderBookLevel::new(45001.0, 1.0)],
                timestamp: 1696348800000,
            }),
        );
        assert!(parser.validate_message(&valid_orderbook).is_ok());

        // Invalid: empty coin
        let invalid_orderbook = HyperLiquidMessage::new(
            "l2Book".to_string(),
            MessageData::OrderBook(OrderBookData {
                coin: "".to_string(),
                bids: vec![],
                asks: vec![],
                timestamp: 1696348800000,
            }),
        );
        assert!(parser.validate_message(&invalid_orderbook).is_err());

        // Invalid: zero timestamp
        let invalid_orderbook = HyperLiquidMessage::new(
            "l2Book".to_string(),
            MessageData::OrderBook(OrderBookData {
                coin: "BTC".to_string(),
                bids: vec![],
                asks: vec![],
                timestamp: 0,
            }),
        );
        assert!(parser.validate_message(&invalid_orderbook).is_err());

        // Invalid: negative price in bid
        let invalid_orderbook = HyperLiquidMessage::new(
            "l2Book".to_string(),
            MessageData::OrderBook(OrderBookData {
                coin: "BTC".to_string(),
                bids: vec![OrderBookLevel::new(-45000.0, 1.0)],
                asks: vec![],
                timestamp: 1696348800000,
            }),
        );
        assert!(parser.validate_message(&invalid_orderbook).is_err());

        // Invalid: zero size in ask
        let invalid_orderbook = HyperLiquidMessage::new(
            "l2Book".to_string(),
            MessageData::OrderBook(OrderBookData {
                coin: "BTC".to_string(),
                bids: vec![],
                asks: vec![OrderBookLevel::new(45001.0, 0.0)],
                timestamp: 1696348800000,
            }),
        );
        assert!(parser.validate_message(&invalid_orderbook).is_err());
    }

    #[test]
    fn test_parse_subscription_response() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "channel": "subscriptionResponse",
            "data": {
                "type": "trades",
                "coin": "BTC",
                "success": true
            }
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_ok());

        let msg = result.unwrap();
        assert!(msg.is_subscription_response());

        if let MessageData::SubscriptionResponse(response) = msg.data {
            assert_eq!(response.subscription_type, "trades");
            assert_eq!(response.coin, "BTC");
            assert!(response.success);
            assert!(response.error.is_none());
        } else {
            panic!("Expected SubscriptionResponse message");
        }
    }

    #[test]
    fn test_parse_subscription_response_with_error() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "channel": "subscriptionResponse",
            "data": {
                "type": "trades",
                "coin": "INVALID",
                "success": false,
                "error": "Unknown coin symbol"
            }
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_ok());

        let msg = result.unwrap();
        if let MessageData::SubscriptionResponse(response) = msg.data {
            assert_eq!(response.subscription_type, "trades");
            assert_eq!(response.coin, "INVALID");
            assert!(!response.success);
            assert_eq!(response.error, Some("Unknown coin symbol".to_string()));
        } else {
            panic!("Expected SubscriptionResponse message");
        }
    }

    #[test]
    fn test_parse_error_message() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "channel": "error",
            "data": {
                "code": 400,
                "message": "Invalid request"
            }
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_ok());

        let msg = result.unwrap();
        assert!(msg.is_error());

        if let MessageData::Error(error) = msg.data {
            assert_eq!(error.code, Some(400));
            assert_eq!(error.message, "Invalid request");
        } else {
            panic!("Expected Error message");
        }
    }

    #[test]
    fn test_parse_error_message_without_code() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "channel": "error",
            "data": {
                "message": "Connection lost"
            }
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_ok());

        let msg = result.unwrap();
        if let MessageData::Error(error) = msg.data {
            assert_eq!(error.code, None);
            assert_eq!(error.message, "Connection lost");
        } else {
            panic!("Expected Error message");
        }
    }

    #[test]
    fn test_parse_numeric_values_as_numbers() {
        let parser = TradeMessageParser::new();
        let raw = r#"{
            "channel": "trades",
            "data": {
                "coin": "BTC",
                "side": "B",
                "px": 45000.5,
                "sz": 0.5,
                "time": 1696348800000,
                "tid": "12345"
            }
        }"#;

        let result = parser.parse_message(raw);
        assert!(result.is_ok());

        let msg = result.unwrap();
        if let MessageData::Trade(trade) = msg.data {
            assert_eq!(trade.price, 45000.5);
            assert_eq!(trade.size, 0.5);
        } else {
            panic!("Expected Trade message");
        }
    }
}
