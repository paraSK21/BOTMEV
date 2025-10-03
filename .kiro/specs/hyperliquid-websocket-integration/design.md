# Design Document

## Overview

This design integrates HyperLiquid's native WebSocket API into the existing MEV bot architecture. The integration creates a new data source that streams real-time trade data from HyperLiquid's exchange, processes it into a format compatible with existing strategies, and feeds it into the strategy engine for opportunity detection.

The key insight is that HyperLiquid trades can be treated similarly to DEX swaps - they represent price movements and liquidity changes that create arbitrage opportunities. By monitoring trades in real-time via WebSocket, the bot can detect these opportunities faster than polling confirmed blocks.

## Architecture

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         MEV Bot                              │
│                                                              │
│  ┌────────────────┐         ┌──────────────────┐           │
│  │  Config Loader │────────▶│  Main Controller │           │
│  └────────────────┘         └──────────────────┘           │
│                                      │                       │
│                    ┌─────────────────┼─────────────────┐    │
│                    │                 │                 │    │
│                    ▼                 ▼                 ▼    │
│         ┌──────────────────┐  ┌──────────────┐  ┌────────┐│
│         │ HyperLiquid WS   │  │  EVM Mempool │  │Strategy││
│         │    Service       │  │   Service    │  │ Engine ││
│         └──────────────────┘  └──────────────┘  └────────┘│
│                │                      │                │    │
│                │                      │                │    │
│                └──────────────────────┴────────────────┘    │
│                                │                             │
│                                ▼                             │
│                    ┌──────────────────────┐                 │
│                    │  Trade Data Adapter  │                 │
│                    └──────────────────────┘                 │
│                                │                             │
│                                ▼                             │
│                    ┌──────────────────────┐                 │
│                    │   Strategy Engine    │                 │
│                    │  (Arbitrage, etc.)   │                 │
│                    └──────────────────────┘                 │
└─────────────────────────────────────────────────────────────┘
```

### Component Interaction Flow

```
1. Bot starts → Config Loader reads hyperliquid settings
2. Main Controller creates HyperLiquidWsService
3. HyperLiquidWsService connects to wss://api.hyperliquid.xyz/ws
4. Service subscribes to trade feeds for configured coins
5. Trade messages arrive → parsed by TradeMessageParser
6. Parsed trades → converted by TradeDataAdapter
7. Adapted data → sent to StrategyEngine
8. StrategyEngine evaluates for opportunities
9. Opportunities → Bundle creation and submission
```

## Components and Interfaces

### 1. HyperLiquidWsService

**Purpose:** Manages WebSocket connection to HyperLiquid API and handles subscriptions.

**Structure:**
```rust
pub struct HyperLiquidWsService {
    ws_url: String,
    subscriptions: Vec<TradingPair>,
    connection: Arc<Mutex<Option<WebSocketStream>>>,
    reconnect_config: ReconnectConfig,
    message_tx: mpsc::UnboundedSender<HyperLiquidMessage>,
    metrics: Arc<PrometheusMetrics>,
    shutdown: Arc<AtomicBool>,
}

pub struct ReconnectConfig {
    min_backoff_secs: u64,
    max_backoff_secs: u64,
    max_consecutive_failures: u32,
}

pub struct TradingPair {
    coin: String,
    subscribe_trades: bool,
    subscribe_orderbook: bool,
}
```

**Key Methods:**
- `new(config: HyperLiquidConfig) -> Result<Self>`
- `async fn start(&self) -> Result<()>` - Main connection loop
- `async fn connect(&self) -> Result<WebSocketStream>` - Establish connection
- `async fn subscribe(&self, pair: &TradingPair) -> Result<()>` - Send subscription
- `async fn handle_message(&self, msg: Message) -> Result<()>` - Process incoming messages
- `async fn reconnect(&self) -> Result<()>` - Reconnection logic with backoff

**Responsibilities:**
- Establish and maintain WebSocket connection
- Send subscription messages on connect/reconnect
- Parse incoming WebSocket messages
- Forward parsed messages to message channel
- Handle connection errors and reconnection
- Expose connection health metrics

### 2. TradeMessageParser

**Purpose:** Parses HyperLiquid's custom message format into structured data.

**Structure:**
```rust
pub struct TradeMessageParser {
    // Stateless parser
}

#[derive(Debug, Clone)]
pub struct HyperLiquidMessage {
    channel: String,
    data: MessageData,
}

#[derive(Debug, Clone)]
pub enum MessageData {
    SubscriptionResponse(SubscriptionResponse),
    Trade(TradeData),
    OrderBook(OrderBookData),
    Error(ErrorData),
}

