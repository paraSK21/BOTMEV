# Implementation Plan

- [x] 1. Create HyperLiquid data structures and configuration





  - Create `crates/mev-hyperliquid/` crate with Cargo.toml
  - Define `HyperLiquidConfig` struct with serde deserialization
  - Define `TradingPair`, `ReconnectConfig` structs
  - Define `TradeData`, `OrderBookData`, `HyperLiquidMessage` enums and structs
  - Add configuration validation logic
  - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5, 5.6_
-

- [x] 2. Implement TradeMessageParser




  - Create `parser.rs` module in mev-hyperliquid crate
  - Implement `TradeMessageParser` struct
  - Implement `parse_message()` to parse JSON from WebSocket
  - Implement `parse_trade()` to extract trade data
  - Implement `parse_orderbook()` to extract order book data
  - Implement `validate_message()` for message validation
  - Handle malformed JSON gracefully with error logging
  - _Requirements: 2.5, 3.1, 3.4, 6.1_

- [x] 2.1 Write unit tests for TradeMessageParser


  - Test parsing valid trade messages
  - Test parsing order book messages
  - Test handling malformed JSON
  - Test handling missing fields
  - Test handling unexpected message types
  - _Requirements: 2.5, 3.4, 6.1_

- [x] 3. Implement TradeDataAdapter





  - Create `adapter.rs` module in mev-hyperliquid crate
  - Implement `TradeDataAdapter` struct with token mapping
  - Implement `adapt_trade()` to convert TradeData to AdaptedTradeEvent
  - Implement `calculate_swap_amounts()` for amount conversion
  - Implement `map_coin_to_address()` for symbol to address mapping
  - Handle unknown coins gracefully
  - _Requirements: 8.1, 8.2, 8.3, 8.5_

- [x] 3.1 Write unit tests for TradeDataAdapter


  - Test trade to swap conversion
  - Test coin to address mapping
  - Test amount calculations with different decimals
  - Test handling unknown coins
  - _Requirements: 8.1, 8.2, 8.3_

- [x] 4. Implement WebSocket connection management





  - Create `websocket.rs` module in mev-hyperliquid crate
  - Implement `HyperLiquidWsService` struct
  - Implement `connect()` to establish WebSocket connection to wss://api.hyperliquid.xyz/ws
  - Implement connection health check with ping/pong
  - Implement graceful connection close on shutdown
  - Add connection status logging
  - _Requirements: 1.1, 1.2, 1.6_
- [x] 5. Implement reconnection logic with exponential backoff













- [ ] 5. Implement reconnection logic with exponential backoff

  - Implement `reconnect()` method with backoff calculation
  - Implement exponential backoff: min 1s, max 60s
  - Implement backoff reset after 5 minutes of stability
  - Implement max consecutive failure tracking (10 attempts)
  - Implement degraded state entry after max failures
  - Add reconnection attempt logging and metrics
  - _Requirements: 1.3, 1.4, 1.5, 6.3, 6.4, 6.5_
-

- [x] 6. Implement subscription management




  - Implement `subscribe()` method to send subscription messages
  - Format subscription message: `{"method": "subscribe", "subscription": {"type": "trades", "coin": "<SYMBOL>"}}`
  - Implement subscription confirmation handling
  - Implement subscription retry logic (5 second delay, 3 max retries)
  - Implement re-subscription on reconnect
  - Support multiple trading pair subscriptions
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.6, 6.6_
-

- [x] 7. Implement message receiving and processing loop




  - Implement `start()` method with main event loop
  - Implement `handle_message()` to process incoming WebSocket messages
  - Parse messages using TradeMessageParser
  - Send parsed messages to mpsc channel
  - Implement message ordering preservation
  - Implement backpressure handling (drop oldest if queue > 1000)
  - Add message processing latency tracking
  - _Requirements: 3.1, 3.2, 3.3, 3.5, 3.6_
-

- [x] 8. Implement Prometheus metrics



  - Add `hyperliquid_ws_connected` gauge (0/1)
  - Add `hyperliquid_trades_received_total` counter
  - Add `hyperliquid_message_processing_duration_seconds` histogram
  - Add `hyperliquid_active_subscriptions` gauge
  - Add `hyperliquid_reconnection_attempts_total` counter
  - Add `hyperliquid_degraded_state` gauge (0/1)
  - Add error counters for connection, subscription, parse, adaptation, network errors
  - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 7.6_

- [x] 9. Integrate HyperLiquid service with main bot








  - Update `crates/mev-bot/src/main.rs` to load HyperLiquid config
  - Create `HyperLiquidDataSource` wrapper implementing common DataSource trait
  - Initialize HyperLiquidWsService when `hyperliquid.enabled = true`
  - Initialize TradeDataAdapter with token mapping from config
  - Start HyperLiquid service in parallel with other data sources
  - Wire adapted trade events to strategy engine
  - _Requirements: 5.1, 5.2, 5.5, 8.4, 8.6_

- [x] 10. Update configuration file structure




  - Add `hyperliquid` section to `config/my-config.yml`
  - Add `enabled`, `ws_url`, `trading_pairs` fields
  - Add `subscribe_orderbook`, reconnection settings
  - Add `token_mapping` for coin to address mapping
  - Set default values: BTC, ETH pairs, orderbook disabled
  - Document configuration options in comments
  - _Requirements: 5.1, 5.2, 5.3, 5.4_

- [x] 11. Implement error handling and resilience



  - Add error handling for connection failures
  - Add error handling for subscription failures
  - Add error handling for parse errors
  - Add error handling for adaptation errors
  - Add error handling for network errors
  - Ensure all errors are logged with appropriate levels
  - Ensure bot continues operating despite errors
  - _Requirements: 6.1, 6.2, 6.3, 6.4, 6.5, 6.6_

- [x] 12. Add optional order book subscription support



  - Implement order book subscription message format
  - Implement order book data parsing
  - Implement in-memory order book representation
  - Calculate best bid/ask from order book
  - Detect significant spread opportunities (>10%)
  - Make order book subscription configurable
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5_

- [x] 13. Write integration tests




  - Create mock HyperLiquid WebSocket server
  - Test connection establishment and subscription flow
  - Test message reception and parsing
  - Test reconnection on disconnect
  - Test end-to-end flow: WebSocket → Parser → Adapter → Strategy Engine
  - Test error handling scenarios
  - Verify metrics are updated correctly
  - _Requirements: 1.1, 2.1, 3.1, 6.6_


- [x] 14. Update documentation



  - Add HyperLiquid integration section to README
  - Document configuration options
  - Document token mapping setup
  - Add example configuration
  - Document metrics and monitoring
  - Add troubleshooting guide
  - _Requirements: 5.1, 7.1, 7.2, 7.3, 7.4, 7.5, 7.6_
