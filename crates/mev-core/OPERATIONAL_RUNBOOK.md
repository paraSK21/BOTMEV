# MEV Bot Operational Runbook

## Overview

This runbook provides operational procedures for safely deploying and managing the MEV bot on HyperEVM mainnet.

## Pre-Deployment Security Checklist

### Critical Security Items

- [ ] **Private Key Security**
  - Private keys are encrypted with AES-256-GCM
  - Key files are stored in secure directory with restricted permissions
  - Backup keys are stored offline in secure location
  - Key rotation schedule is established

- [ ] **Fund Protection**
  - Maximum fund exposure limits are configured (recommended: < 1% of total funds)
  - Emergency withdrawal procedures are tested
  - Multi-signature wallet is configured for large operations
  - Insurance coverage is in place

- [ ] **Kill Switch Functionality**
  - Kill switch mechanism is tested and functional
  - Multiple authorized operators can activate kill switch
  - Automatic kill switch triggers are configured
  - Recovery procedures are documented

### High Priority Items

- [ ] **Network Security**
  - All RPC connections use HTTPS/WSS
  - VPN or private network connection is established
  - Rate limiting is configured
  - Connection monitoring is active

- [ ] **Transaction Safety**
  - Gas limit caps are set (recommended: 500,000 gas per transaction)
  - Gas price limits are configured (recommended: 200 gwei max)
  - Slippage protection is enabled
  - Transaction timeout limits are set

- [ ] **Monitoring and Alerting**
  - Prometheus metrics are configured
  - Alert thresholds are set for key metrics
  - Log aggregation is active
  - Performance dashboards are deployed

## Deployment Procedures

### 1. Environment Setup

```bash
# Set environment variables
export MEV_BOT_ENV=production
export MEV_BOT_RPC_URL=https://rpc.hyperevm.org
export MEV_BOT_KEY_DIR=/secure/keys
export MEV_BOT_LOG_LEVEL=info

# Create secure directories
mkdir -p /secure/keys
chmod 700 /secure/keys

# Install dependencies
cargo build --release
```

### 2. Key Management Setup

```bash
# Generate encrypted key file
./target/release/mev-bot generate-key \
  --output /secure/keys/mev-bot.json \
  --password-file /secure/password.txt

# Verify key loading
./target/release/mev-bot verify-key \
  --key-file /secure/keys/mev-bot.json \
  --password-file /secure/password.txt
```

### 3. Configuration Validation

```bash
# Validate configuration
./target/release/mev-bot validate-config \
  --config config/production.yaml

# Test connectivity
./target/release/mev-bot test-connection \
  --rpc-url https://rpc.hyperevm.org
```

### 4. Dry Run Execution

```bash
# Execute mainnet dry run with dummy values
./target/release/mev-bot dry-run \
  --config config/production.yaml \
  --duration 300 \
  --dummy-values \
  --verbose
```

## Operational Procedures

### Starting the MEV Bot

```bash
# Start with full monitoring
./target/release/mev-bot start \
  --config config/production.yaml \
  --log-level info \
  --metrics-port 9090 \
  --health-check-port 8080
```

### Monitoring Commands

```bash
# Check bot status
curl http://localhost:8080/health

# View metrics
curl http://localhost:9090/metrics

# Check active bundles
./target/release/mev-bot status --bundles

# View recent performance
./target/release/mev-bot stats --last 1h
```

### Emergency Procedures

#### Immediate Kill Switch Activation

```bash
# Activate kill switch immediately
./target/release/mev-bot kill-switch activate \
  --reason "Emergency shutdown" \
  --operator $(whoami)

# Verify all operations stopped
./target/release/mev-bot status --verify-stopped
```

#### Partial Shutdown (Specific Strategy)

```bash
# Disable specific strategy
./target/release/mev-bot disable-strategy \
  --strategy arbitrage \
  --reason "Market conditions"

# List active strategies
./target/release/mev-bot list-strategies
```

#### Fund Emergency Withdrawal

```bash
# Withdraw all funds to safe address
./target/release/mev-bot emergency-withdraw \
  --to 0x742d35Cc6634C0532925a3b8D4C9db96C4b5Da5e \
  --confirm-address \
  --gas-price 100gwei
```

## Failure Response Procedures

### Nonce Collision

**Symptoms:** Transaction failures due to nonce conflicts

**Response:**
1. Check for concurrent operations
2. Reset nonce tracking: `./target/release/mev-bot reset-nonce`
3. Restart with fresh nonce state
4. Monitor for resolution

### Gas Price Issues

**Symptoms:** Transactions not being included, gas underpricing errors

**Response:**
1. Check current network gas prices
2. Adjust gas price multiplier: `./target/release/mev-bot set-gas-multiplier 1.5`
3. Monitor inclusion rates
4. Consider temporary strategy disable if costs too high