#[derive(Debug, Clone)]
pub struct TradeData {
    coin: String,
    side: TradeSide,
    price: f64,
    size: f64,
    timestamp: u64,
    trade_id: String,
}

#[derive(Debug, Clone)]
pub enum TradeSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub struct OrderBookData {
    coin: String,
    bids: Vec<OrderBookLevel>,
    asks: Vec<OrderBookLevel>,
    timestamp: u64,
}

#[derive(Debug, Clone)]
pub struct OrderBookLevel {
    price: f64,
    size: f64,
}
```

**Key Methods:**
- `fn parse_message(raw: &str) -> Result<HyperLiquidMessage>`
- `fn parse_trade(data: &Value) -> Result<TradeData>`
- `fn parse_orderbook(data: &Value) -> Result<OrderBookData>`
- `fn validate_message(msg: &HyperLiquidMessage) -> Result<()>`

**Responsibilities:**
- Parse JSON messages from WebSocket
- Validate message structure
- Extract trade and order book data
- Handle malformed messages gracefully

### 3. TradeDataAdapter

**Purpose:** Converts HyperLiquid trade data into format compatible with existing strategy engine.

**Structure:**
```rust
pub struct TradeDataAdapter {
    chain_id: u64,
    // Maps HyperLiquid coins to EVM token addresses for cross-exchange arbitrage
    token_mapping: HashMap<String, Address>,
}

#[derive(Debug, Clone)]
pub struct AdaptedTradeEvent {
    // Compatible with ParsedTransaction structure
    pub hash: String,
    pub from: Address,
    pub to: Address,
    pub value: U256,
    pub gas_price: U256,
    pub timestamp: u64,
    pub swap_data: Option<SwapData>,
}

#[derive(Debug, Clone)]
pub struct SwapData {
    pub dex: String,
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: U256,
    pub amount_out: U256,
    pub price: f64,
}
```

**Key Methods:**
- `fn new(chain_id: u64, token_mapping: HashMap<String, Address>) -> Self`
- `fn adapt_trade(&self, trade: &TradeData) -> Result<AdaptedTradeEvent>`
- `fn calculate_swap_amounts(&self, trade: &TradeData) -> (U256, U256)`
- `fn map_coin_to_address(&self, coin: &str) -> Option<Address>`

**Responsibilities:**
- Convert HyperLiquid trades to ParsedTransaction-like format
- Map coin symbols to EVM addresses
- Calculate equivalent swap amounts
- Maintain compatibility with existing strategy engine

### 4. HyperLiquidConfig

**Purpose:** Configuration structure for HyperLiquid integration.

**Structure:**
```rust
#[derive(Debug, Clone, Deserialize)]
pub struct HyperLiquidConfig {
    pub enabled: bool,
    pub ws_url: String,
    pub trading_pairs: Vec<String>,
    pub subscribe_orderbook: bool,
    pub reconnect_min_backoff_secs: u64,
    pub reconnect_max_backoff_secs: u64,
    pub max_consecutive_failures: u32,
    pub token_mapping: HashMap<String, String>, // coin -> address
}

impl Default for HyperLiquidConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ws_url: "wss://api.hyperliquid.xyz/ws".to_string(),
            trading_pairs: vec!["BTC".to_string(), "ETH".to_string()],
            subscribe_orderbook: false,
            reconnect_min_backoff_secs: 1,
            reconnect_max_backoff_secs: 60,
            max_consecutive_failures: 10,
            token_mapping: HashMap::new(),
        }
    }
}
```

### 5. Integration with Main Bot

**Modified main.rs structure:**
```rust
async fn main() -> Result<()> {
    // Load config
    let config = load_config()?;
    
    // Initialize metrics
    let metrics = Arc::new(PrometheusMetrics::new()?);
    
    // Initialize strategy engine
    let strategy_engine = Arc::new(StrategyEngine::new(config.strategy_config, metrics.clone()));
    
    // Initialize data sources based on config
    let mut data_sources: Vec<Box<dyn DataSource>> = Vec::new();
    
    // HyperLiquid WebSocket (if enabled)
    if config.hyperliquid.enabled {
        let hl_service = HyperLiquidWsService::new(config.hyperliquid.clone())?;
        let adapter = TradeDataAdapter::new(config.chain_id, config.hyperliquid.token_mapping);
        data_sources.push(Box::new(HyperLiquidDataSource::new(hl_service, adapter)));
    }
    
    // EVM Mempool (if enabled)
    if config.mempool.enabled {
        let mempool_service = MempoolService::new(config.mempool.endpoint, config.mempool.rpc_url)?;
        data_sources.push(Box::new(MempoolDataSource::new(mempool_service)));
    }
    
    // Start all data sources
    for source in &data_sources {
        source.start(strategy_engine.clone()).await?;
    }
    
    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    
    Ok(())
}
```

## Data Models

### Message Flow Data Structures

```rust
// Raw WebSocket message from HyperLiquid
{
  "channel": "trades",
  "data": {
    "coin": "BTC",
    "side": "B",  // B = Buy, A = Ask/Sell
    "px": "45000.5",
    "sz": "0.5",
    "time": 1696348800000,
    "tid": "12345"
  }
}

