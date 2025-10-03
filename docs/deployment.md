# MEV Bot Deployment Guide

This guide covers comprehensive deployment procedures for the MEV bot across different environments, from local development to production mainnet deployment.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Environment Setup](#environment-setup)
- [Local Development](#local-development)
- [Testnet Deployment](#testnet-deployment)
- [Staging Environment](#staging-environment)
- [Production Deployment](#production-deployment)
- [Monitoring Setup](#monitoring-setup)
- [Security Considerations](#security-considerations)
- [Rollback Procedures](#rollback-procedures)
- [Troubleshooting](#troubleshooting)

## Prerequisites

### System Requirements

**Minimum Requirements:**
- CPU: 4 cores, 2.5GHz+
- RAM: 8GB
- Storage: 50GB SSD
- Network: 100Mbps with low latency to Ethereum RPC

**Recommended for Production:**
- CPU: 8+ cores, 3.0GHz+ (Intel Xeon or AMD EPYC)
- RAM: 32GB+
- Storage: 200GB NVMe SSD
- Network: 1Gbps with <10ms latency to RPC provider

### Software Dependencies

```bash
# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update stable

# Docker and Docker Compose
curl -fsSL https://get.docker.com -o get-docker.sh
sh get-docker.sh
sudo usermod -aG docker $USER

# Node.js and Foundry (for testing)
curl -fsSL https://deb.nodesource.com/setup_18.x | sudo -E bash -
sudo apt-get install -y nodejs
curl -L https://foundry.paradigm.xyz | bash
foundryup
```

### Network Access

- **RPC Provider**: Alchemy, Infura, or self-hosted Ethereum node
- **WebSocket Access**: For real-time mempool monitoring
- **Outbound HTTPS**: For bundle submission and monitoring
- **Monitoring Ports**: 8080 (health), 9090 (metrics)

## Environment Setup

### Configuration Management

Create environment-specific configurations:

```bash
mkdir -p config/{local,testnet,staging,production}
```

### Environment Variables

Create `.env` files for each environment:

**`.env.local`:**
```bash
MEV_BOT_ENV=local
MEV_BOT_RPC_URL=http://localhost:8545
MEV_BOT_WS_URL=ws://localhost:8545
MEV_BOT_CHAIN_ID=31337
MEV_BOT_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
RUST_LOG=debug
```

**`.env.testnet`:**
```bash
MEV_BOT_ENV=testnet
MEV_BOT_RPC_URL=https://testnet-rpc.hyperevm.org
MEV_BOT_WS_URL=wss://testnet-ws.hyperevm.org
MEV_BOT_CHAIN_ID=999
MEV_BOT_PRIVATE_KEY_FILE=/secure/keys/testnet.key
RUST_LOG=info
```

**`.env.production`:**
```bash
MEV_BOT_ENV=production
MEV_BOT_RPC_URL=https://rpc.hyperevm.org
MEV_BOT_WS_URL=wss://ws.hyperevm.org
MEV_BOT_CHAIN_ID=998
MEV_BOT_PRIVATE_KEY_FILE=/secure/keys/production.key
RUST_LOG=warn
```

## Local Development

### 1. Setup Local Environment

```bash
# Clone repository
git clone https://github.com/your-org/mev-bot.git
cd mev-bot

# Install dependencies
cargo build

# Start local blockchain (anvil)
anvil --fork-url https://rpc.hyperevm.org --host 0.0.0.0 --port 8545 &
```

### 2. Configure Local Settings

Create `config/local.yaml`:

```yaml
network:
  rpc_url: "http://localhost:8545"
  ws_url: "ws://localhost:8545"
  chain_id: 31337

mempool:
  max_pending_transactions: 1000
  filter_rules:
    min_gas_price: 1000000000

strategies:
  backrun:
    enabled: true
    min_profit_wei: 1000000000000000
  sandwich:
    enabled: false

performance:
  simulation_concurrency: 5
  decision_timeout_ms: 100

security:
  enable_dry_run: true
  max_bundle_value_eth: 0.1

monitoring:
  prometheus_port: 9090
  health_check_port: 8080
  log_level: "debug"
```

### 3. Run Local Tests

```bash
# Load environment
source .env.local

# Run comprehensive tests
./scripts/run-all-tests.bat

# Start bot in dry-run mode
cargo run --release -- --config config/local.yaml --dry-run
```

## Testnet Deployment

### 1. Testnet Configuration

Create `config/testnet.yaml`:

```yaml
network:
  rpc_url: "https://testnet-rpc.hyperevm.org"
  ws_url: "wss://testnet-ws.hyperevm.org"
  chain_id: 999

mempool:
  max_pending_transactions: 5000
  filter_rules:
    min_gas_price: 1000000000
    max_gas_limit: 1000000

strategies:
  backrun:
    enabled: true
    min_profit_wei: 5000000000000000  # 0.005 ETH
    max_gas_price: 50000000000        # 50 gwei
  sandwich:
    enabled: true
    min_profit_wei: 10000000000000000 # 0.01 ETH
    max_gas_price: 100000000000       # 100 gwei

performance:
  simulation_concurrency: 10
  decision_timeout_ms: 50

security:
  private_key_file: "/secure/keys/testnet.key"
  enable_dry_run: false
  max_bundle_value_eth: 1.0

monitoring:
  prometheus_port: 9090
  health_check_port: 8080
  log_level: "info"
```

### 2. Deploy to Testnet

```bash
# Build release binary
cargo build --release

# Create secure key directory
sudo mkdir -p /secure/keys
sudo chmod 700 /secure/keys

# Generate or copy testnet private key
echo "0x..." | sudo tee /secure/keys/testnet.key
sudo chmod 600 /secure/keys/testnet.key

# Run testnet deployment
cargo run --release -- --config config/testnet.yaml
```

### 3. Validate Testnet Deployment

```bash
# Check health
curl http://localhost:8080/health

# Check metrics
curl http://localhost:9090/metrics

# Run validation script
./scripts/validate-deployment.bat testnet http://localhost:8080 http://localhost:9090
```

## Staging Environment

### 1. Docker Setup

Create `docker-compose.staging.yml`:

```yaml
version: '3.8'

services:
  mev-bot:
    build: .
    container_name: mev-bot-staging
    restart: unless-stopped
    ports:
      - "8080:8080"
      - "9090:9090"
    volumes:
      - ./config:/app/config:ro
      - /secure/keys:/app/keys:ro
    environment:
      - MEV_BOT_ENV=staging
      - RUST_LOG=info
    command: ["/usr/local/bin/mev-bot", "--config", "/app/config/staging.yaml"]
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 40s

  prometheus:
    image: prom/prometheus:latest
    container_name: prometheus-staging
    ports:
      - "9091:9090"
    volumes:
      - ./monitoring/prometheus:/etc/prometheus:ro
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.path=/prometheus'
      - '--web.console.libraries=/etc/prometheus/console_libraries'
      - '--web.console.templates=/etc/prometheus/consoles'

  grafana:
    image: grafana/grafana:latest
    container_name: grafana-staging
    ports:
      - "3000:3000"
    volumes:
      - ./monitoring/grafana:/etc/grafana/provisioning:ro
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=admin123
```

### 2. Staging Configuration

Create `config/staging.yaml`:

```yaml
network:
  rpc_url: "https://rpc.hyperevm.org"
  ws_url: "wss://ws.hyperevm.org"
  chain_id: 998

mempool:
  max_pending_transactions: 8000
  filter_rules:
    min_gas_price: 2000000000        # 2 gwei
    max_gas_limit: 800000
    target_contracts:
      - "0x1234567890123456789012345678901234567890"

strategies:
  backrun:
    enabled: true
    min_profit_wei: 10000000000000000  # 0.01 ETH
    max_gas_price: 80000000000         # 80 gwei
    slippage_tolerance: 0.01
  sandwich:
    enabled: true
    min_profit_wei: 25000000000000000  # 0.025 ETH
    max_gas_price: 150000000000        # 150 gwei
    slippage_tolerance: 0.005

performance:
  simulation_concurrency: 15
  decision_timeout_ms: 30
  cpu_pinning: [0, 1, 2, 3]

security:
  private_key_file: "/app/keys/staging.key"
  enable_dry_run: false
  max_bundle_value_eth: 5.0

monitoring:
  prometheus_port: 9090
  health_check_port: 8080
  log_level: "info"
  metrics_interval_seconds: 5
```

### 3. Deploy Staging

```bash
# Build and deploy
docker-compose -f docker-compose.staging.yml up -d

# Wait for services to start
sleep 30

# Validate deployment
./scripts/validate-deployment.bat staging http://localhost:8080 http://localhost:9090

# Check logs
docker logs mev-bot-staging --tail 50
```

## Production Deployment

### 1. Pre-deployment Checklist

- [ ] **Security Review**: Keys secured, configuration validated
- [ ] **Performance Testing**: Load tests passed, benchmarks acceptable
- [ ] **Monitoring Setup**: Prometheus, Grafana, alerting configured
- [ ] **Backup Procedures**: Key backup, configuration backup
- [ ] **Rollback Plan**: Tested rollback procedures
- [ ] **Team Notification**: Operations team informed
- [ ] **Maintenance Window**: Scheduled if required

### 2. Production Configuration

Create `config/production.yaml`:

```yaml
network:
  rpc_url: "https://rpc.hyperevm.org"
  ws_url: "wss://ws.hyperevm.org"
  chain_id: 998

mempool:
  max_pending_transactions: 15000
  filter_rules:
    min_gas_price: 5000000000         # 5 gwei
    max_gas_limit: 500000
    target_contracts:
      - "0x1234567890123456789012345678901234567890"  # Uniswap V2
      - "0x2345678901234567890123456789012345678901"  # Uniswap V3
      - "0x3456789012345678901234567890123456789012"  # SushiSwap

strategies:
  backrun:
    enabled: true
    min_profit_wei: 20000000000000000  # 0.02 ETH
    max_gas_price: 100000000000        # 100 gwei
    slippage_tolerance: 0.005
    max_position_size_eth: 50.0
  sandwich:
    enabled: true
    min_profit_wei: 50000000000000000  # 0.05 ETH
    max_gas_price: 200000000000        # 200 gwei
    slippage_tolerance: 0.003
    max_position_size_eth: 100.0

performance:
  simulation_concurrency: 25
  decision_timeout_ms: 20
  cpu_pinning: [0, 1, 2, 3, 4, 5, 6, 7]
  memory_limit_gb: 16

security:
  private_key_file: "/app/keys/production.key"
  enable_dry_run: false
  max_bundle_value_eth: 20.0
  enable_circuit_breaker: true
  max_consecutive_failures: 10

monitoring:
  prometheus_port: 9090
  health_check_port: 8080
  log_level: "warn"
  metrics_interval_seconds: 1
  enable_detailed_metrics: true
```

### 3. Production Deployment Steps

#### Blue-Green Deployment

```bash
# 1. Prepare new version (Green)
docker build -t mev-bot:production-$(date +%Y%m%d-%H%M%S) .
docker tag mev-bot:production-$(date +%Y%m%d-%H%M%S) mev-bot:production-new

# 2. Stop current version (Blue) gracefully
docker exec mev-bot-production /usr/local/bin/mev-bot --shutdown-graceful
docker stop mev-bot-production
docker rename mev-bot-production mev-bot-production-backup

# 3. Start new version (Green)
docker run -d \
  --name mev-bot-production \
  --restart unless-stopped \
  -p 8080:8080 \
  -p 9090:9090 \
  -v $(pwd)/config:/app/config:ro \
  -v /secure/keys:/app/keys:ro \
  --memory=16g \
  --cpus=8 \
  mev-bot:production-new

# 4. Wait for startup
sleep 30

# 5. Validate deployment
./scripts/validate-deployment.bat production http://localhost:8080 http://localhost:9090

# 6. Monitor for 10 minutes
for i in {1..10}; do
  echo "Monitoring minute $i/10..."
  curl -f http://localhost:8080/health || exit 1
  sleep 60
done

# 7. Remove backup if successful
docker rm mev-bot-production-backup
```

#### Using CI/CD Pipeline

The GitHub Actions workflow handles production deployment automatically:

```bash
# Trigger production deployment
git checkout main
git merge develop
git push origin main

# Monitor deployment
gh run watch
```

### 4. Post-deployment Validation

```bash
# Comprehensive validation
./scripts/validate-deployment.bat production

# Performance validation
curl -s http://localhost:9090/metrics | grep mev_bot_detection_latency

# Security validation
docker exec mev-bot-production /usr/local/bin/mev-bot --validate-config --security-check

# Load test (optional)
./scripts/chaos-testing.bat
```

## Monitoring Setup

### 1. Prometheus Configuration

Create `monitoring/prometheus/prometheus.yml`:

```yaml
global:
  scrape_interval: 15s
  evaluation_interval: 15s

rule_files:
  - "mev_bot_rules.yml"

scrape_configs:
  - job_name: 'mev-bot'
    static_configs:
      - targets: ['localhost:9090']
    scrape_interval: 5s
    metrics_path: /metrics

alerting:
  alertmanagers:
    - static_configs:
        - targets:
          - alertmanager:9093
```

### 2. Grafana Dashboards

Import dashboards from `monitoring/grafana/dashboards/`:

- **MEV Bot Overview**: `mev-bot-overview.json`
- **Performance Metrics**: `performance-metrics.json`
- **Strategy Analysis**: `strategy-analysis.json`
- **System Health**: `system-health.json`

### 3. Alerting Rules

Create `monitoring/prometheus/mev_bot_rules.yml`:

```yaml
groups:
  - name: mev_bot_alerts
    rules:
      - alert: HighDetectionLatency
        expr: histogram_quantile(0.95, mev_bot_detection_latency_seconds) > 0.05
        for: 2m
        labels:
          severity: warning
        annotations:
          summary: "MEV bot detection latency is high"
          
      - alert: LowBundleSuccessRate
        expr: rate(mev_bot_bundle_success_total[5m]) / rate(mev_bot_bundle_attempts_total[5m]) < 0.8
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "MEV bot bundle success rate is low"
```

## Security Considerations

### 1. Key Management

```bash
# Generate secure key
openssl rand -hex 32 > /tmp/new_key.txt

# Encrypt key file
gpg --symmetric --cipher-algo AES256 /tmp/new_key.txt

# Store encrypted key
sudo mv /tmp/new_key.txt.gpg /secure/keys/production.key.gpg
sudo chmod 600 /secure/keys/production.key.gpg

# Decrypt for use
gpg --decrypt /secure/keys/production.key.gpg > /secure/keys/production.key
```

### 2. Network Security

```bash
# Configure firewall
sudo ufw allow 22/tcp    # SSH
sudo ufw allow 8080/tcp  # Health checks
sudo ufw allow 9090/tcp  # Metrics (restrict to monitoring network)
sudo ufw enable

# Configure fail2ban
sudo apt install fail2ban
sudo systemctl enable fail2ban
```

### 3. Container Security

```dockerfile
# Use non-root user in Dockerfile
RUN adduser --disabled-password --gecos '' mevbot
USER mevbot

# Read-only filesystem
docker run --read-only --tmpfs /tmp mev-bot:production
```

## Rollback Procedures

### 1. Automated Rollback

Use the GitHub Actions rollback workflow:

```bash
# Trigger emergency rollback
gh workflow run rollback.yml \
  -f environment=production \
  -f reason="High error rate detected"
```

### 2. Manual Rollback

```bash
# 1. Stop current deployment
docker stop mev-bot-production

# 2. Restore previous version
docker rename mev-bot-production mev-bot-production-failed
docker rename mev-bot-production-backup mev-bot-production
docker start mev-bot-production

# 3. Validate rollback
./scripts/validate-deployment.bat production

# 4. Investigate failure
docker logs mev-bot-production-failed > rollback-investigation.log
```

### 3. Database Rollback

```bash
# Restore state from backup
sqlite3 /data/mev-bot.db ".restore /backups/mev-bot-$(date -d '1 hour ago' +%Y%m%d-%H).db"
```

## Troubleshooting

### Common Deployment Issues

#### 1. Container Won't Start

```bash
# Check logs
docker logs mev-bot-production

# Check configuration
docker exec mev-bot-production /usr/local/bin/mev-bot --validate-config

# Check permissions
ls -la /secure/keys/
```

#### 2. High Memory Usage

```bash
# Monitor memory
docker stats mev-bot-production

# Adjust memory limits
docker update --memory=32g mev-bot-production
```

#### 3. Network Connectivity Issues

```bash
# Test RPC connectivity
docker exec mev-bot-production curl -X POST \
  -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  https://rpc.hyperevm.org
```

#### 4. Performance Issues

```bash
# Check CPU usage
htop

# Profile application
docker exec mev-bot-production perf record -g /usr/local/bin/mev-bot --profile

# Check network latency
ping rpc.hyperevm.org
```

### Emergency Procedures

#### 1. Kill Switch Activation

```bash
# Immediate shutdown
docker exec mev-bot-production /usr/local/bin/mev-bot --emergency-stop

# Or force stop
docker kill mev-bot-production
```

#### 2. Incident Response

1. **Assess Impact**: Check metrics and logs
2. **Contain Issue**: Stop bot if necessary
3. **Investigate**: Analyze logs and system state
4. **Communicate**: Notify stakeholders
5. **Resolve**: Fix issue and redeploy
6. **Post-mortem**: Document lessons learned

### Support Contacts

- **On-call Engineer**: +1-555-0123
- **DevOps Team**: devops@your-org.com
- **Security Team**: security@your-org.com
- **Escalation**: cto@your-org.com

---

This deployment guide provides comprehensive procedures for safely deploying and operating the MEV bot across all environments. Always follow the security checklist and validate deployments thoroughly before going live.