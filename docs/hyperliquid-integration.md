# HyperLiquid Integration Guide

This guide covers the integration of HyperLiquid's native WebSocket API into the MEV bot for real-time trade monitoring and cross-exchange arbitrage opportunities.

## Table of Contents

- [Overview](#overview)
- [Configuration](#configuration)
- [Token Mapping](#token-mapping)
- [Monitoring and Metrics](#monitoring-and-metrics)
- [Troubleshooting](#troubleshooting)
- [Best Practices](#best-practices)

## Overview

The HyperLiquid integration provides real-time access to trade data from HyperLiquid's native exchange through their WebSocket API. This enables the bot to:

- Monitor trades across multiple trading pairs in real-time
- Detect arbitrage opportunities between HyperLiquid and EVM-based DEXes
- Subscribe to order book updates for deeper market analysis
- Automatically handle connection failures with exponential backoff
- Track comprehensive metrics for monitoring and alerting

### Architecture

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

### Data Flow

1. **Connection**: Service establishes WebSocket connection to `wss://api.hyperliquid.xyz/ws`
2. **Subscription**: Sends subscription messages for configured trading pairs
3. **Trade Reception**: Receives real-time trade data from HyperLiquid
4. **Parsing**: TradeMessageParser validates and structures the data
5. **Adaptation**: TradeDataAdapter converts trades to internal format
6. **Strategy Evaluation**: Strategy engine evaluates for arbitrage opportunities
7. **Execution**: Profitable opportunities are executed via bundle submission

## Configuration

### Basic Configuration

Add the `hyperliquid` section to your `config/my-config.yml`:

```yaml
hyperliquid:
  # Enable/disable the integration
  enabled: true
  
  # WebSocket endpoint (default: wss://api.hyperliquid.xyz/ws)
  ws_url: "wss://api.hyperliquid.xyz/ws"
  
  # Trading pairs to monitor
  trading_pairs:
    - "BTC"
    - "ETH"
    - "SOL"
    - "ARB"
  
  # Order book subscription (optional)
  subscribe_orderbook: false
  
  # Reconnection settings
  reconnect_min_backoff_secs: 1
  reconnect_max_backoff_secs: 60
  max_consecutive_failures: 10
  
  # Token mapping (see Token Mapping section)
  token_mapping:
    BTC: "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"
    ETH: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
```

### Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | boolean | `false` | Enable/disable HyperLiquid integration |
| `ws_url` | string | `wss://api.hyperliquid.xyz/ws` | WebSocket endpoint URL |
| `trading_pairs` | array | `["BTC", "ETH"]` | List of coin symbols to monitor |
| `subscribe_orderbook` | boolean | `false` | Subscribe to L2 order book updates |
| `reconnect_min_backoff_secs` | integer | `1` | Minimum reconnection backoff (seconds) |
| `reconnect_max_backoff_secs` | integer | `60` | Maximum reconnection backoff (seconds) |
| `max_consecutive_failures` | integer | `10` | Max failures before degraded state |
| `token_mapping` | object | `{}` | Map of coin symbols to EVM addresses |

### Trading Pairs

HyperLiquid supports various trading pairs. Common symbols include:

- **Major Cryptocurrencies**: BTC, ETH, SOL, BNB, XRP, ADA, DOT, AVAX
- **DeFi Tokens**: UNI, AAVE, LINK, CRV, SNX, COMP
- **Layer 2s**: ARB, OP, MATIC
- **Meme Coins**: DOGE, SHIB, PEPE

Check [HyperLiquid's documentation](https://hyperliquid.xyz/docs) for the complete list of supported pairs.

### Order Book Subscription

Enable order book monitoring for deeper market analysis:

```yaml
hyperliquid:
  subscribe_orderbook: true
```

**Benefits:**
- Access to L2 order book data (bids and asks)
- Calculate best bid/ask prices
- Detect significant spread opportunities (>10%)
- Identify order book imbalances

**Trade-offs:**
- Higher data volume
- Increased processing overhead
- More network bandwidth usage

**Recommendation**: Enable only if your strategy requires order book data.

## Token Mapping

Token mapping enables cross-exchange arbitrage by mapping HyperLiquid coin symbols to EVM token addresses.

### Purpose

When a trade occurs on HyperLiquid (e.g., BTC/USDC), the bot needs to know the corresponding EVM token address (e.g., WBTC) to:
- Compare prices with EVM-based DEXes
- Calculate arbitrage opportunities
- Execute cross-exchange trades

### Configuration

```yaml
hyperliquid:
  token_mapping:
    # Format: COIN_SYMBOL: "EVM_TOKEN_ADDRESS"
    BTC: "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"  # WBTC on Ethereum
    ETH: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"  # WETH on Ethereum
    SOL: "0x5288738df1aB05A68337cB9dD7a607285Ac3Cf90"  # SOL (example)
    ARB: "0x912CE59144191C1204E64559FE8253a0e49E6548"  # ARB token
    USDC: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48" # USDC
```

### Common Token Addresses

**Ethereum Mainnet:**
```yaml
token_mapping:
  BTC: "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"  # WBTC
  ETH: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"  # WETH
  USDC: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48" # USDC
  USDT: "0xdAC17F958D2ee523a2206206994597C13D831ec7" # USDT
  DAI: "0x6B175474E89094C44Da98b954EedeAC495271d0F"  # DAI
  UNI: "0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984"  # UNI
  LINK: "0x514910771AF9Ca656af840dff83E8264EcF986CA" # LINK
  AAVE: "0x7Fc66500c84A76Ad7e9c93437bFc5Ac33E2DDaE9" # AAVE
```

**Arbitrum:**
```yaml
token_mapping:
  BTC: "0x2f2a2543B76A4166549F7aaB2e75Bef0aefC5B0f"  # WBTC
  ETH: "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1"  # WETH
  USDC: "0xFF970A61A04b1cA14834A43f5dE4533eBDDB5CC8" # USDC
  ARB: "0x912CE59144191C1204E64559FE8253a0e49E6548"  # ARB
```

### Validation

The bot validates token addresses on startup:
- Checks address format (0x + 40 hex characters)
- Logs warnings for missing mappings
- Continues operation with available mappings

## Monitoring and Metrics

### Prometheus Metrics

All metrics are exposed on `:9090/metrics`:

#### Connection Metrics

```
# Connection status (0 = disconnected, 1 = connected)
hyperliquid_ws_connected{url="wss://api.hyperliquid.xyz/ws"} 1

# Total reconnection attempts
hyperliquid_reconnection_attempts_total{reason="connection_lost"} 5
hyperliquid_reconnection_attempts_total{reason="degraded_state"} 0

# Degraded state indicator
hyperliquid_degraded_state{reason="max_failures"} 0
```

#### Trade Metrics

```
# Total trades received
hyperliquid_trades_received_total{coin="BTC",side="buy"} 1234
hyperliquid_trades_received_total{coin="BTC",side="sell"} 987
hyperliquid_trades_received_total{coin="ETH",side="buy"} 2345

# Message processing latency (histogram)
hyperliquid_message_processing_duration_seconds_bucket{message_type="trade",le="0.001"} 950
hyperliquid_message_processing_duration_seconds_bucket{message_type="trade",le="0.01"} 1200
hyperliquid_message_processing_duration_seconds_sum{message_type="trade"} 1.234
hyperliquid_message_processing_duration_seconds_count{message_type="trade"} 1234
```

#### Subscription Metrics

```
# Active subscriptions
hyperliquid_active_subscriptions{type="all"} 4

# Subscription errors
hyperliquid_subscription_errors_total{coin="BTC",type="trades"} 0
hyperliquid_subscription_errors_total{coin="INVALID",type="trades"} 3
```

#### Error Metrics

```
# Connection errors
hyperliquid_connection_errors_total{type="connection_failed"} 2
hyperliquid_connection_errors_total{type="timeout"} 1

# Parse errors
hyperliquid_parse_errors_total{coin="BTC"} 0
hyperliquid_parse_errors_total{coin="unknown"} 5

# Adaptation errors
hyperliquid_adaptation_errors_total{coin="UNKNOWN",reason="unknown_coin"} 3
hyperliquid_adaptation_errors_total{coin="USDC",reason="missing_usdc_mapping"} 0

# Network errors
hyperliquid_network_errors_total{type="stream_error"} 1
hyperliquid_network_errors_total{type="ping_failed"} 0
```

### Grafana Dashboards

Create a dashboard with the following panels:

#### Connection Health
```promql
# Connection status
hyperliquid_ws_connected

# Reconnection rate (per minute)
rate(hyperliquid_reconnection_attempts_total[1m])

# Time since last reconnection
time() - hyperliquid_reconnection_attempts_total
```

#### Trade Volume
```promql
# Trades per second by coin
rate(hyperliquid_trades_received_total[1m])

# Total trades by coin
sum by (coin) (hyperliquid_trades_received_total)

# Buy/sell ratio
sum by (coin) (hyperliquid_trades_received_total{side="buy"}) / 
sum by (coin) (hyperliquid_trades_received_total{side="sell"})
```

#### Latency Analysis
```promql
# P50 processing latency
histogram_quantile(0.5, rate(hyperliquid_message_processing_duration_seconds_bucket[5m]))

# P95 processing latency
histogram_quantile(0.95, rate(hyperliquid_message_processing_duration_seconds_bucket[5m]))

# P99 processing latency
histogram_quantile(0.99, rate(hyperliquid_message_processing_duration_seconds_bucket[5m]))
```

#### Error Rates
```promql
# Parse error rate
rate(hyperliquid_parse_errors_total[5m])

# Subscription error rate
rate(hyperliquid_subscription_errors_total[5m])

# Network error rate
rate(hyperliquid_network_errors_total[5m])
```

### Alerting Rules

Configure alerts for critical conditions:

```yaml
groups:
  - name: hyperliquid_alerts
    rules:
      # Connection down for > 5 minutes
      - alert: HyperLiquidDisconnected
        expr: hyperliquid_ws_connected == 0
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "HyperLiquid WebSocket disconnected"
          description: "Connection has been down for more than 5 minutes"
      
      # Degraded state active
      - alert: HyperLiquidDegraded
        expr: hyperliquid_degraded_state > 0
        for: 1m
        labels:
          severity: warning
        annotations:
          summary: "HyperLiquid service in degraded state"
          description: "Service has entered degraded state due to repeated failures"
      
      # High parse error rate
      - alert: HyperLiquidHighParseErrors
        expr: rate(hyperliquid_parse_errors_total[5m]) > 0.01
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High parse error rate"
          description: "Parse error rate exceeds 1% over 5 minutes"
      
      # High processing latency
      - alert: HyperLiquidHighLatency
        expr: histogram_quantile(0.99, rate(hyperliquid_message_processing_duration_seconds_bucket[5m])) > 0.1
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High message processing latency"
          description: "P99 latency exceeds 100ms"
      
      # No trades received
      - alert: HyperLiquidNoTrades
        expr: rate(hyperliquid_trades_received_total[5m]) == 0
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "No trades received"
          description: "No trades received for active pairs in 5 minutes"
```

## Troubleshooting

### Connection Issues

**Symptom**: `hyperliquid_ws_connected` = 0

**Diagnosis**:
```bash
# Check WebSocket connectivity
wscat -c wss://api.hyperliquid.xyz/ws

# Check DNS resolution
nslookup api.hyperliquid.xyz

# Check network connectivity
ping api.hyperliquid.xyz

# Review logs
docker logs mev-bot | grep "hyperliquid" | grep "ERROR"
```

**Solutions**:
- Verify network connectivity
- Check firewall rules
- Ensure WebSocket port (443) is not blocked
- Verify HyperLiquid API is operational

### No Trades Received

**Symptom**: `hyperliquid_trades_received_total` not increasing

**Diagnosis**:
```bash
# Check active subscriptions
curl http://localhost:9090/metrics | grep hyperliquid_active_subscriptions

# Check subscription errors
curl http://localhost:9090/metrics | grep hyperliquid_subscription_errors_total

# Review logs for subscription confirmations
docker logs mev-bot | grep "Subscription confirmed"
```

**Solutions**:
- Verify trading pairs are valid HyperLiquid symbols
- Check `enabled: true` in configuration
- Review subscription error metrics
- Ensure subscriptions are confirmed

### High Parse Errors

**Symptom**: `hyperliquid_parse_errors_total` increasing

**Diagnosis**:
```bash
# Check parse error rate
curl http://localhost:9090/metrics | grep hyperliquid_parse_errors_total

# Review logs for parse errors
docker logs mev-bot | grep "Failed to parse message"
```

**Solutions**:
- Check if HyperLiquid API format has changed
- Review raw messages in logs
- Update parser if needed
- Report issue if API format is unexpected

### Degraded State

**Symptom**: `hyperliquid_degraded_state` = 1

**Diagnosis**:
```bash
# Check reconnection attempts
curl http://localhost:9090/metrics | grep hyperliquid_reconnection_attempts_total

# Check consecutive failures
docker logs mev-bot | grep "degraded state"
```

**Solutions**:
- Service will automatically recover when connection is restored
- Check network stability
- Verify HyperLiquid API availability
- Review `max_consecutive_failures` setting
- Monitor for automatic recovery

### High Latency

**Symptom**: Processing latency > 10ms

**Diagnosis**:
```bash
# Check latency histogram
curl http://localhost:9090/metrics | grep hyperliquid_message_processing_duration_seconds

# Check system resources
htop
iostat -x 1
```

**Solutions**:
- Check CPU usage
- Review system load
- Optimize strategy evaluation
- Consider scaling horizontally
- Reduce number of trading pairs if needed

### Adaptation Errors

**Symptom**: `hyperliquid_adaptation_errors_total` increasing

**Diagnosis**:
```bash
# Check adaptation errors
curl http://localhost:9090/metrics | grep hyperliquid_adaptation_errors_total

# Review logs
docker logs mev-bot | grep "adaptation error"
```

**Solutions**:
- Verify token mapping is complete
- Add missing coin mappings
- Check for unknown coins in trading pairs
- Ensure USDC is mapped (required as quote currency)

## Best Practices

### Configuration

1. **Start Small**: Begin with 1-2 trading pairs, expand gradually
2. **Monitor First**: Run in monitoring mode before enabling strategies
3. **Test Token Mapping**: Verify all coins have valid EVM addresses
4. **Set Appropriate Backoff**: Balance reconnection speed vs. API load

### Monitoring

1. **Set Up Alerts**: Configure alerts for critical metrics
2. **Monitor Latency**: Track P95/P99 processing latency
3. **Watch Error Rates**: Alert on parse/adaptation errors
4. **Track Subscriptions**: Ensure all pairs are subscribed

### Performance

1. **Disable Order Books**: Unless needed for strategy
2. **Limit Trading Pairs**: More pairs = more processing overhead
3. **Optimize Strategies**: Minimize processing time per trade
4. **Use Backpressure**: Let the queue drop old messages if needed

### Security

1. **Use WSS**: Always use secure WebSocket (wss://)
2. **Validate Data**: Parser validates all incoming data
3. **Handle Errors**: All errors are logged and tracked
4. **Monitor Anomalies**: Alert on unusual patterns

### Operational

1. **Review Logs**: Regularly check for warnings/errors
2. **Update Mappings**: Keep token addresses current
3. **Test Reconnection**: Verify automatic recovery works
4. **Plan for Degraded State**: Understand behavior when degraded

---

For additional support, see:
- [Main README](../README.md)
- [Configuration Guide](configuration.md)
- [Monitoring Setup](monitoring.md)
- [Troubleshooting Guide](troubleshooting.md)