// Parsed into TradeData
TradeData {
    coin: "BTC",
    side: TradeSide::Buy,
    price: 45000.5,
    size: 0.5,
    timestamp: 1696348800000,
    trade_id: "12345",
}

// Adapted into AdaptedTradeEvent
AdaptedTradeEvent {
    hash: "hl_trade_12345",
    from: Address::from("0x..."), // HyperLiquid exchange address
    to: Address::from("0x..."),   // Buyer address (synthetic)
    value: U256::zero(),
    gas_price: U256::zero(),
    timestamp: 1696348800000,
    swap_data: Some(SwapData {
        dex: "HyperLiquid",
        token_in: Address::from("0x..."), // USDC
        token_out: Address::from("0x..."), // WBTC
        amount_in: U256::from(22500250000000u64), // 22500.25 USDC (6 decimals)
        amount_out: U256::from(50000000000000000u64), // 0.5 BTC (18 decimals)
        price: 45000.5,
    }),
}
```

### Configuration File Structure

```yaml
# HyperLiquid Network Configuration
hyperliquid:
  enabled: true
  ws_url: "wss://api.hyperliquid.xyz/ws"
  
  # Trading pairs to monitor
  trading_pairs:
    - "BTC"
    - "ETH"
    - "SOL"
    - "ARB"
  
  # Subscribe to order book updates (optional, more data)
  subscribe_orderbook: false
  
  # Reconnection settings
  reconnect_min_backoff_secs: 1
  reconnect_max_backoff_secs: 60
  max_consecutive_failures: 10
  
  # Map HyperLiquid coins to EVM token addresses for arbitrage
  token_mapping:
    BTC: "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"  # WBTC
    ETH: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"  # WETH
    SOL: "0x5288738df1aB05A68337cB9dD7a607285Ac3Cf90"  # SOL (example)
    ARB: "0x912CE59144191C1204E64559FE8253a0e49E6548"  # ARB

# EVM Mempool (can run alongside HyperLiquid)
mempool:
  enabled: false  # Disable since HyperLiquid doesn't support it
  rpc_url: "https://rpc.hyperliquid.xyz/evm"
  ws_url: "wss://rpc.hyperliquid.xyz/evm"
