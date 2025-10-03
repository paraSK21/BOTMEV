use anyhow::Result;
use mev_mempool::{TransactionParser, TransactionRingBuffer, AbiDecoder};
use serde_json::json;
use tracing::{info, Level};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("MEV Mempool Demo - Processing Sample Transactions");

    // Create components
    let parser = TransactionParser::new();
    let ring_buffer = TransactionRingBuffer::new(100);
    let abi_decoder = AbiDecoder::new();

    // Sample transaction data (Uniswap V2 swap)
    let sample_transactions = vec![
        json!({
            "hash": "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
            "from": "0x742d35cc6634c0532925a3b8d4c9db7c4c4c4c4c",
            "to": "0x7a250d5630b4cf539739df2c5dacb4c659f2488d", // Uniswap V2 Router
            "value": "0x0",
            "gasPrice": "0x4a817c800", // 20 gwei
            "gas": "0x33450", // 210000
            "nonce": "0x42",
            "input": "0x38ed1739000000000000000000000000000000000000000000000000016345785d8a0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000742d35cc6634c0532925a3b8d4c9db7c4c4c4c4c000000000000000000000000000000000000000000000000000000006507c5a0"
        }),
        json!({
            "hash": "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
            "from": "0x456def789abc123456def789abc123456def789a",
            "to": "0xa0b86a33e6c3b4c6e6b8b8b8b8b8b8b8b8b8b8b8", // Random contract
            "value": "0x1000000000000000000", // 1 ETH
            "gasPrice": "0x77359400", // 2 gwei
            "gas": "0x5208", // 21000
            "nonce": "0x1",
            "input": "0xa9059cbb000000000000000000000000742d35cc6634c0532925a3b8d4c9db7c4c4c4c4c0000000000000000000000000000000000000000000000000de0b6b3a7640000"
        }),
        json!({
            "hash": "0xfedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321",
            "from": "0x789abc123def456789abc123def456789abc123d",
            "to": "0xe592427a0aece92de3edee1f18e0157c05861564", // Uniswap V3 Router
            "value": "0x0",
            "gasPrice": "0x9502f9000", // 40 gwei
            "gas": "0x493e0", // 300000
            "nonce": "0x7",
            "input": "0x414bf389000000000000000000000000a0b86a33e6c3b4c6e6b8b8b8b8b8b8b8b8b8b8b8000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc20000000000000000000000000000000000000000000000000000000000000bb8"
        })
    ];

    info!("Processing {} sample transactions...", sample_transactions.len());

    for (i, raw_tx) in sample_transactions.iter().enumerate() {
        info!("--- Transaction {} ---", i + 1);
        
        // Parse transaction
        match parser.parse_transaction(raw_tx) {
            Ok(parsed_tx) => {
                // Log parsed transaction details
                info!(
                    "Hash: {} | From: {} | To: {:?}",
                    parsed_tx.transaction.hash,
                    parsed_tx.transaction.from,
                    parsed_tx.transaction.to
                );
                
                info!(
                    "Target Type: {:?} | Processing Time: {}ms",
                    parsed_tx.target_type,
                    parsed_tx.processing_time_ms
                );

                if let Some(ref decoded) = parsed_tx.decoded_input {
                    info!(
                        "Function: {} | Signature: {} | Params: {}",
                        decoded.function_name,
                        decoded.function_signature,
                        decoded.parameters.len()
                    );
                }

                // Check if interesting for MEV
                let is_interesting = parser.is_interesting_transaction(&parsed_tx);
                info!("MEV Interesting: {}", is_interesting);

                // Push to ring buffer
                if ring_buffer.push(parsed_tx) {
                    info!("✅ Added to ring buffer");
                } else {
                    info!("❌ Ring buffer full");
                }
            }
            Err(e) => {
                info!("❌ Failed to parse transaction: {}", e);
            }
        }
        
        info!("");
    }

    // Show buffer statistics
    info!("=== Ring Buffer Statistics ===");
    info!("Length: {}/{}", ring_buffer.len(), ring_buffer.capacity());
    info!("Utilization: {:.1}%", ring_buffer.utilization());
    info!("Dropped: {}", ring_buffer.dropped_count());

    // Demonstrate ABI decoder directly
    info!("=== ABI Decoder Demo ===");
    let sample_input = "0x38ed1739000000000000000000000000000000000000000000000000016345785d8a0000";
    match abi_decoder.decode_input(sample_input, Some("0x7a250d5630b4cf539739df2c5dacb4c659f2488d")) {
        Ok(Some(decoded)) => {
            info!("Decoded function: {} ({})", decoded.function_name, decoded.function_signature);
            for param in decoded.parameters {
                info!("  {}: {} = {}", param.name, param.param_type, param.value);
            }
        }
        Ok(None) => info!("Could not decode input"),
        Err(e) => info!("Decode error: {}", e),
    }

    info!("Demo completed successfully!");
    Ok(())
}
