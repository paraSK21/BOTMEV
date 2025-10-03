use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub hash: String,
    pub from: String,
    pub to: Option<String>,
    pub value: String,
    pub gas_price: String,
    pub gas_limit: String,
    pub nonce: u64,
    pub input: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedTransaction {
    pub transaction: Transaction,
    pub decoded_input: Option<DecodedInput>,
    pub target_type: TargetType,
    pub processing_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodedInput {
    pub function_name: String,
    pub function_signature: String,
    pub parameters: Vec<DecodedParameter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodedParameter {
    pub name: String,
    pub param_type: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TargetType {
    UniswapV2,
    UniswapV3,
    SushiSwap,
    Curve,
    Balancer,
    OrderBook,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub number: u64,
    pub hash: String,
    pub timestamp: u64,
    pub transactions: Vec<Transaction>,
}
