use anyhow::{anyhow, Result};
use chrono::Utc;
use mev_core::{ParsedTransaction, Transaction, TargetType};
use serde_json::Value;
use std::time::Instant;
use tracing::{debug, error};

use crate::abi_decoder::AbiDecoder;

/// Concurrent transaction parser
pub struct TransactionParser {
    abi_decoder: AbiDecoder,
}

impl TransactionParser {
    pub fn new() -> Self {
        Self {
            abi_decoder: AbiDecoder::new(),
        }
    }

    /// Parse raw transaction data from WebSocket notification
    pub fn parse_transaction(&self, raw_tx: &Value) -> Result<ParsedTransaction> {
        let start_time = Instant::now();
        
        // Extract transaction fields
        let transaction = self.extract_transaction_fields(raw_tx)?;
        
        // Decode input data if present
        let decoded_input = if !transaction.input.is_empty() && transaction.input != "0x" {
            match self.abi_decoder.decode_input(&transaction.input, transaction.to.as_deref()) {
                Ok(decoded) => decoded,
                Err(e) => {
                    debug!("Failed to decode input for tx {}: {}", transaction.hash, e);
                    None
                }
            }
        } else {
            None
        };

        // Determine target type
        let target_type = if let Some(ref decoded) = decoded_input {
            self.abi_decoder.determine_target_type(
                transaction.to.as_deref(),
                &decoded.function_signature,
            )
        } else {
            self.abi_decoder.determine_target_type(transaction.to.as_deref(), "")
        };

        let processing_time = start_time.elapsed().as_millis() as u64;

        Ok(ParsedTransaction {
            transaction,
            decoded_input,
            target_type,
            processing_time_ms: processing_time,
        })
    }

    /// Extract transaction fields from JSON value
    fn extract_transaction_fields(&self, raw_tx: &Value) -> Result<Transaction> {
        let hash = raw_tx.get("hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing or invalid transaction hash"))?
            .to_string();

        let from = raw_tx.get("from")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing or invalid from address"))?
            .to_string();

        let to = raw_tx.get("to")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let value = raw_tx.get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("0x0")
            .to_string();

        let gas_price = raw_tx.get("gasPrice")
            .and_then(|v| v.as_str())
            .unwrap_or("0x0")
            .to_string();

        let gas_limit = raw_tx.get("gas")
            .and_then(|v| v.as_str())
            .unwrap_or("0x0")
            .to_string();

        let nonce = raw_tx.get("nonce")
            .and_then(|v| v.as_str())
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);

        let input = raw_tx.get("input")
            .and_then(|v| v.as_str())
            .unwrap_or("0x")
            .to_string();

        Ok(Transaction {
            hash,
            from,
            to,
            value,
            gas_price,
            gas_limit,
            nonce,
            input,
            timestamp: Utc::now(),
        })
    }

    /// Batch parse multiple transactions
    pub fn parse_batch(&self, raw_transactions: Vec<Value>) -> Vec<ParsedTransaction> {
        let mut parsed_transactions = Vec::with_capacity(raw_transactions.len());
        
        for raw_tx in raw_transactions {
            match self.parse_transaction(&raw_tx) {
                Ok(parsed_tx) => {
                    parsed_transactions.push(parsed_tx);
                }
                Err(e) => {
                    error!("Failed to parse transaction: {}", e);
                }
            }
        }
        
        parsed_transactions
    }

    /// Check if transaction is interesting for MEV (basic filtering)
    pub fn is_interesting_transaction(&self, parsed_tx: &ParsedTransaction) -> bool {
        // Filter criteria for MEV opportunities
        
        // 1. Must have a target contract
        if parsed_tx.transaction.to.is_none() {
            return false;
        }

        // 2. Must have input data (function call)
        if parsed_tx.transaction.input.is_empty() || parsed_tx.transaction.input == "0x" {
            return false;
        }

        // 3. Must be targeting known AMM/DEX protocols
        match parsed_tx.target_type {
            TargetType::UniswapV2 | 
            TargetType::UniswapV3 | 
            TargetType::SushiSwap | 
            TargetType::Curve | 
            TargetType::Balancer => true,
            _ => false,
        }
    }

    /// Extract gas price in wei
    pub fn extract_gas_price_wei(&self, gas_price_hex: &str) -> Result<u64> {
        let cleaned = gas_price_hex.trim_start_matches("0x");
        u64::from_str_radix(cleaned, 16)
            .map_err(|e| anyhow!("Failed to parse gas price: {}", e))
    }

    /// Extract value in wei
    pub fn extract_value_wei(&self, value_hex: &str) -> Result<u128> {
        let cleaned = value_hex.trim_start_matches("0x");
        if cleaned.is_empty() {
            return Ok(0);
        }
        u128::from_str_radix(cleaned, 16)
            .map_err(|e| anyhow!("Failed to parse value: {}", e))
    }
}

impl Default for TransactionParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_transaction() {
        let parser = TransactionParser::new();
        
        let raw_tx = json!({
            "hash": "0x123abc",
            "from": "0x456def",
            "to": "0x789ghi",
            "value": "0x1000",
            "gasPrice": "0x4a817c800",
            "gas": "0x5208",
            "nonce": "0x1",
            "input": "0xa9059cbb000000000000000000000000742d35cc6634c0532925a3b8d4c9db7c4c4c4c4c0000000000000000000000000000000000000000000000000de0b6b3a7640000"
        });

        let result = parser.parse_transaction(&raw_tx).unwrap();
        
        assert_eq!(result.transaction.hash, "0x123abc");
        assert_eq!(result.transaction.from, "0x456def");
        assert_eq!(result.transaction.to, Some("0x789ghi".to_string()));
        assert!(result.decoded_input.is_some());
    }

    #[test]
    fn test_extract_gas_price_wei() {
        let parser = TransactionParser::new();
        
        let gas_price = parser.extract_gas_price_wei("0x4a817c800").unwrap();
        assert_eq!(gas_price, 20000000000); // 20 gwei
    }

    #[test]
    fn test_is_interesting_transaction() {
        let parser = TransactionParser::new();
        
        let mut parsed_tx = ParsedTransaction {
            transaction: Transaction {
                hash: "0x123".to_string(),
                from: "0x456".to_string(),
                to: Some("0x789".to_string()),
                value: "1000".to_string(),
                gas_price: "20000000000".to_string(),
                gas_limit: "21000".to_string(),
                nonce: 1,
                input: "0xa9059cbb".to_string(),
                timestamp: Utc::now(),
            },
            decoded_input: None,
            target_type: TargetType::UniswapV2,
            processing_time_ms: 5,
        };

        assert!(parser.is_interesting_transaction(&parsed_tx));
        
        parsed_tx.target_type = TargetType::Unknown;
        assert!(!parser.is_interesting_transaction(&parsed_tx));
    }
}
