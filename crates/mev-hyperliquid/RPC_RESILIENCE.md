# RPC Error Handling and Resilience

This document describes the error handling and resilience features implemented in the HyperLiquid RPC service.

## Overview

The RPC service implements several resilience patterns to ensure the bot continues operating despite RPC failures:

1. **Retry Logic with Exponential Backoff**
2. **Circuit Breaker Pattern**
3. **Timeout Handling**
4. **Graceful Degradation**
5. **Comprehensive Error Logging**

## Features

### 1. Retry Logic with Exponential Backoff

All RPC operations are wrapped with retry logic that:
- Attempts the operation up to **3 times** (configurable via `MAX_RETRY_ATTEMPTS`)
- Uses exponential backoff between retries:
  - Initial backoff: **100ms**
  - Backoff doubles after each failure
  - Maximum backoff: **10 seconds**
- Logs each retry attempt with appropriate severity

**Example:**
```rust
// First attempt fails -> wait 100ms
// Second attempt fails -> wait 200ms
// Third attempt fails -> operation fails
```

### 2. Circuit Breaker Pattern

The circuit breaker prevents overwhelming a failing RPC service:

#### States
- **Closed**: Normal operation, all requests proceed
- **Open**: Service is failing, requests are blocked
- **Half-Open**: Testing if service has recovered

#### Behavior
- Opens after **5 consecutive failures** (configurable via `CIRCUIT_BREAKER_THRESHOLD`)
- Remains open for **30 seconds** cooldown period (configurable via `CIRCUIT_BREAKER_COOLDOWN_SECS`)
- After cooldown, transitions to half-open state
- Single success in half-open state closes the circuit
- Single failure in half-open state reopens the circuit

#### Benefits
- Prevents wasting resources on a failing service
- Allows service time to recover
- Automatically resumes when service is healthy

### 3. Timeout Handling

All RPC operations have configurable timeouts:
- Default timeout: **10 seconds** (configurable via `RpcConfig::timeout_secs`)
- Operations that exceed timeout are cancelled
- Timeout errors are logged and trigger retry logic
- Prevents indefinite hangs on unresponsive endpoints

**Configuration:**
```rust
let config = RpcConfig::new(rpc_url, polling_interval_ms)
    .with_timeout(30); // 30 second timeout
```

### 4. Graceful Degradation

The bot continues operating despite RPC failures:
- **Polling Loop**: Continues even if individual polls fail
- **Circuit Breaker**: Skips operations when circuit is open, resumes automatically
- **Error Isolation**: RPC failures don't crash the bot
- **State Updates**: WebSocket continues providing market data even if RPC fails

### 5. Comprehensive Error Logging

Errors are logged with appropriate severity levels:

#### Error Levels
- **ERROR**: Circuit breaker opens, all retries exhausted, critical failures
- **WARN**: Individual operation failures, retry attempts, circuit breaker warnings
- **DEBUG**: Retry attempts, backoff delays, operation details
- **INFO**: Successful operations after retries, circuit breaker state transitions

#### Example Log Output
```
WARN  Failed to poll blockchain state: Connection timeout. Consecutive failures: 1
WARN  Retrying RPC operation 'poll_blockchain_state' after 100ms backoff
WARN  Failed to poll blockchain state: Connection timeout. Consecutive failures: 2
ERROR Circuit breaker threshold reached (5 consecutive failures), opening circuit
WARN  Circuit breaker is open (5 consecutive failures). Skipping poll, waiting for cooldown.
INFO  Circuit breaker cooldown elapsed, transitioning to half-open state
INFO  Circuit breaker test successful, transitioning to closed state
```

## Implementation Details

### Core Components

#### `CircuitBreaker` Struct
```rust
struct CircuitBreaker {
    state: Arc<AtomicU64>,              // Current state (Closed/Open/HalfOpen)
    consecutive_failures: Arc<AtomicU64>, // Failure counter
    opened_at: Arc<AtomicU64>,          // Timestamp when opened
}
```

