# MEV Bot - High-Performance Ethereum MEV Detection and Execution

[![CI/CD Pipeline](https://github.com/your-org/mev-bot/workflows/MEV%20Bot%20CI/CD%20Pipeline/badge.svg)](https://github.com/your-org/mev-bot/actions)
[![Coverage](https://codecov.io/gh/your-org/mev-bot/branch/main/graph/badge.svg)](https://codecov.io/gh/your-org/mev-bot)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A high-performance, production-ready MEV (Maximal Extractable Value) bot designed for real-time detection and execution of arbitrage opportunities on Ethereum-compatible networks.

## 🚀 Features

- **Ultra-Low Latency**: ≤20ms median detection latency, ≤25ms decision loop
- **High Throughput**: ≥200 simulations/second with concurrent processing
- **Real-Time Mempool Monitoring**: WebSocket-based transaction ingestion with backpressure handling (Ethereum networks)
- **HyperLiquid Dual-Channel Integration**: WebSocket for real-time market data + RPC for blockchain operations (no mempool access on HyperLiquid EVM)
- **Advanced Strategy System**: Pluggable strategies for backrun, sandwich, and custom MEV opportunities
- **Fork Simulation**: Accurate profit estimation using eth_call with state overrides
- **Production Monitoring**: Comprehensive Prometheus metrics and Grafana dashboards
- **Robust Infrastructure**: Automated deployment, rollback procedures, and chaos testing

## 📋 Table of Contents

- [Quick Start](#quick-start)
- [Architecture](#architecture)
- [Installation](#installation)
- [Configuration](#configuration)
- [Deployment](#deployment)
- [Monitoring](#monitoring)
- [Development](#development)
- [Security](#security)
- [Troubleshooting](#troubleshooting)
- [Contributing](#contributing)

## 🚀 Quick Start

### Prerequisites

- **Rust 1.70+** with cargo
- **Docker** and Docker Compose
- **Node.js 18+** (for anvil/foundry)
- **Access to Ethereum RPC** (Alchemy, Infura, or self-hosted)

### 1. Clone and Build

```bash
git clone https://github.com/your-org/mev-bot.git
cd mev-bot

# Build in release mode
cargo build --release
```

### 2. Configuration

```bash
# Copy example configuration
cp config/example.yaml config/mainnet.yaml

# Edit configuration (see Configuration section)
nano config/mainnet.yaml
```

### 3. Run Tests

```bash
# Start anvil for integration tests
anvil --fork-url https://rpc.hyperevm.org --host 0.0.0.0 &

# Run comprehensive test suite
./scripts/run-all-tests.bat
```

### 4. Start the Bot

```bash
# Dry run mode (recommended first)
cargo run --release -- --config config/mainnet.yaml --dry-run

# Live mode (after validation)
cargo run --release -- --config config/mainnet.yaml
```

## 🏗️ Architecture

The MEV bot follows a modular, high-performance architecture:

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   Mempool       │    │    Strategy      │    │   Simulation    │
│   Ingestion     │───▶│    Engine        │───▶│   Engine        │
│                 │    │                  │    │                 │
│ • WebSocket     │    │ • Backrun        │    │ • Fork Sim      │
│ • Filtering     │    │ • Sandwich       │    │ • Gas Est       │
│ • Backpressure  │    │ • Custom         │    │ • Profit Calc   │
└─────────────────┘    └──────────────────┘    └─────────────────┘
         │                       │                       │
         ▼                       ▼                       ▼
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   State         │    │    Bundle        │    │   Monitoring    │
│   Management    │    │    Execution     │    │   & Metrics     │
│                 │    │                  │    │                 │
│ • Reorg Track   │    │ • Signing        │    │ • Prometheus    │
│ • Persistence   │    │ • Submission     │    │ • Grafana       │
│ • Recovery      │    │ • Tracking       │    │ • Alerting      │
└─────────────────┘    └──────────────────┘    └─────────────────┘
```

### Core Components

- **`mev-core`**: Core types, simulation engine, and bundle management
- **`mev-mempool`**: WebSocket client, transaction parsing, and filtering (not used for HyperLiquid)
- **`mev-hyperliquid`**: HyperLiquid dual-channel integration (WebSocket for market data, RPC for blockchain operations)
- **`mev-strategies`**: Strategy implementations and evaluation engine
- **`mev-config`**: Configuration management and validation
- **`mev-bot`**: Main binary and orchestration logic

**Note:** HyperLiquid EVM does not support mempool monitoring. The `mev-hyperliquid` crate uses a dual-channel architecture instead: WebSocket for real-time market data from HyperLiquid's native API, and RPC for blockchain state polling and transaction submission.

## 📦 Installation

### From Source

```bash
# Clone repository
git clone https://github.com/your-org/mev-bot.git
cd mev-bot

# Install dependencies
cargo build --release

# Install globally (optional)
cargo install --path crates/mev-bot
```

### Using Docker

```bash
# Build Docker image
docker build -t mev-bot .

# Run with Docker Compose
docker-compose up -d
```

### Pre-built Binaries

Download from [GitHub Releases](https://github.com/your-org/mev-bot/releases):

```bash
# Linux x86_64
wget https://github.com/your-org/mev-bot/releases/latest/download/mev-bot-linux-x86_64.tar.gz
tar -xzf mev-bot-linux-x86_64.tar.gz

# Windows x86_64
# Download mev-bot-windows-x86_64.zip from releases page
```

## ⚙️ Configuration

### Basic Configuration

Create `config/mainnet.yaml`:

```yaml
# Network Configuration
network:
  rpc_url: "https://rpc.hyperevm.org"
  ws_url: "wss://ws.hyperevm.org"
  chain_id: 998
  
# Mempool Settings (not applicable to HyperLiquid - see HyperLiquid Integration section)
mempool:
  max_pending_transactions: 10000
  filter_rules:
    min_gas_price: 1000000000  # 1 gwei
    max_gas_limit: 1000000
    target_contracts:
      - "0x1234567890123456789012345678901234567890"  # Uniswap V2 Router
  
# Strategy Configuration
strategies:
  backrun:
    enabled: true
    min_profit_wei: 10000000000000000  # 0.01 ETH
    max_gas_price: 100000000000        # 100 gwei
    slippage_tolerance: 0.01
  
  sandwich:
    enabled: false
    min_profit_wei: 50000000000000000  # 0.05 ETH
    max_gas_price: 200000000000        # 200 gwei
    slippage_tolerance: 0.005

# Performance Settings
performance:
  simulation_concurrency: 20
  decision_timeout_ms: 25
  cpu_pinning: [0, 1, 2, 3]  # Pin to specific CPU cores
  
# Security Settings
security:
  private_key_file: "/secure/keys/mev-bot.key"
  max_bundle_value_eth: 10.0
  enable_dry_run: false
  
# Monitoring
monitoring:
  prometheus_port: 9090
  health_check_port: 8080
  log_level: "info"
  metrics_interval_seconds: 10
```

### Environment Variables

```bash
# Required
export MEV_BOT_PRIVATE_KEY="0x..."
export MEV_BOT_RPC_URL="https://rpc.hyperevm.org"

# Optional
export MEV_BOT_WS_URL="wss://ws.hyperevm.org"
export MEV_BOT_CHAIN_ID="998"
export RUST_LOG="info"
```

### HyperLiquid Integration

The bot integrates with HyperLiquid using a **dual-channel architecture** that combines WebSocket streaming for real-time market data with RPC polling for blockchain operations.

#### Architecture Overview

**Important:** HyperLiquid EVM does not support traditional mempool queries. Unlike standard Ethereum networks, you cannot subscribe to pending transactions on HyperLiquid. The bot uses two separate channels:

1. **WebSocket Channel**: Real-time market data from HyperLiquid's native API (`wss://api.hyperliquid.xyz/ws`)
2. **RPC Channel**: Blockchain state polling and transaction submission via QuickNode RPC endpoint

```
┌─────────────────────────────────────────────────────────────────┐
│                         MEV Bot Core                             │
│                                                                   │
│  ┌──────────────────┐         ┌──────────────────────────────┐  │
│  │  Strategy Engine │◄────────┤   Opportunity Detector       │  │
│  └────────┬─────────┘         └──────────▲───────────────────┘  │
│           │                               │                       │
│           │ Execute                       │ Opportunities         │
│           ▼                               │                       │
│  ┌──────────────────┐                    │                       │
│  │ Transaction      │                    │                       │
│  │ Executor         │                    │                       │
│  └────────┬─────────┘                    │                       │
└───────────┼──────────────────────────────┼───────────────────────┘
            │                               │
            │ Submit TX                     │ Market Data + State
            ▼                               │
┌───────────────────────────────────────────┼───────────────────────┐
│           HyperLiquid Integration         │                       │
│                                           │                       │
│  ┌────────────────────────────────────────┴─────────────────┐    │
│  │              HyperLiquid Service Manager                  │    │
│  └───────┬──────────────────────────────────────┬───────────┘    │
│          │                                       │                │
│          ▼                                       ▼                │
│  ┌──────────────────┐                  ┌──────────────────┐      │
│  │  WebSocket       │                  │  RPC Client      │      │
│  │  Service         │                  │  Service         │      │
│  │                  │                  │                  │      │
│  │  Market Data     │                  │  State Polling   │      │
│  │  Streaming       │                  │  TX Submission   │      │
│  └────────┬─────────┘                  └────────┬─────────┘      │
└───────────┼──────────────────────────────────────┼────────────────┘
            │                                       │
            ▼                                       ▼
    wss://api.hyperliquid.xyz/ws    https://...quiknode.pro/.../evm
    (Market Data)                    (Blockchain Operations)
```

#### Why Dual-Channel Architecture?

HyperLiquid EVM's architecture differs from traditional Ethereum networks:

- **No Mempool Access**: The `eth_subscribe("pendingTransactions")` method is not supported
- **Native Exchange**: HyperLiquid has a native exchange with its own WebSocket API for market data
- **EVM Compatibility**: The blockchain layer is EVM-compatible and accessible via standard RPC

This dual-channel approach allows the bot to:
- Monitor real-time market movements via WebSocket
- Verify on-chain state and submit transactions via RPC
- Detect arbitrage opportunities by combining both data sources

#### Configuration

```yaml
# HyperLiquid Dual-Channel Configuration
hyperliquid:
  # Enable/disable HyperLiquid integration
  enabled: true
  
  # WebSocket Configuration (Market Data)
  # Connects to HyperLiquid's native exchange API for real-time trades
  ws_url: "wss://api.hyperliquid.xyz/ws"
  
  # Trading pairs to monitor (coin symbols)
  trading_pairs:
    - "BTC"
    - "ETH"
    - "SOL"
    - "ARB"
  
  # Subscribe to order book updates (optional)
  subscribe_orderbook: false
  
  # RPC Configuration (Blockchain Operations)
  # Connects to HyperLiquid EVM via QuickNode for state queries and transaction submission
  rpc_url: "https://cosmopolitan-quaint-voice.hype-mainnet.quiknode.pro/YOUR_API_KEY/evm"
  
  # Polling interval for blockchain state (milliseconds)
  # Default: 1000ms (1 second)
  polling_interval_ms: 1000
  
  # Reconnection settings (for WebSocket)
  reconnect_min_backoff_secs: 1
  reconnect_max_backoff_secs: 60
  max_consecutive_failures: 10
  
  # Token mapping for cross-exchange arbitrage
  # Maps HyperLiquid coin symbols to EVM token addresses
  token_mapping:
    BTC: "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599"  # WBTC
    ETH: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"  # WETH
    SOL: "0x5288738df1aB05A68337cB9dD7a607285Ac3Cf90"  # SOL
    ARB: "0x912CE59144191C1204E64559FE8253a0e49E6548"  # ARB
```

#### Opportunity Detection Flow

The bot combines data from both channels to detect and execute opportunities:

1. **Market Data Streaming** (WebSocket):
   - Receives real-time trade data from HyperLiquid exchange
   - Monitors price movements and trading volume
   - Detects potential arbitrage opportunities

2. **State Verification** (RPC):
   - Polls blockchain state at regular intervals
   - Queries token prices from DEX contracts
   - Verifies smart contract states
   - Confirms opportunity conditions on-chain

3. **Opportunity Validation**:
   - Combines WebSocket market data with RPC blockchain state
   - Calculates expected profit considering gas costs
   - Validates that opportunity still exists on-chain

4. **Transaction Execution** (RPC):
   - Signs transaction with configured private key
   - Submits via `eth_sendRawTransaction`
   - Polls for transaction confirmation
   - Tracks gas usage and success/failure

#### Key Features

**WebSocket Channel:**
- Real-time trade data streaming from HyperLiquid's native exchange
- Automatic reconnection with exponential backoff
- Subscription management with retry logic
- Order book monitoring (optional)
- Low-latency market data updates

**RPC Channel:**
- Blockchain state polling at configurable intervals
- Transaction submission and confirmation tracking
- Smart contract state queries
- Retry logic with exponential backoff
- Circuit breaker for consecutive failures

**Combined Features:**
- Cross-exchange arbitrage detection
- On-chain opportunity verification
- Graceful degradation (continues WebSocket if RPC fails)
- Comprehensive metrics and monitoring

#### Metrics

**WebSocket Metrics:**
- `hyperliquid_ws_connected`: Connection status (0/1)
- `hyperliquid_trades_received_total`: Total trades received by coin and side
- `hyperliquid_message_processing_duration_seconds`: Message processing latency
- `hyperliquid_active_subscriptions`: Number of active subscriptions
- `hyperliquid_reconnection_attempts_total`: Reconnection attempts by reason
- `hyperliquid_degraded_state`: Degraded state indicator (0/1)

**RPC Metrics:**
- `hyperliquid_rpc_calls_total`: Total RPC calls by method and status
- `hyperliquid_rpc_call_duration_seconds`: RPC call latency histogram
- `hyperliquid_rpc_errors_total`: RPC errors by method and error type
- `hyperliquid_state_poll_interval_seconds`: Current polling interval
- `hyperliquid_state_freshness_seconds`: Time since last successful state update
- `hyperliquid_tx_submitted_total`: Transactions submitted by status
- `hyperliquid_tx_confirmation_duration_seconds`: Transaction confirmation time
- `hyperliquid_tx_gas_used`: Gas used by transactions

**Opportunity Detection Metrics:**
- `hyperliquid_opportunities_detected_total`: Opportunities detected by type
- `hyperliquid_opportunities_verified_total`: Opportunities verified via RPC
- `hyperliquid_opportunity_detection_latency_seconds`: Detection latency

#### Troubleshooting

##### WebSocket Connection Issues

**Symptoms:** No market data received, `hyperliquid_ws_connected` = 0

**Diagnosis:**
```bash
# Test WebSocket connectivity
wscat -c wss://api.hyperliquid.xyz/ws

# Check subscription status
curl http://localhost:9090/metrics | grep hyperliquid_active_subscriptions

# Review WebSocket logs
docker logs mev-bot | grep "hyperliquid.*ws"
```

**Solutions:**
- Verify trading pairs are valid HyperLiquid symbols (BTC, ETH, SOL, etc.)
- Check if `enabled: true` in configuration
- Ensure network allows WebSocket connections (check firewall)
- Review reconnection metrics: `hyperliquid_reconnection_attempts_total`
- Service will automatically recover when connection is restored

##### RPC Connection Issues

**Symptoms:** No state updates, `hyperliquid_rpc_errors_total` increasing

**Diagnosis:**
```bash
# Test RPC connectivity
curl -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  https://your-quicknode-url.quiknode.pro/YOUR_API_KEY/evm

# Check RPC metrics
curl http://localhost:9090/metrics | grep hyperliquid_rpc

# Review RPC logs
docker logs mev-bot | grep "hyperliquid.*rpc"
```

**Solutions:**
- Verify QuickNode RPC URL is correct and includes API key
- Check QuickNode account for rate limits or quota issues
- Ensure `rpc_url` is configured in `config/mainnet.yaml`
- Verify network connectivity to QuickNode endpoint
- Check `hyperliquid_state_freshness_seconds` - high values indicate stale data
- Review circuit breaker status in logs

##### No Opportunities Detected

**Symptoms:** Bot running but no opportunities found

**Diagnosis:**
```bash
# Check both channels are working
curl http://localhost:9090/metrics | grep -E "hyperliquid_(ws_connected|rpc_calls_total)"

# Review opportunity metrics
curl http://localhost:9090/metrics | grep hyperliquid_opportunities

# Check logs for detection logic
docker logs mev-bot | grep "opportunity"
```

**Solutions:**
- Verify both WebSocket and RPC channels are connected
- Check that trading pairs are configured correctly
- Review token mapping for correct EVM addresses
- Ensure `polling_interval_ms` is not too high (default: 1000ms)
- Check strategy configuration for reasonable profit thresholds
- Monitor market conditions - opportunities may be rare

##### High RPC Latency

**Symptoms:** `hyperliquid_rpc_call_duration_seconds` p95 > 500ms

**Diagnosis:**
```bash
# Check RPC latency distribution
curl http://localhost:9090/metrics | grep hyperliquid_rpc_call_duration_seconds

# Test direct RPC latency
time curl -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  https://your-quicknode-url.quiknode.pro/YOUR_API_KEY/evm
```

**Solutions:**
- Consider using a QuickNode endpoint closer to your deployment region
- Increase `polling_interval_ms` to reduce RPC call frequency
- Check QuickNode account tier - higher tiers have better performance
- Monitor network latency between bot and QuickNode
- Consider caching frequently queried state

##### Transaction Submission Failures

**Symptoms:** `hyperliquid_tx_submitted_total{status="failed"}` increasing

**Diagnosis:**
```bash
# Check transaction error types
curl http://localhost:9090/metrics | grep hyperliquid_tx_errors_total

# Review transaction logs
docker logs mev-bot | grep "transaction.*failed"
```

**Solutions:**
- Verify private key is configured correctly
- Check account has sufficient balance for gas
- Review gas price settings in strategy configuration
- Ensure transactions are properly signed
- Check for nonce management issues
- Verify smart contract addresses are correct

##### Degraded State

**Symptoms:** `hyperliquid_degraded_state` = 1

**Diagnosis:**
```bash
# Check degraded state reason
docker logs mev-bot | grep "degraded"

# Review both channel statuses
curl http://localhost:9090/metrics | grep -E "hyperliquid_(ws_connected|rpc_errors)"
```

**Solutions:**
- Check which channel is failing (WebSocket or RPC)
- Review reconnection attempts and backoff timing
- Verify both endpoints are operational
- Check for network connectivity issues
- Monitor for service recovery - bot continues operating with available channel
- If persistent, restart the bot: `docker restart mev-bot`

##### General Debugging

```bash
# View all HyperLiquid metrics
curl http://localhost:9090/metrics | grep hyperliquid

# Follow logs in real-time
docker logs -f mev-bot | grep hyperliquid

# Check configuration
docker exec mev-bot cat /app/config/mainnet.yaml | grep -A 30 hyperliquid

# Test both endpoints
wscat -c wss://api.hyperliquid.xyz/ws
curl -X POST https://your-quicknode-url.quiknode.pro/YOUR_API_KEY/evm \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```

### Advanced Configuration

See [docs/configuration.md](docs/configuration.md) for detailed configuration options.

## 🚀 Deployment

### Testnet Deployment

1. **Configure for testnet**:
```yaml
network:
  rpc_url: "https://testnet-rpc.hyperevm.org"
  chain_id: 999
security:
  enable_dry_run: true
  max_bundle_value_eth: 0.1
```

2. **Deploy and test**:
```bash
cargo run --release -- --config config/testnet.yaml --dry-run
```

### Mainnet Deployment

1. **Security checklist**:
   - [ ] Private keys stored securely
   - [ ] Configuration validated
   - [ ] Dry run testing completed
   - [ ] Monitoring configured
   - [ ] Rollback procedures tested

2. **Deploy to staging**:
```bash
# Build and deploy to staging
docker build -t mev-bot:staging .
docker run -d --name mev-bot-staging \
  -p 8080:8080 -p 9090:9090 \
  -v ./config:/app/config:ro \
  -v ./keys:/app/keys:ro \
  mev-bot:staging
```

3. **Validate deployment**:
```bash
./scripts/validate-deployment.bat staging
```

4. **Deploy to production**:
```bash
# Use CI/CD pipeline or manual deployment
docker tag mev-bot:staging mev-bot:production
docker run -d --name mev-bot-production \
  --restart unless-stopped \
  -p 8080:8080 -p 9090:9090 \
  -v ./config:/app/config:ro \
  -v ./keys:/app/keys:ro \
  mev-bot:production
```

### Using CI/CD Pipeline

The repository includes GitHub Actions workflows for automated deployment:

- **Staging**: Deploys on push to `develop` branch
- **Production**: Deploys on push to `main` branch
- **Rollback**: Manual workflow for emergency rollbacks

See [.github/workflows/](/.github/workflows/) for pipeline configuration.

## 📊 Monitoring

### Prometheus Metrics

Key metrics exposed on `:9090/metrics`:

**Core Metrics:**
- `mev_bot_mempool_transactions_total`: Total transactions processed
- `mev_bot_detection_latency_seconds`: Strategy detection latency
- `mev_bot_simulation_latency_seconds`: Bundle simulation latency
- `mev_bot_bundle_success_total`: Successful bundle submissions
- `mev_bot_profit_eth_total`: Total profit in ETH

**HyperLiquid Metrics (Dual-Channel):**

*WebSocket Channel (Market Data):*
- `hyperliquid_ws_connected`: WebSocket connection status (0/1)
- `hyperliquid_trades_received_total`: Total trades received by coin and side
- `hyperliquid_message_processing_duration_seconds`: Message processing latency histogram
- `hyperliquid_active_subscriptions`: Number of active trading pair subscriptions
- `hyperliquid_reconnection_attempts_total`: Total reconnection attempts by reason
- `hyperliquid_degraded_state`: Degraded state indicator (0/1)

*RPC Channel (Blockchain Operations):*
- `hyperliquid_rpc_calls_total`: Total RPC calls by method and status
- `hyperliquid_rpc_call_duration_seconds`: RPC call latency histogram by method
- `hyperliquid_rpc_errors_total`: RPC errors by method and error type
- `hyperliquid_state_poll_interval_seconds`: Current blockchain state polling interval
- `hyperliquid_state_freshness_seconds`: Time since last successful state update
- `hyperliquid_tx_submitted_total`: Transactions submitted by status
- `hyperliquid_tx_confirmation_duration_seconds`: Transaction confirmation time histogram
- `hyperliquid_tx_gas_used`: Gas used by transactions histogram

*Opportunity Detection:*
- `hyperliquid_opportunities_detected_total`: Opportunities detected by type
- `hyperliquid_opportunities_verified_total`: Opportunities verified via RPC by type
- `hyperliquid_opportunity_detection_latency_seconds`: Detection latency histogram

### Grafana Dashboards

Import dashboards from `monitoring/grafana/`:

1. **MEV Bot Overview**: High-level metrics and performance
2. **Latency Analysis**: Detailed latency histograms and percentiles
3. **Strategy Performance**: Per-strategy success rates and profitability
4. **System Health**: Resource usage and error rates

### Health Checks

- **Health endpoint**: `GET :8080/health`
- **Readiness check**: `GET :8080/ready`
- **Metrics endpoint**: `GET :9090/metrics`

### Alerting

Configure alerts for:
- Detection latency > 50ms (p95)
- Simulation latency > 100ms (p95)
- Error rate > 5%
- Bundle success rate < 80%
- Memory usage > 80%

## 🛠️ Development

### Building from Source

```bash
# Development build
cargo build

# Release build with optimizations
cargo build --release

# Build specific crate
cargo build -p mev-core
```

### Running Tests

```bash
# Unit tests
cargo test

# Integration tests (requires anvil)
anvil --fork-url https://rpc.hyperevm.org &
cargo test --test integration

# Load tests
cargo test --test load_testing -- --ignored

# All tests with coverage
./scripts/test-coverage.bat
```

### Benchmarking

```bash
# Run all benchmarks
cargo bench

# Specific benchmark
cargo bench --bench mempool_ingestion
cargo bench --bench simulation_engine
cargo bench --bench strategy_performance
```

### Code Quality

```bash
# Format code
cargo fmt

# Lint code
cargo clippy --all-targets --all-features

# Security audit
cargo audit

# Dependency check
cargo deny check
```

### Adding New Strategies

1. **Implement Strategy trait**:
```rust
use async_trait::async_trait;
use mev_strategies::{Strategy, StrategyResult};

pub struct MyStrategy {
    config: MyStrategyConfig,
}

#[async_trait]
impl Strategy for MyStrategy {
    async fn evaluate(&self, tx: &ParsedTransaction) -> StrategyResult<Option<Opportunity>> {
        // Strategy logic here
        Ok(None)
    }
}
```

2. **Register strategy**:
```rust
let mut engine = StrategyEngine::new();
engine.register_strategy(Box::new(MyStrategy::new(config))).await?;
```

3. **Add configuration**:
```yaml
strategies:
  my_strategy:
    enabled: true
    min_profit_wei: 1000000000000000
    # Custom parameters
```

## 🔒 Security

### Key Management

- **Never commit private keys** to version control
- Use encrypted key files or hardware security modules
- Rotate keys regularly
- Implement key access logging

### Network Security

- Use TLS for all RPC connections
- Validate SSL certificates
- Implement rate limiting
- Monitor for suspicious activity

### Operational Security

- Run with minimal privileges
- Use read-only configuration mounts
- Implement circuit breakers
- Monitor for anomalous behavior

### Security Checklist

- [ ] Private keys encrypted and secured
- [ ] Configuration validated and reviewed
- [ ] Network connections use TLS
- [ ] Monitoring and alerting configured
- [ ] Incident response procedures documented
- [ ] Regular security audits scheduled

## 🔧 Troubleshooting

### Common Issues

#### High Latency
```bash
# Check system resources
htop
iostat -x 1

# Review configuration
cargo run -- --validate-config

# Check network connectivity
ping rpc.hyperevm.org
```

#### Connection Issues
```bash
# Test RPC connectivity
curl -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  https://rpc.hyperevm.org

# Check WebSocket connection
wscat -c wss://ws.hyperevm.org
```

#### Memory Issues
```bash
# Monitor memory usage
docker stats mev-bot-production

# Check for memory leaks
valgrind --tool=memcheck ./target/release/mev-bot --dry-run
```

### Log Analysis

```bash
# View recent logs
docker logs mev-bot-production --tail 100

# Search for errors
docker logs mev-bot-production | grep ERROR

# Follow logs in real-time
docker logs -f mev-bot-production
```

### Performance Debugging

```bash
# Profile CPU usage
cargo flamegraph --bin mev-bot

# Benchmark specific components
cargo bench --bench simulation_engine

# Memory profiling
cargo run --bin mev-bot --features dhat-heap
```

## ✅ Validation and Testing

### HyperLiquid Mainnet Validation

Before deploying to production, validate the HyperLiquid integration:

```bash
# Quick validation (5 minutes)
scripts\validate-hyperliquid-mainnet.bat

# Run integration tests
cargo test --test hyperliquid_mainnet_validation -- --ignored --nocapture

# Monitor in real-time
scripts\monitor-hyperliquid.bat
```

**Validation Documentation:**
- [Quick Start Guide](VALIDATION_QUICK_START.md) - 5-minute validation checklist
- [Comprehensive Guide](HYPERLIQUID_MAINNET_VALIDATION.md) - Detailed validation procedures

**What Gets Validated:**
- ✅ Configuration structure and values
- ✅ WebSocket connection to HyperLiquid API
- ✅ RPC connection to QuickNode
- ✅ Market data streaming (trades, order books)
- ✅ Blockchain state polling (blocks, prices)
- ✅ Metrics collection (Prometheus)
- ✅ Opportunity detection flow
- ✅ Error handling and resilience
- ✅ Transaction submission (with caution)

**Requirements Validated:**
- Dual-channel architecture (WebSocket + RPC)
- Real-time market data streaming
- Blockchain state polling
- Transaction submission and confirmation
- Comprehensive metrics and monitoring
- Error handling and graceful degradation

See [HYPERLIQUID_MAINNET_VALIDATION.md](HYPERLIQUID_MAINNET_VALIDATION.md) for complete validation procedures.

## 📚 Documentation

- [Configuration Guide](docs/configuration.md)
- [Strategy Development](docs/strategies.md)
- [Deployment Guide](docs/deployment.md)
- [Monitoring Setup](docs/monitoring.md)
- [Security Guidelines](docs/security.md)
- [API Reference](docs/api.md)
- [Troubleshooting](docs/troubleshooting.md)
- [HyperLiquid Validation](HYPERLIQUID_MAINNET_VALIDATION.md) - **NEW**
- [Validation Quick Start](VALIDATION_QUICK_START.md) - **NEW**

## 🤝 Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

### Development Setup

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests
5. Run the test suite
6. Submit a pull request

### Code Standards

- Follow Rust conventions and `rustfmt`
- Add comprehensive tests
- Update documentation
- Ensure CI passes

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## ⚠️ Disclaimer

This software is provided for educational and research purposes. MEV extraction involves financial risk and regulatory considerations. Users are responsible for:

- Understanding applicable laws and regulations
- Managing financial risks
- Ensuring proper security measures
- Complying with exchange terms of service

The authors are not responsible for any financial losses or legal issues arising from the use of this software.

## 🙏 Acknowledgments

- [Ethereum Foundation](https://ethereum.org/) for the underlying technology
- [Flashbots](https://flashbots.net/) for MEV research and tooling
- [Foundry](https://github.com/foundry-rs/foundry) for development tools
- The Rust community for excellent libraries and tools

## 📞 Support

- **Documentation**: [docs/](docs/)
- **Issues**: [GitHub Issues](https://github.com/your-org/mev-bot/issues)
- **Discussions**: [GitHub Discussions](https://github.com/your-org/mev-bot/discussions)
- **Security**: security@your-org.com

---

**Built with ❤️ and ⚡ by the MEV Bot Team**