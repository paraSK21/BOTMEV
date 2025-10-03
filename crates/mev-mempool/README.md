# MEV Mempool Service

Day 2 implementation of the MEV bot mempool monitoring service.

## Features

- **WebSocket eth_subscribe**: Binary RPC client for pending transaction monitoring
- **Concurrent Transaction Parser**: Parses and decodes AMM/DEX function calls
- **Lock-free Ring Buffer**: Uses `crossbeam::ArrayQueue` for high-performance downstream processing
- **Structured JSON Logging**: Comprehensive logging with timestamps for each transaction
- **ABI Decoder**: Precompiled ABIs for major protocols (Uniswap V2/V3, SushiSwap, Curve, Balancer)

## Supported Protocols

- **Uniswap V2**: `swapExactTokensForTokens`, `swapExactETHForTokens`, `swapExactTokensForETH`
- **Uniswap V3**: `exactInputSingle`, `exactInput`
- **SushiSwap**: Similar to Uniswap V2 functions
- **Curve**: `exchange`, `exchange_underlying`
- **Balancer**: `swap`
- **ERC20**: `transfer`

## Usage

### Running the Mempool Service

```bash
# Set WebSocket endpoint (optional, defaults to ws://localhost:8546)
export WS_ENDPOINT="ws://your-ethereum-node:8546"

# Run the mempool service
cargo run --bin mempool-service
```

### Running the Consumer Example

```bash
# Run the consumer example that demonstrates downstream processing
cargo run --example consumer_example
```

## Configuration

The service can be configured via environment variables:

- `WS_ENDPOINT`: WebSocket endpoint for Ethereum node (default: `ws://localhost:8546`)

## Output

The service will log structured JSON for each interesting transaction:

```json
{
  "target": "mempool_transaction",
  "tx_hash": "0x123...",
  "from": "0x456...",
  "to": "0x789...",
  "value": "1000000000000000000",
  "gas_price": "20000000000",
  "gas_limit": "200000",
  "nonce": 42,
  "target_type": "UniswapV2",
  "function_name": "swapExactTokensForTokens",
  "function_signature": "0x38ed1739",
  "processing_time_ms": 2,
  "timestamp": "2024-01-01T12:00:00Z"
}
```

## Performance Metrics

The service provides real-time performance metrics:

- **TPS**: Transactions per second processed
- **Buffer Utilization**: Ring buffer usage percentage
- **Dropped Transactions**: Count of transactions dropped due to buffer overflow

## Architecture

```
WebSocket Client → Transaction Parser → ABI Decoder → Ring Buffer → Downstream Consumers
                                    ↓
                              Structured Logging
```

## Testing

```bash
# Run unit tests
cargo test

# Run with a local Ethereum node
# Make sure you have a WebSocket-enabled Ethereum node running on ws://localhost:8546
cargo run --bin mempool-service
```

## Dependencies

- `tokio-tungstenite`: WebSocket client
- `crossbeam`: Lock-free data structures
- `ethabi`: Ethereum ABI encoding/decoding
- `tracing`: Structured logging
- `serde_json`: JSON serialization
