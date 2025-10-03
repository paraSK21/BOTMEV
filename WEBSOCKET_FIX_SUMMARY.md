# WebSocket Subscription Fix - Implementation Summary

## Problem
The MEV bot was continuously failing to subscribe to Hyperliquid's WebSocket endpoint with the error:
```
"No response received for subscription request, restarting in 5 seconds..."
```

This occurred because `wss://api.hyperliquid.xyz/ws` is Hyperliquid's general API endpoint, not an Ethereum-compatible WebSocket that supports `eth_subscribe`.

## Solution Implemented: Hybrid Mode (Option 3)

### What Changed

#### 1. **New Mempool Modes**
Added three operational modes:
- **Auto** (default): Try WebSocket first, automatically fallback to HTTP polling if it fails
- **WebSocket**: WebSocket-only mode with retry loop
- **Polling**: HTTP polling-only mode

#### 2. **HTTP Polling Fallback**
When WebSocket fails, the bot now:
- Polls `eth_getBlockByNumber("pending", true)` every 300ms (configurable)
- Deduplicates transactions using a hash set (keeps last 10k hashes)
- Processes new transactions through the same pipeline

#### 3. **WebSocket Timeout**
Added 3-second timeout for subscription responses to fail fast instead of hanging indefinitely.

#### 4. **Configuration Updates**
New config options in `config/my-config.yml`:
```yaml
network:
  mempool_mode: "auto"  # auto, websocket, polling
  polling_interval_ms: 300
  websocket_timeout_secs: 3
```

### Files Modified

1. **crates/mev-mempool/src/lib.rs**
   - Added `MempoolMode` enum
   - Added `rpc_url` and mode configuration to `MempoolService`
   - Implemented `start_polling_mode()` with HTTP polling
   - Implemented `poll_pending_transactions()` with deduplication
   - Added `try_websocket_mode()` for auto fallback

2. **crates/mev-mempool/src/websocket.rs**
   - Added timeout parameter to `send_subscribe_request()`
   - Wrapped subscription wait in `tokio::time::timeout()`

3. **crates/mev-bot/src/main.rs**
   - Updated `MempoolService` initialization to pass RPC URL
   - Added mode parsing from config
   - Configured polling interval and timeout

4. **crates/mev-config/src/lib.rs**
   - Added `mempool_mode`, `polling_interval_ms`, `websocket_timeout_secs` to `NetworkConfig`

5. **config/my-config.yml**
   - Added new mempool configuration options

## How to Run

### Option 1: Auto Mode (Recommended)
The bot will try WebSocket first, then automatically fall back to polling:
```bash
cargo run --release --bin mev-bot -- -c config/my-config.yml
```

### Option 2: Force Polling Mode
If you want to skip WebSocket entirely:
```yaml
# In config/my-config.yml
network:
  mempool_mode: "polling"
```

### Option 3: Try QuickNode WebSocket
Your QuickNode endpoint might support proper EVM WebSocket:
```yaml
# In config/my-config.yml - uncomment QuickNode URLs
network:
  rpc_url: "https://cosmopolitan-quaint-voice.hype-mainnet.quiknode.pro/.../evm"
  ws_url: "wss://cosmopolitan-quaint-voice.hype-mainnet.quiknode.pro/.../evm"
  mempool_mode: "websocket"  # Try WebSocket-only mode
```

## Expected Behavior

### With Auto Mode:
```
INFO Starting mempool service with mode: Auto
INFO Auto mode: Attempting WebSocket first...
INFO Connecting to WebSocket: wss://api.hyperliquid.xyz/ws
INFO WebSocket connected successfully
ERROR Timeout waiting for subscription response after 3 seconds
WARN WebSocket mode failed: Timeout waiting for subscription response
INFO Falling back to HTTP polling mode
INFO Starting HTTP polling mode on https://api.hyperliquid.xyz/evm
INFO Polling interval: 300ms
INFO Polled 15 pending transactions
```

### With Polling Mode:
```
INFO Starting mempool service with mode: Polling
INFO Polling-only mode
INFO Starting HTTP polling mode on https://api.hyperliquid.xyz/evm
INFO Polling interval: 300ms
INFO Polled 23 pending transactions
```

## Performance Considerations

### HTTP Polling Mode
- **Latency**: ~300-500ms detection latency (vs <20ms target for WebSocket)
- **Rate Limits**: Be mindful of RPC rate limits with frequent polling
- **Bandwidth**: Higher bandwidth usage than WebSocket subscriptions
- **Reliability**: More reliable for Hyperliquid's current infrastructure

### Recommendations
1. Start with **auto mode** to see which method works
2. If WebSocket never succeeds, switch to **polling mode** permanently
3. Adjust `polling_interval_ms` based on:
   - Network conditions (lower = more load)
   - Rate limits from your RPC provider
   - Acceptable latency for your strategy

## Next Steps

1. **Test the bot** with auto mode to confirm it works
2. **Monitor logs** to see if it uses WebSocket or falls back to polling
3. **Adjust polling interval** if needed based on transaction volume
4. **Consider QuickNode** if you need lower latency and their WebSocket works
5. **Monitor metrics** via Prometheus to track detection latency

## Troubleshooting

### If polling also fails:
- Check RPC endpoint is accessible: `curl https://api.hyperliquid.xyz/evm`
- Verify network connectivity
- Check for rate limiting (429 errors)

### If you want to test WebSocket with QuickNode:
- Uncomment QuickNode URLs in config
- Set `mempool_mode: "websocket"`
- Check QuickNode dashboard for connection status

### If latency is too high:
- Reduce `polling_interval_ms` (e.g., 100-200ms)
- Consider upgrading to premium RPC provider
- Use multiple RPC endpoints for redundancy