```

## Error Handling

### Error Categories and Responses

1. **Connection Errors**
   - WebSocket connection fails
   - Response: Log error, trigger reconnection with exponential backoff
   - Metric: Increment `hyperliquid_connection_errors_total`

2. **Subscription Errors**
   - Subscription message rejected
   - Response: Log error, retry after 5 seconds, max 3 retries
   - Metric: Increment `hyperliquid_subscription_errors_total`

3. **Parse Errors**
   - Malformed JSON message
   - Response: Log error with raw message, continue processing
   - Metric: Increment `hyperliquid_parse_errors_total`

4. **Adaptation Errors**
   - Cannot convert trade to internal format
   - Response: Log error, skip trade, continue processing
   - Metric: Increment `hyperliquid_adaptation_errors_total`

5. **Network Errors**
   - Timeout, connection reset
   - Response: Close connection, trigger reconnection
   - Metric: Increment `hyperliquid_network_errors_total`

### Degraded State Handling

When `max_consecutive_failures` is reached:
1. Enter degraded state
2. Set `hyperliquid_degraded_state` metric to 1
3. Reduce reconnection frequency to once per 5 minutes
4. Log WARN level message every 30 minutes
5. Continue attempting reconnection indefinitely
6. Exit degraded state on successful connection

## Testing Strategy

### Unit Tests

1. **TradeMessageParser Tests**
   - Test parsing valid trade messages
   - Test parsing order book messages
   - Test handling malformed JSON
   - Test handling missing fields
   - Test handling unexpected message types

2. **TradeDataAdapter Tests**
   - Test trade to swap conversion
   - Test coin to address mapping
   - Test amount calculations
   - Test handling unknown coins

3. **ReconnectConfig Tests**
   - Test backoff calculation
   - Test max backoff limit
   - Test backoff reset after success

### Integration Tests

1. **WebSocket Connection Test**
   - Mock HyperLiquid WebSocket server
   - Test connection establishment
   - Test subscription flow
   - Test message reception
   - Test reconnection on disconnect

2. **End-to-End Flow Test**
   - Mock WebSocket with sample trade data
   - Verify parsing → adaptation → strategy engine flow
   - Verify opportunity detection from trades
   - Verify metrics are updated correctly

3. **Error Handling Test**
   - Test connection failure scenarios
   - Test malformed message handling
   - Test degraded state entry/exit
   - Verify no crashes on errors

### Manual Testing

1. **Live Connection Test**
   - Connect to real HyperLiquid WebSocket
   - Subscribe to BTC and ETH trades
   - Verify trade data is received
   - Monitor for 1 hour, check stability

2. **Reconnection Test**
   - Start bot, verify connection
   - Kill network connection
   - Verify reconnection attempts
   - Restore network, verify recovery

3. **Multi-Pair Test**
   - Subscribe to 5+ trading pairs
   - Verify all subscriptions succeed
   - Verify trades from all pairs are processed
   - Check for message ordering issues

## Performance Considerations

### Latency Targets

- WebSocket message receipt to parse: < 1ms
- Parse to adaptation: < 5ms
- Adaptation to strategy engine: < 10ms
- Total end-to-end: < 20ms

### Throughput Targets

- Handle 1000 trades/second across all pairs
- Process order book updates at 100 updates/second
- Maintain < 100ms p99 latency under load

### Resource Usage

- Memory: < 100MB for WebSocket service
- CPU: < 5% for message processing
- Network: Minimal (WebSocket is efficient)

### Optimization Strategies

1. **Zero-Copy Parsing**: Use serde_json streaming parser
2. **Message Batching**: Process multiple trades in single strategy evaluation
3. **Async Processing**: All I/O operations are async
4. **Connection Pooling**: Reuse WebSocket connection
5. **Backpressure**: Drop old messages if processing falls behind

## Security Considerations

1. **WebSocket Security**
   - Use WSS (secure WebSocket) for all connections
   - Validate TLS certificates
   - No authentication required (public API)

2. **Data Validation**
   - Validate all incoming message fields
   - Sanitize coin symbols before use
   - Bounds check all numeric values
   - Reject messages with invalid timestamps

3. **Resource Limits**
   - Limit message queue size (1000 messages)
   - Limit reconnection attempts (10 consecutive)
   - Timeout on subscription responses (5 seconds)
   - Rate limit subscription requests

4. **Error Information Disclosure**
   - Don't log sensitive data in errors
   - Sanitize error messages before logging
   - Use structured logging for security events

## Deployment Considerations

### Configuration Management

- Store token mappings in config file
- Allow runtime updates to trading pairs (future)
- Validate config on startup
- Provide clear error messages for invalid config

### Monitoring and Alerting

**Key Metrics to Monitor:**
- `hyperliquid_ws_connected`: Connection status (0/1)
- `hyperliquid_trades_received_total`: Trade message count
- `hyperliquid_message_processing_duration_seconds`: Processing latency
- `hyperliquid_reconnection_attempts_total`: Reconnection frequency
- `hyperliquid_degraded_state`: Degraded state indicator (0/1)

**Alert Conditions:**
- Connection down for > 5 minutes
- Degraded state active
- Parse error rate > 1%
- Processing latency p99 > 100ms
- No trades received for > 60 seconds (for active pairs)

### Rollout Strategy

1. **Phase 1**: Deploy with `enabled: false`, test configuration
2. **Phase 2**: Enable for single pair (BTC), monitor for 24 hours
3. **Phase 3**: Add ETH, monitor for 24 hours
4. **Phase 4**: Add remaining pairs, full production
5. **Rollback**: Set `enabled: false` if issues occur

## Future Enhancements

1. **Order Book Arbitrage**: Use order book data for larger opportunities
2. **Multi-Exchange Arbitrage**: Compare HyperLiquid prices with other DEXes
3. **Historical Data**: Store trade history for backtesting
4. **Dynamic Pair Selection**: Auto-subscribe to high-volume pairs
5. **Advanced Filtering**: Filter trades by size, price movement
6. **Liquidation Monitoring**: Subscribe to liquidation events
7. **Funding Rate Arbitrage**: Monitor funding rates for perp arbitrage