### Reorg Handling

**Symptoms:** Bundle inclusion in wrong blocks, reorg notifications

**Response:**
1. Verify reorg depth and impact
2. Check if rollback is needed: `./target/release/mev-bot check-rollback`
3. Execute rollback if necessary: `./target/release/mev-bot execute-rollback --bundle-id <id>`
4. Adjust confirmation requirements if frequent reorgs

### Network Connectivity Issues

**Symptoms:** RPC timeouts, connection errors

**Response:**
1. Check network connectivity: `ping rpc.hyperevm.org`
2. Verify RPC endpoint status
3. Switch to backup RPC if available: `./target/release/mev-bot set-rpc --url <backup-url>`
4. Restart with connection retry logic

## Performance Optimization

### Monitoring Key Metrics

- **Bundle Success Rate:** Target > 80%
- **Average Inclusion Latency:** Target < 15 seconds
- **Gas Efficiency:** Monitor gas used vs. profit
- **Opportunity Detection Rate:** Track vs. market activity

### Optimization Actions

```bash
# Adjust strategy parameters
./target/release/mev-bot tune-strategy \
  --strategy arbitrage \
  --min-profit 0.01ETH \
  --max-gas 300000

# Update gas price strategy
./target/release/mev-bot set-gas-strategy \
  --base-multiplier 1.1 \
  --priority-fee 2gwei \
  --max-fee 200gwei
```

## Maintenance Procedures

### Daily Checks

```bash
# Check system health
./scripts/daily-health-check.sh

# Review performance metrics
./scripts/performance-report.sh --yesterday

# Verify key security
./scripts/security-audit.sh --keys
```

### Weekly Maintenance

```bash
# Update gas price models
./target/release/mev-bot update-gas-model

# Clean old logs and data
./scripts/cleanup-old-data.sh --older-than 7d

# Backup configuration and keys
./scripts/backup-config.sh
```

### Monthly Reviews

- Review and update security checklist
- Analyze failure patterns and update handling
- Update operational procedures based on lessons learned
- Test disaster recovery procedures

## Security Incident Response

### Suspected Compromise

1. **Immediate Actions:**
   - Activate kill switch
   - Disconnect from network
   - Preserve logs and evidence

2. **Investigation:**
   - Review access logs
   - Check for unauthorized transactions
   - Analyze system integrity

3. **Recovery:**
   - Rotate all keys
   - Update security measures
   - Gradual service restoration

### Unauthorized Access

1. **Detection:**
   - Monitor for unusual activity
   - Check authentication logs
   - Verify transaction patterns

2. **Response:**
   - Lock affected accounts
   - Change all credentials
   - Notify relevant parties

## Contact Information

### Emergency Contacts

- **Primary Operator:** [Contact Info]
- **Security Team:** [Contact Info]
- **Infrastructure Team:** [Contact Info]

### Escalation Procedures

1. **Level 1:** Operational issues - Primary operator
2. **Level 2:** Security incidents - Security team
3. **Level 3:** Critical failures - All teams + management

## Recovery Procedures

### Service Recovery After Kill Switch

```bash
# 1. Verify issue resolution
./target/release/mev-bot verify-safe-conditions

# 2. Run system checks
./target/release/mev-bot system-check --comprehensive

# 3. Deactivate kill switch
./target/release/mev-bot kill-switch deactivate \
  --operator $(whoami) \
  --confirm-safe

# 4. Gradual restart
./target/release/mev-bot start \
  --safe-mode \
  --limited-strategies
```

### Data Recovery

```bash
# Restore from backup
./scripts/restore-backup.sh --date YYYY-MM-DD

# Verify data integrity
./target/release/mev-bot verify-data --comprehensive

# Reconcile transactions
./target/release/mev-bot reconcile --from-block <block>
```

## Appendix

### Configuration Templates

See `config/` directory for:
- `production.yaml` - Production configuration
- `staging.yaml` - Staging environment
- `development.yaml` - Development setup

### Log Analysis

```bash
# Common log analysis commands
grep "ERROR" /var/log/mev-bot/mev-bot.log | tail -20
grep "bundle_submitted" /var/log/mev-bot/mev-bot.log | wc -l
grep "kill_switch" /var/log/mev-bot/mev-bot.log
```

### Performance Baselines

- **Mainnet Inclusion Rate:** 85-95%
- **Average Gas Cost:** 150-300k gas per bundle
- **Profit Margins:** 0.01-0.1 ETH per successful bundle
- **System Latency:** < 100ms for opportunity detection

---

**Document Version:** 1.0  
**Last Updated:** [Current Date]  
**Next Review:** [Date + 1 month]