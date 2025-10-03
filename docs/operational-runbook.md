# MEV Bot Operational Runbook

This runbook provides step-by-step procedures for operating, monitoring, and troubleshooting the MEV bot in production environments.

## Table of Contents

- [Daily Operations](#daily-operations)
- [Monitoring and Alerting](#monitoring-and-alerting)
- [Incident Response](#incident-response)
- [Maintenance Procedures](#maintenance-procedures)
- [Performance Tuning](#performance-tuning)
- [Security Operations](#security-operations)
- [Backup and Recovery](#backup-and-recovery)
- [Troubleshooting Guide](#troubleshooting-guide)
- [Emergency Procedures](#emergency-procedures)

## Daily Operations

### Morning Health Check (9:00 AM UTC)

**Checklist:**
- [ ] Check system status dashboard
- [ ] Review overnight alerts and incidents
- [ ] Validate key performance metrics
- [ ] Check resource utilization
- [ ] Review profit/loss summary
- [ ] Verify backup completion

**Commands:**
```bash
# System health check
curl -f http://mev-bot:8080/health
curl -f http://mev-bot:9090/metrics | grep mev_bot_status

# Check container status
docker ps | grep mev-bot
docker stats mev-bot-production --no-stream

# Review logs for errors
docker logs mev-bot-production --since 24h | grep -i error | wc -l
```

**Expected Values:**
- Health endpoint: HTTP 200
- Detection latency p95: <50ms
- Simulation latency p95: <100ms
- Memory usage: <80%
- CPU usage: <70%
- Error rate: <1%

### Evening Review (6:00 PM UTC)

**Checklist:**
- [ ] Review daily performance metrics
- [ ] Check profit/loss against targets
- [ ] Analyze strategy performance
- [ ] Review any incidents or alerts
- [ ] Plan maintenance activities
- [ ] Update operational log

**Performance Review:**
```bash
# Daily metrics summary
curl -s http://mev-bot:9090/metrics | grep -E "(mev_bot_profit|mev_bot_bundle_success|mev_bot_detection_latency)"

# Strategy performance
curl -s http://mev-bot:9090/metrics | grep mev_bot_strategy_success_rate

# Resource utilization trends
docker stats mev-bot-production --no-stream
```

## Monitoring and Alerting

### Key Metrics to Monitor

#### Performance Metrics
- **Detection Latency**: `mev_bot_detection_latency_seconds`
  - Target: p50 <20ms, p95 <50ms
  - Critical: p95 >100ms
- **Simulation Latency**: `mev_bot_simulation_latency_seconds`
  - Target: p50 <50ms, p95 <100ms
  - Critical: p95 >200ms
- **Throughput**: `mev_bot_transactions_processed_per_second`
  - Target: >100 TPS
  - Critical: <50 TPS

#### Business Metrics
- **Bundle Success Rate**: `mev_bot_bundle_success_rate`
  - Target: >80%
  - Critical: <60%
- **Profit Rate**: `mev_bot_profit_eth_per_hour`
  - Target: >0.1 ETH/hour
  - Warning: <0.05 ETH/hour
- **Gas Efficiency**: `mev_bot_gas_efficiency_ratio`
  - Target: >0.8
  - Warning: <0.6

#### System Metrics
- **Memory Usage**: `container_memory_usage_bytes`
  - Warning: >80%
  - Critical: >90%
- **CPU Usage**: `container_cpu_usage_percent`
  - Warning: >80%
  - Critical: >95%
- **Disk Usage**: `container_fs_usage_bytes`
  - Warning: >80%
  - Critical: >90%

### Alert Response Procedures

#### High Priority Alerts

**Alert: MEV Bot Down**
```bash
# 1. Check container status
docker ps -a | grep mev-bot

# 2. Check logs for crash reason
docker logs mev-bot-production --tail 100

# 3. Attempt restart
docker restart mev-bot-production

# 4. If restart fails, check configuration
docker exec mev-bot-production /usr/local/bin/mev-bot --validate-config

# 5. Escalate if issue persists >5 minutes
```

**Alert: High Detection Latency**
```bash
# 1. Check system resources
docker stats mev-bot-production --no-stream
htop

# 2. Check network connectivity
ping rpc.hyperevm.org
curl -w "@curl-format.txt" -s -o /dev/null https://rpc.hyperevm.org

# 3. Review RPC provider status
curl -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  https://rpc.hyperevm.org

# 4. Consider switching to backup RPC if needed
```

**Alert: Low Bundle Success Rate**
```bash
# 1. Check recent bundle failures
docker logs mev-bot-production | grep "bundle.*failed" | tail -20

# 2. Check gas price settings
curl -s http://mev-bot:9090/metrics | grep mev_bot_gas_price

# 3. Review strategy configuration
docker exec mev-bot-production cat /app/config/production.yaml | grep -A 10 strategies

# 4. Check mempool congestion
curl -s http://mev-bot:9090/metrics | grep mev_bot_mempool_size
```

#### Medium Priority Alerts

**Alert: High Memory Usage**
```bash
# 1. Check memory breakdown
docker exec mev-bot-production cat /proc/meminfo

# 2. Look for memory leaks
docker exec mev-bot-production ps aux --sort=-%mem | head -10

# 3. Consider restarting if usage >90%
docker restart mev-bot-production

# 4. Monitor post-restart
watch "docker stats mev-bot-production --no-stream"
```

**Alert: RPC Rate Limiting**
```bash
# 1. Check RPC response codes
docker logs mev-bot-production | grep -i "rate limit\|429\|too many requests"

# 2. Review RPC usage patterns
curl -s http://mev-bot:9090/metrics | grep mev_bot_rpc_requests_per_second

# 3. Implement backoff if needed
docker exec mev-bot-production /usr/local/bin/mev-bot --adjust-rpc-rate 0.8

# 4. Consider switching RPC providers
```

## Incident Response

### Incident Classification

**Severity 1 (Critical)**
- MEV bot completely down
- Data corruption or loss
- Security breach
- Financial loss >$10,000/hour

**Severity 2 (High)**
- Significant performance degradation
- Partial functionality loss
- Financial loss $1,000-$10,000/hour

**Severity 3 (Medium)**
- Minor performance issues
- Non-critical feature failures
- Financial loss <$1,000/hour

### Incident Response Process

#### 1. Detection and Assessment (0-5 minutes)
```bash
# Immediate assessment
curl -f http://mev-bot:8080/health
docker ps | grep mev-bot
docker logs mev-bot-production --tail 50

# Check key metrics
curl -s http://mev-bot:9090/metrics | grep -E "(mev_bot_status|mev_bot_error_rate)"

# Determine severity level
```

#### 2. Initial Response (5-15 minutes)
```bash
# For Severity 1: Immediate containment
docker stop mev-bot-production  # If security breach suspected

# For Severity 2/3: Gather more information
docker logs mev-bot-production --since 1h > incident-logs.txt
curl -s http://mev-bot:9090/metrics > incident-metrics.txt

# Notify stakeholders
echo "Incident detected: [DESCRIPTION]" | mail -s "MEV Bot Incident" ops-team@company.com
```

#### 3. Investigation and Diagnosis (15-60 minutes)
```bash
# Detailed log analysis
grep -E "(ERROR|FATAL|PANIC)" incident-logs.txt

# System resource analysis
docker stats mev-bot-production --no-stream
df -h
free -m

# Network connectivity tests
ping -c 5 rpc.hyperevm.org
traceroute rpc.hyperevm.org

# Configuration validation
docker exec mev-bot-production /usr/local/bin/mev-bot --validate-config --verbose
```

#### 4. Resolution (Variable)
```bash
# Common resolution steps

# Restart service
docker restart mev-bot-production

# Rollback to previous version
docker stop mev-bot-production
docker rename mev-bot-production mev-bot-production-failed
docker rename mev-bot-production-backup mev-bot-production
docker start mev-bot-production

# Configuration fix
docker exec mev-bot-production vi /app/config/production.yaml
docker restart mev-bot-production

# Emergency shutdown
docker exec mev-bot-production /usr/local/bin/mev-bot --emergency-stop
```

#### 5. Recovery Validation (15-30 minutes)
```bash
# Validate system recovery
./scripts/validate-deployment.bat production

# Monitor for stability
for i in {1..10}; do
  curl -f http://mev-bot:8080/health
  sleep 60
done

# Performance validation
curl -s http://mev-bot:9090/metrics | grep mev_bot_detection_latency
```

#### 6. Post-Incident Review
- Document timeline and root cause
- Update runbooks and procedures
- Implement preventive measures
- Communicate lessons learned

## Maintenance Procedures

### Weekly Maintenance (Sundays 2:00 AM UTC)

**Checklist:**
- [ ] Update system packages
- [ ] Rotate log files
- [ ] Clean up old Docker images
- [ ] Backup configuration and keys
- [ ] Review and update monitoring rules
- [ ] Performance optimization review

**Commands:**
```bash
# System updates
sudo apt update && sudo apt upgrade -y

# Docker cleanup
docker system prune -f
docker image prune -a -f

# Log rotation
docker exec mev-bot-production logrotate /etc/logrotate.conf

# Backup
tar -czf /backups/mev-bot-config-$(date +%Y%m%d).tar.gz config/
cp /secure/keys/production.key /backups/production-key-$(date +%Y%m%d).key
```

### Monthly Maintenance (First Sunday of month)

**Checklist:**
- [ ] Security audit and key rotation
- [ ] Performance benchmark comparison
- [ ] Dependency updates
- [ ] Disaster recovery test
- [ ] Documentation updates
- [ ] Team training review

**Commands:**
```bash
# Security audit
cargo audit
docker scan mev-bot:production

# Performance benchmarking
cargo bench --workspace > benchmarks-$(date +%Y%m%d).txt

# Dependency updates
cargo update
cargo outdated
```

### Quarterly Maintenance

**Checklist:**
- [ ] Full system backup and restore test
- [ ] Penetration testing
- [ ] Business continuity plan review
- [ ] Performance capacity planning
- [ ] Technology stack review
- [ ] Compliance audit

## Performance Tuning

### CPU Optimization

**Check CPU usage patterns:**
```bash
# Monitor CPU usage by core
htop
mpstat -P ALL 1 10

# Check CPU pinning effectiveness
docker exec mev-bot-production cat /proc/self/status | grep Cpus_allowed_list

# Profile CPU usage
docker exec mev-bot-production perf top -p $(pgrep mev-bot)
```

**Optimization steps:**
```bash
# Adjust CPU pinning
docker update --cpuset-cpus="0-7" mev-bot-production

# Increase CPU priority
docker update --cpu-shares=1024 mev-bot-production

# Enable CPU governor performance mode
echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor
```

### Memory Optimization

**Monitor memory usage:**
```bash
# Detailed memory analysis
docker exec mev-bot-production cat /proc/meminfo
docker exec mev-bot-production pmap $(pgrep mev-bot)

# Check for memory leaks
valgrind --tool=memcheck --leak-check=full docker exec mev-bot-production /usr/local/bin/mev-bot --dry-run --duration 60s
```

**Optimization steps:**
```bash
# Adjust memory limits
docker update --memory=32g --memory-swap=32g mev-bot-production

# Configure swap behavior
echo 1 | sudo tee /proc/sys/vm/swappiness

# Enable memory compression
echo 1 | sudo tee /sys/module/zswap/parameters/enabled
```

### Network Optimization

**Monitor network performance:**
```bash
# Check network latency to RPC
ping -c 100 rpc.hyperevm.org | tail -1

# Monitor network throughput
iftop -i eth0

# Check connection pool usage
netstat -an | grep :443 | wc -l
```

**Optimization steps:**
```bash
# Increase connection limits
echo 'net.core.somaxconn = 65535' | sudo tee -a /etc/sysctl.conf
echo 'net.ipv4.tcp_max_syn_backlog = 65535' | sudo tee -a /etc/sysctl.conf
sudo sysctl -p

# Optimize TCP settings
echo 'net.ipv4.tcp_congestion_control = bbr' | sudo tee -a /etc/sysctl.conf
echo 'net.core.default_qdisc = fq' | sudo tee -a /etc/sysctl.conf
```

## Security Operations

### Daily Security Checks

**Checklist:**
- [ ] Review authentication logs
- [ ] Check for unauthorized access attempts
- [ ] Validate key file permissions
- [ ] Monitor network connections
- [ ] Review configuration changes

**Commands:**
```bash
# Check authentication logs
sudo grep "authentication failure" /var/log/auth.log | tail -20

# Monitor active connections
netstat -an | grep :8080
netstat -an | grep :9090

# Validate key permissions
ls -la /secure/keys/
stat /secure/keys/production.key

# Check for configuration changes
git log --oneline --since="24 hours ago" config/
```

### Security Incident Response

**Suspected Breach:**
```bash
# 1. Immediate isolation
docker network disconnect bridge mev-bot-production

# 2. Preserve evidence
docker logs mev-bot-production > security-incident-$(date +%Y%m%d-%H%M%S).log
docker exec mev-bot-production netstat -an > network-connections.log

# 3. Rotate keys immediately
./scripts/rotate-keys.sh emergency

# 4. Notify security team
echo "SECURITY INCIDENT: Suspected breach detected" | mail -s "URGENT: MEV Bot Security Incident" security@company.com
```

**Key Compromise:**
```bash
# 1. Stop all operations immediately
docker exec mev-bot-production /usr/local/bin/mev-bot --emergency-stop

# 2. Generate new keys
openssl rand -hex 32 > /tmp/new-production.key

# 3. Update configuration
docker cp /tmp/new-production.key mev-bot-production:/app/keys/production.key

# 4. Restart with new keys
docker restart mev-bot-production

# 5. Monitor for unauthorized transactions
```

## Backup and Recovery

### Automated Backup Procedures

**Daily Backups (3:00 AM UTC):**
```bash
#!/bin/bash
# /scripts/daily-backup.sh

DATE=$(date +%Y%m%d)
BACKUP_DIR="/backups/daily"

# Configuration backup
tar -czf "$BACKUP_DIR/config-$DATE.tar.gz" config/

# Database backup
docker exec mev-bot-production sqlite3 /data/mev-bot.db ".backup /tmp/backup.db"
docker cp mev-bot-production:/tmp/backup.db "$BACKUP_DIR/database-$DATE.db"

# Key backup (encrypted)
gpg --symmetric --cipher-algo AES256 --output "$BACKUP_DIR/keys-$DATE.gpg" /secure/keys/production.key

# Cleanup old backups (keep 30 days)
find "$BACKUP_DIR" -name "*.tar.gz" -mtime +30 -delete
find "$BACKUP_DIR" -name "*.db" -mtime +30 -delete
find "$BACKUP_DIR" -name "*.gpg" -mtime +30 -delete
```

### Recovery Procedures

**Configuration Recovery:**
```bash
# 1. Stop current instance
docker stop mev-bot-production

# 2. Restore configuration
tar -xzf /backups/daily/config-20231201.tar.gz

# 3. Validate configuration
docker run --rm -v $(pwd)/config:/app/config mev-bot:production /usr/local/bin/mev-bot --validate-config

# 4. Restart service
docker start mev-bot-production
```

**Database Recovery:**
```bash
# 1. Stop service
docker stop mev-bot-production

# 2. Restore database
docker cp /backups/daily/database-20231201.db mev-bot-production:/data/mev-bot.db

# 3. Verify database integrity
docker exec mev-bot-production sqlite3 /data/mev-bot.db "PRAGMA integrity_check;"

# 4. Restart service
docker start mev-bot-production
```

**Complete System Recovery:**
```bash
# 1. Rebuild from scratch
docker pull mev-bot:production
docker stop mev-bot-production
docker rm mev-bot-production

# 2. Restore all components
tar -xzf /backups/daily/config-20231201.tar.gz
gpg --decrypt /backups/daily/keys-20231201.gpg > /secure/keys/production.key

# 3. Deploy fresh instance
docker run -d \
  --name mev-bot-production \
  --restart unless-stopped \
  -p 8080:8080 -p 9090:9090 \
  -v $(pwd)/config:/app/config:ro \
  -v /secure/keys:/app/keys:ro \
  mev-bot:production

# 4. Restore database
docker cp /backups/daily/database-20231201.db mev-bot-production:/data/mev-bot.db

# 5. Validate recovery
./scripts/validate-deployment.bat production
```

## Troubleshooting Guide

### Common Issues and Solutions

#### Issue: High Detection Latency

**Symptoms:**
- Detection latency >50ms p95
- Slow response to mempool transactions
- Missed MEV opportunities

**Diagnosis:**
```bash
# Check system resources
htop
iostat -x 1 5

# Check network latency
ping -c 10 rpc.hyperevm.org

# Check RPC response times
curl -w "@curl-format.txt" -s -o /dev/null -X POST \
  -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' \
  https://rpc.hyperevm.org
```

**Solutions:**
```bash
# 1. Optimize CPU pinning
docker update --cpuset-cpus="0-3" mev-bot-production

# 2. Increase RPC connection pool
docker exec mev-bot-production /usr/local/bin/mev-bot --adjust-rpc-pool-size 50

# 3. Switch to faster RPC provider
# Update config/production.yaml with new RPC URL

# 4. Reduce simulation concurrency if CPU bound
# Update performance.simulation_concurrency in config
```

#### Issue: Memory Leaks

**Symptoms:**
- Gradually increasing memory usage
- Out of memory errors
- Container restarts

**Diagnosis:**
```bash
# Monitor memory growth
watch "docker stats mev-bot-production --no-stream | grep memory"

# Check for memory leaks
docker exec mev-bot-production cat /proc/$(pgrep mev-bot)/status | grep VmRSS

# Profile memory usage
docker exec mev-bot-production valgrind --tool=massif /usr/local/bin/mev-bot --dry-run --duration 300s
```

**Solutions:**
```bash
# 1. Restart service to clear memory
docker restart mev-bot-production

# 2. Reduce buffer sizes
# Update mempool.max_pending_transactions in config

# 3. Enable memory limits
docker update --memory=16g --oom-kill-disable=false mev-bot-production

# 4. Schedule periodic restarts
echo "0 6 * * * docker restart mev-bot-production" | crontab -
```

#### Issue: Bundle Submission Failures

**Symptoms:**
- Low bundle success rate
- "Transaction underpriced" errors
- "Nonce too low" errors

**Diagnosis:**
```bash
# Check recent bundle failures
docker logs mev-bot-production | grep -i "bundle.*fail" | tail -20

# Check gas price trends
curl -s http://mev-bot:9090/metrics | grep mev_bot_gas_price

# Check nonce management
docker logs mev-bot-production | grep -i nonce | tail -10
```

**Solutions:**
```bash
# 1. Increase gas price buffer
# Update strategies.*.max_gas_price in config

# 2. Improve nonce management
docker exec mev-bot-production /usr/local/bin/mev-bot --reset-nonce

# 3. Check network congestion
curl -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_gasPrice","params":[],"id":1}' \
  https://rpc.hyperevm.org

# 4. Adjust submission timing
# Update performance.decision_timeout_ms in config
```

## Emergency Procedures

### Emergency Shutdown

**When to use:**
- Security breach detected
- Significant financial losses
- System instability
- Regulatory concerns

**Procedure:**
```bash
# 1. Immediate stop (graceful)
docker exec mev-bot-production /usr/local/bin/mev-bot --shutdown-graceful

# 2. Force stop if needed
docker kill mev-bot-production

# 3. Prevent restart
docker update --restart=no mev-bot-production

# 4. Secure the system
docker network disconnect bridge mev-bot-production

# 5. Notify stakeholders
echo "EMERGENCY SHUTDOWN: MEV Bot stopped due to [REASON]" | \
  mail -s "URGENT: MEV Bot Emergency Shutdown" \
  ops-team@company.com,management@company.com
```

### Emergency Key Rotation

**When to use:**
- Suspected key compromise
- Unauthorized transactions detected
- Security audit findings

**Procedure:**
```bash
# 1. Generate new key immediately
openssl rand -hex 32 > /tmp/emergency-key.txt

# 2. Stop current operations
docker exec mev-bot-production /usr/local/bin/mev-bot --emergency-stop

# 3. Replace key
docker cp /tmp/emergency-key.txt mev-bot-production:/app/keys/production.key

# 4. Update key permissions
docker exec mev-bot-production chmod 600 /app/keys/production.key

# 5. Restart with new key
docker restart mev-bot-production

# 6. Validate new key is working
./scripts/validate-deployment.bat production

# 7. Secure old key
shred -vfz /secure/keys/production.key.old
```

### Disaster Recovery

**Complete System Failure:**
```bash
# 1. Assess damage
docker ps -a
docker images
df -h

# 2. Restore from backup
./scripts/disaster-recovery.sh

# 3. Validate restoration
./scripts/validate-deployment.bat production

# 4. Resume operations gradually
docker exec mev-bot-production /usr/local/bin/mev-bot --dry-run --duration 300s
```

### Contact Information

**Emergency Contacts:**
- **On-call Engineer**: +1-555-0123 (24/7)
- **DevOps Lead**: +1-555-0124 (Business hours)
- **Security Team**: +1-555-0125 (24/7)
- **Management**: +1-555-0126 (Escalation)

**Communication Channels:**
- **Slack**: #mev-bot-ops (Real-time updates)
- **Email**: ops-team@company.com (Formal notifications)
- **PagerDuty**: MEV Bot Service (Automated alerts)

---

This operational runbook should be reviewed and updated monthly to ensure accuracy and completeness. All team members should be familiar with these procedures and practice them regularly during maintenance windows.