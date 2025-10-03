# MEV Bot - High-Performance Ethereum MEV Detection and Execution

[![CI/CD Pipeline](https://github.com/your-org/mev-bot/workflows/MEV%20Bot%20CI/CD%20Pipeline/badge.svg)](https://github.com/your-org/mev-bot/actions)
[![Coverage](https://codecov.io/gh/your-org/mev-bot/branch/main/graph/badge.svg)](https://codecov.io/gh/your-org/mev-bot)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A high-performance, production-ready MEV (Maximal Extractable Value) bot designed for real-time detection and execution of arbitrage opportunities on Ethereum-compatible networks.

## 🚀 Features

- **Ultra-Low Latency**: ≤20ms median detection latency, ≤25ms decision loop
- **High Throughput**: ≥200 simulations/second with concurrent processing
- **Real-Time Mempool Monitoring**: WebSocket-based transaction ingestion with backpressure handling
- **HyperLiquid Native Integration**: Real-time trade data from HyperLiquid's native WebSocket API for cross-exchange arbitrage
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
- **`mev-mempool`**: WebSocket client, transaction parsing, and filtering
- **`mev-hyperliquid`**: HyperLiquid native WebSocket integration for real-time trade data
- **`mev-strategies`**: Strategy implementations and evaluation engine
- **`mev-config`**: Configuration management and validation
- **`mev-bot`**: Main binary and orchestration logic

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
  
# Mempool Settings
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

The bot supports real-time trade monitoring from HyperLiquid's native exchange via WebSocket API:

```yaml
# HyperLiquid Native WebSocket API Configuration
hyperliquid:
  # Enable/disable HyperLiquid WebSocket integration
  enabled: true
  
  # WebSocket endpoint for HyperLiquid's native API
  ws_url: "wss://api.hyperliquid.xyz/ws"
  
  # Trading pairs to monitor (coin symbols)
  trading_pairs:
    - "BTC"
    - "ETH"
    - "SOL"
    - "ARB"
  
  # Subscribe to order book updates (optional)
  subscribe_orderbook: false
  
  # Reconnection settings
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

**Key Features:**
- Real-time trade data streaming from HyperLiquid's native exchange
- Automatic reconnection with exponential backoff
- Subscription management with retry logic
- Order book monitoring (optional)
- Cross-exchange arbitrage via token mapping
- Comprehensive metrics and monitoring

**Metrics:**
- `hyperliquid_ws_connected`: Connection status (0/1)
- `hyperliquid_trades_received_total`: Total trades received
- `hyperliquid_message_processing_duration_seconds`: Processing latency
- `hyperliquid_active_subscriptions`: Number of active subscriptions
- `hyperliquid_reconnection_attempts_total`: Reconnection attempts
- `hyperliquid_degraded_state`: Degraded state indicator (0/1)

**Troubleshooting:**

*Connection Issues:*
```bash
# Test WebSocket connectivity
wscat -c wss://api.hyperliquid.xyz/ws

# Check subscription status
curl http://localhost:9090/metrics | grep hyperliquid_active_subscriptions
```

*No Trades Received:*
- Verify trading pairs are valid HyperLiquid symbols
- Check if `enabled: true` in configuration
- Review logs for subscription errors: `docker logs mev-bot | grep hyperliquid`

*Degraded State:*
- Check network connectivity
- Verify HyperLiquid API is operational
- Review reconnection metrics: `hyperliquid_reconnection_attempts_total`
- Service will automatically recover when connection is restored

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

**HyperLiquid Metrics:**
- `hyperliquid_ws_connected`: WebSocket connection status (0/1)
- `hyperliquid_trades_received_total`: Total trades received by coin and side
- `hyperliquid_message_processing_duration_seconds`: Message processing latency histogram
- `hyperliquid_active_subscriptions`: Number of active trading pair subscriptions
- `hyperliquid_reconnection_attempts_total`: Total reconnection attempts by reason
- `hyperliquid_degraded_state`: Degraded state indicator (0/1)
- `hyperliquid_connection_errors_total`: Connection errors by type
- `hyperliquid_subscription_errors_total`: Subscription errors by coin and type
- `hyperliquid_parse_errors_total`: Message parsing errors by coin
- `hyperliquid_adaptation_errors_total`: Trade adaptation errors by coin and reason
- `hyperliquid_network_errors_total`: Network errors by type

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

## 📚 Documentation

- [Configuration Guide](docs/configuration.md)
- [Strategy Development](docs/strategies.md)
- [Deployment Guide](docs/deployment.md)
- [Monitoring Setup](docs/monitoring.md)
- [Security Guidelines](docs/security.md)
- [API Reference](docs/api.md)
- [Troubleshooting](docs/troubleshooting.md)

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