#### `execute_with_retry` Method
Wraps all RPC operations with:
- Circuit breaker check
- Retry loop with exponential backoff
- Timeout handling
- Error logging
- Success/failure recording

### Protected Operations

The following RPC operations are protected by retry logic and circuit breaker:

1. **`poll_blockchain_state()`**
   - Queries current block number and timestamp
   - Emits state update events
   - Used by continuous polling loop

2. **`submit_transaction()`**
   - Signs and submits transactions
   - Handles nonce, gas estimation, gas price
   - Returns transaction hash

3. **`wait_for_confirmation()`**
   - Polls for transaction receipt
   - Tracks confirmation status
   - Has its own timeout mechanism

## Configuration

### Constants (in `rpc_service.rs`)
```rust
const MAX_RETRY_ATTEMPTS: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 100;
const MAX_BACKOFF_MS: u64 = 10_000;
const CIRCUIT_BREAKER_THRESHOLD: u64 = 5;
const CIRCUIT_BREAKER_COOLDOWN_SECS: u64 = 30;
```

### Runtime Configuration
```rust
let config = RpcConfig::new(rpc_url, polling_interval_ms)
    .with_timeout(30)  // Set custom timeout
    .with_private_key(private_key);  // Optional for transaction signing
```

## Monitoring

### Circuit Breaker Status
```rust
let (state, failures) = service.get_circuit_breaker_status();
// state: "closed", "open", or "half-open"
// failures: number of consecutive failures
```

### Recommended Metrics (to be implemented in task 7)
- `hyperliquid_rpc_calls_total{method, status}` - Total RPC calls
- `hyperliquid_rpc_call_duration_seconds{method}` - Call latency
- `hyperliquid_rpc_errors_total{method, error_type}` - Error counts
- `hyperliquid_circuit_breaker_state{state}` - Circuit breaker state
- `hyperliquid_circuit_breaker_failures` - Consecutive failure count

## Testing

The implementation includes comprehensive tests:

### Circuit Breaker Tests
- Initial state verification
- Success recording
- Failure recording
- Threshold triggering
- State transitions
- Reset behavior

### Retry Logic Tests
- Success on first attempt
- Success after failures
- All attempts fail
- Timeout handling
- Circuit breaker blocking

### Integration Tests
- Invalid URL handling
- Connection failures
- Transaction submission without private key
- Confirmation timeout

## Usage Example

```rust
use mev_hyperliquid::rpc_service::{HyperLiquidRpcService, RpcConfig};

// Create configuration
let config = RpcConfig::new(
    "https://rpc.hyperliquid.xyz/evm".to_string(),
    1000,  // Poll every 1 second
)
.with_timeout(30)  // 30 second timeout
.with_private_key(private_key);

// Create service
let (state_tx, state_rx) = mpsc::unbounded_channel();
let mut service = HyperLiquidRpcService::new(config, Some(state_tx));

// Connect
service.connect().await?;

// Start polling (runs indefinitely with error handling)
tokio::spawn(async move {
    service.start_polling().await
});

// Service will:
// - Retry failed operations automatically
// - Open circuit breaker after 5 consecutive failures
// - Resume automatically after 30 second cooldown
// - Continue operating despite RPC failures
// - Log all errors with appropriate severity
```

## Benefits

1. **Reliability**: Bot continues operating despite RPC failures
2. **Performance**: Exponential backoff prevents overwhelming failing services
3. **Observability**: Comprehensive logging helps diagnose issues
4. **Automatic Recovery**: Circuit breaker automatically resumes when service recovers
5. **Resource Efficiency**: Circuit breaker prevents wasting resources on failing operations
6. **Maintainability**: Centralized error handling logic

## Future Enhancements

Potential improvements for future tasks:
1. Add metrics for circuit breaker state and failure counts
2. Make retry parameters configurable via config file
3. Implement RPC endpoint failover (multiple RPC URLs)
4. Add adaptive timeout based on historical latency
5. Implement request prioritization (critical vs. non-critical operations)
