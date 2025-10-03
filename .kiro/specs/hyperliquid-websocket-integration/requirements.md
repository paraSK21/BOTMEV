# Requirements Document

## Introduction

This feature integrates HyperLiquid's native WebSocket API into the MEV bot to enable real-time monitoring of trades and market activity on the HyperLiquid DEX. Unlike standard Ethereum mempool monitoring, HyperLiquid uses a custom WebSocket protocol that streams trade data, order book updates, and other exchange events. This integration will replace the current non-functional EVM mempool polling with a working real-time data feed that can detect arbitrage and other MEV opportunities as they happen on the exchange.

## Requirements

### Requirement 1: WebSocket Connection Management

**User Story:** As a bot operator, I want the system to maintain a stable connection to HyperLiquid's WebSocket API, so that I can receive continuous real-time trade data without interruptions.

#### Acceptance Criteria

1. WHEN the bot starts THEN the system SHALL establish a WebSocket connection to `wss://api.hyperliquid.xyz/ws`
2. WHEN the WebSocket connection is established THEN the system SHALL log a successful connection message
3. IF the WebSocket connection drops THEN the system SHALL automatically attempt to reconnect with exponential backoff
4. WHEN reconnecting THEN the system SHALL wait a minimum of 1 second and maximum of 60 seconds between attempts
5. WHEN the connection is stable for 5 minutes THEN the system SHALL reset the reconnection backoff timer
6. WHEN the bot shuts down THEN the system SHALL gracefully close the WebSocket connection

### Requirement 2: Trade Data Subscription

**User Story:** As a bot operator, I want to subscribe to real-time trade feeds for specific trading pairs, so that I can monitor market activity and detect arbitrage opportunities.

#### Acceptance Criteria

1. WHEN the WebSocket connection is established THEN the system SHALL send subscription messages for configured trading pairs
2. WHEN subscribing to a trading pair THEN the system SHALL use the HyperLiquid subscription format: `{"method": "subscribe", "subscription": {"type": "trades", "coin": "<SYMBOL>"}}`
3. WHEN a subscription is successful THEN the system SHALL receive a `subscriptionResponse` confirmation message
4. IF a subscription fails THEN the system SHALL log an error and retry the subscription after 5 seconds
5. WHEN the system receives trade data THEN the system SHALL parse the message and extract trade details (coin, price, size, timestamp, side)
6. WHEN multiple trading pairs are configured THEN the system SHALL subscribe to all pairs sequentially

### Requirement 3: Real-Time Trade Processing

**User Story:** As a bot operator, I want incoming trade data to be processed in real-time and fed into the strategy engine, so that arbitrage opportunities can be detected immediately.

#### Acceptance Criteria

1. WHEN a trade message is received THEN the system SHALL parse it within 10 milliseconds
2. WHEN trade data is parsed THEN the system SHALL convert it into a standardized internal format compatible with existing strategies
3. WHEN trade data is converted THEN the system SHALL send it to the strategy engine for opportunity evaluation
4. IF parsing fails THEN the system SHALL log the error with the raw message and continue processing other messages
5. WHEN processing trade data THEN the system SHALL maintain message ordering to ensure accurate price tracking
6. WHEN the message queue exceeds 1000 pending messages THEN the system SHALL apply backpressure and drop oldest messages

### Requirement 4: Order Book Subscription (Optional)

**User Story:** As a bot operator, I want to optionally subscribe to order book updates, so that I can detect larger arbitrage opportunities from order book imbalances.

#### Acceptance Criteria

1. WHEN order book monitoring is enabled in config THEN the system SHALL subscribe to order book updates using `{"method": "subscribe", "subscription": {"type": "l2Book", "coin": "<SYMBOL>"}}`
2. WHEN order book data is received THEN the system SHALL update an in-memory order book representation
3. WHEN the order book is updated THEN the system SHALL calculate the best bid and ask prices
4. WHEN significant order book imbalances are detected (>10% spread) THEN the system SHALL notify the strategy engine
5. IF order book monitoring is disabled THEN the system SHALL only subscribe to trade feeds

### Requirement 5: Configuration Integration

**User Story:** As a bot operator, I want to configure HyperLiquid WebSocket settings in the existing config file, so that I can control which pairs to monitor and connection parameters.

#### Acceptance Criteria

1. WHEN the config file is loaded THEN the system SHALL read HyperLiquid-specific settings from a new `hyperliquid` section
2. WHEN HyperLiquid settings are present THEN the system SHALL use the native WebSocket API instead of EVM mempool polling
3. WHEN trading pairs are configured THEN the system SHALL accept a list of coin symbols (e.g., ["BTC", "ETH", "SOL"])
4. WHEN no trading pairs are configured THEN the system SHALL default to monitoring ["BTC", "ETH"]
5. WHEN the config specifies `use_hyperliquid_api: true` THEN the system SHALL disable EVM mempool monitoring
6. WHEN the config is invalid THEN the system SHALL log an error and fail to start with a clear error message

### Requirement 6: Error Handling and Resilience

**User Story:** As a bot operator, I want the system to handle errors gracefully and continue operating, so that temporary issues don't cause the bot to crash.

#### Acceptance Criteria

1. WHEN a malformed message is received THEN the system SHALL log the error and continue processing
2. WHEN the WebSocket receives an error message from HyperLiquid THEN the system SHALL log it with ERROR level
3. IF the connection fails 10 times consecutively THEN the system SHALL enter a degraded state and alert the operator
4. WHEN in degraded state THEN the system SHALL continue attempting reconnection but reduce retry frequency to once per 5 minutes
5. WHEN network connectivity is lost THEN the system SHALL detect it within 30 seconds and begin reconnection attempts
6. WHEN the system recovers from errors THEN the system SHALL re-subscribe to all configured trading pairs

### Requirement 7: Metrics and Monitoring

**User Story:** As a bot operator, I want to monitor the health and performance of the HyperLiquid WebSocket connection, so that I can ensure the bot is receiving data properly.

#### Acceptance Criteria

1. WHEN the WebSocket is connected THEN the system SHALL expose a Prometheus metric `hyperliquid_ws_connected` with value 1
2. WHEN the WebSocket is disconnected THEN the system SHALL set `hyperliquid_ws_connected` to 0
3. WHEN trade messages are received THEN the system SHALL increment a counter `hyperliquid_trades_received_total`
4. WHEN messages are processed THEN the system SHALL track processing latency in a histogram `hyperliquid_message_processing_duration_seconds`
5. WHEN subscriptions are active THEN the system SHALL expose a gauge `hyperliquid_active_subscriptions` showing the count
6. WHEN reconnection attempts occur THEN the system SHALL increment `hyperliquid_reconnection_attempts_total`

### Requirement 8: Strategy Engine Integration

**User Story:** As a bot operator, I want HyperLiquid trade data to work seamlessly with existing arbitrage strategies, so that I don't need to rewrite strategy logic.

#### Acceptance Criteria

1. WHEN trade data is received THEN the system SHALL convert it to a format compatible with the existing `ParsedTransaction` structure
2. WHEN converting trade data THEN the system SHALL map HyperLiquid trades to equivalent DEX swap events
3. WHEN the strategy engine receives trade data THEN it SHALL evaluate it using existing arbitrage detection logic
4. WHEN an opportunity is detected THEN the system SHALL use existing bundle building and submission logic
5. IF the trade data format is incompatible THEN the system SHALL create an adapter layer to bridge the formats
6. WHEN multiple data sources are active THEN the system SHALL merge opportunities from both HyperLiquid and EVM sources
