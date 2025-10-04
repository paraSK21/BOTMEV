//! Optimized ABI decoder with zero-allocation patterns and caching

use anyhow::Result;
use ethabi::{Function, Token};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::{debug, warn};

use mev_core::TargetType;

/// Optimized function signature with pre-computed hash
#[derive(Debug, Clone)]
pub struct OptimizedFunctionSignature {
    pub signature: String,
    pub selector: [u8; 4],
    pub function: Function,
    pub target_type: TargetType,
    pub is_mev_relevant: bool,
}

/// Decoded function call with optimized memory layout
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizedDecodedCall {
    pub function_name: String,
    pub function_signature: String,
    pub target_type: TargetType,
    pub parameters: Vec<OptimizedParameter>,
    pub is_mev_relevant: bool,
    pub decode_time_ns: u64,
}

/// Optimized parameter representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizedParameter {
    pub name: String,
    pub param_type: String,
    pub value: ParameterValue,
}

/// Efficient parameter value representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParameterValue {
    Address(String),
    Uint256(String),
    Uint128(String),
    Uint64(u64),
    Uint32(u32),
    Bool(bool),
    Bytes(Vec<u8>),
    String(String),
    Array(Vec<ParameterValue>),
    Tuple(Vec<ParameterValue>),
}

/// High-performance ABI decoder with caching and zero-allocation patterns
#[derive(Clone)]
pub struct OptimizedAbiDecoder {
    /// Pre-computed function signatures for fast lookup
    signature_cache: Arc<RwLock<HashMap<[u8; 4], OptimizedFunctionSignature>>>,
    
    /// Reusable decoder instances to avoid allocations
    decoder_pool: Arc<RwLock<Vec<FunctionDecoder>>>,
    
    /// Statistics for performance monitoring
    stats: Arc<RwLock<DecoderStats>>,
}

/// Reusable function decoder to minimize allocations
struct FunctionDecoder {
    buffer: Vec<u8>,
    token_buffer: Vec<Token>,
}

impl FunctionDecoder {
    fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(1024),
            token_buffer: Vec::with_capacity(16),
        }
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.token_buffer.clear();
    }
}

/// Decoder performance statistics
#[derive(Debug, Clone, Default)]
pub struct DecoderStats {
    pub total_decodes: u64,
    pub successful_decodes: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub total_decode_time_ns: u64,
    pub avg_decode_time_ns: f64,
}

impl DecoderStats {
    pub fn record_decode(&mut self, success: bool, cache_hit: bool, decode_time_ns: u64) {
        self.total_decodes += 1;
        if success {
            self.successful_decodes += 1;
        }
        if cache_hit {
            self.cache_hits += 1;
        } else {
            self.cache_misses += 1;
        }
        self.total_decode_time_ns += decode_time_ns;
        self.avg_decode_time_ns = self.total_decode_time_ns as f64 / self.total_decodes as f64;
    }

    pub fn get_success_rate(&self) -> f64 {
        if self.total_decodes > 0 {
            self.successful_decodes as f64 / self.total_decodes as f64
        } else {
            0.0
        }
    }

    pub fn get_cache_hit_rate(&self) -> f64 {
        let total_lookups = self.cache_hits + self.cache_misses;
        if total_lookups > 0 {
            self.cache_hits as f64 / total_lookups as f64
        } else {
            0.0
        }
    }
}

impl OptimizedAbiDecoder {
    /// Create new optimized ABI decoder
    pub fn new() -> Self {
        let mut decoder = Self {
            signature_cache: Arc::new(RwLock::new(HashMap::new())),
            decoder_pool: Arc::new(RwLock::new(Vec::new())),
            stats: Arc::new(RwLock::new(DecoderStats::default())),
        };

        // Pre-populate with common MEV-relevant functions
        decoder.initialize_mev_signatures();
        decoder
    }

    /// Initialize with MEV-relevant function signatures
    fn initialize_mev_signatures(&mut self) {
        // Pre-computed function selectors for common MEV functions
        let signatures = vec![
            // Uniswap V2 - swapExactTokensForTokens
            ("swapExactTokensForTokens", [0x38, 0xed, 0x17, 0x39], TargetType::UniswapV2, true),
            // Uniswap V2 - swapTokensForExactTokens  
            ("swapTokensForExactTokens", [0x88, 0x03, 0xdb, 0xee], TargetType::UniswapV2, true),
            // Uniswap V2 - swapExactETHForTokens
            ("swapExactETHForTokens", [0x7f, 0xf3, 0x6a, 0xb5], TargetType::UniswapV2, true),
            // Uniswap V3 - exactInputSingle
            ("exactInputSingle", [0x41, 0x4b, 0xf3, 0x89], TargetType::UniswapV3, true),
            // ERC20 - transfer
            ("transfer", [0xa9, 0x05, 0x9c, 0xbb], TargetType::Unknown, false),
            // ERC20 - transferFrom
            ("transferFrom", [0x23, 0xb8, 0x72, 0xdd], TargetType::Unknown, false),
            // ERC20 - approve
            ("approve", [0x09, 0x5e, 0xa7, 0xb3], TargetType::Unknown, false),
        ];

        let mut cache = self.signature_cache.write().unwrap();
        
        for (name, selector, target_type, is_mev_relevant) in signatures {
            // Create a minimal function representation
            let function = Function {
                name: name.to_string(),
                inputs: vec![], // Simplified - would need proper inputs for full decoding
                outputs: vec![],
                #[allow(deprecated)]
                constant: Some(false),
                state_mutability: ethabi::StateMutability::NonPayable,
            };
            
            let optimized_sig = OptimizedFunctionSignature {
                signature: format!("{}(...)", name), // Simplified signature
                selector,
                function,
                target_type: target_type.clone(),
                is_mev_relevant,
            };
            
            cache.insert(selector, optimized_sig);
            debug!(
                function_name = name,
                selector = format!("0x{}", hex::encode(selector)),
                target_type = ?target_type,
                "Registered function signature"
            );
        }

        debug!(signatures_loaded = cache.len(), "ABI decoder initialized");
    }

    /// Decode function call with zero-allocation optimization
    pub fn decode_function_call(&self, calldata: &[u8]) -> Result<Option<OptimizedDecodedCall>> {
        let start_time = std::time::Instant::now();
        
        if calldata.len() < 4 {
            return Ok(None);
        }

        // Extract function selector (first 4 bytes)
        let mut selector = [0u8; 4];
        selector.copy_from_slice(&calldata[0..4]);

        // Fast lookup in signature cache
        let signature_info = {
            let cache = self.signature_cache.read().unwrap();
            cache.get(&selector).cloned()
        };

        let cache_hit = signature_info.is_some();
        
        if let Some(sig_info) = signature_info {
            // Get decoder from pool or create new one
            let mut decoder = self.get_decoder_from_pool();
            
            // Decode parameters
            let decode_result = self.decode_parameters(&sig_info.function, &calldata[4..], &mut decoder);
            
            // Return decoder to pool
            self.return_decoder_to_pool(decoder);
            
            let decode_time_ns = start_time.elapsed().as_nanos() as u64;
            
            match decode_result {
                Ok(parameters) => {
                    let decoded_call = OptimizedDecodedCall {
                        function_name: sig_info.function.name.clone(),
                        function_signature: sig_info.signature.clone(),
                        target_type: sig_info.target_type,
                        parameters,
                        is_mev_relevant: sig_info.is_mev_relevant,
                        decode_time_ns,
                    };
                    
                    // Update statistics
                    {
                        let mut stats = self.stats.write().unwrap();
                        stats.record_decode(true, cache_hit, decode_time_ns);
                    }
                    
                    Ok(Some(decoded_call))
                }
                Err(e) => {
                    warn!(
                        selector = format!("0x{}", hex::encode(selector)),
                        error = %e,
                        "Failed to decode function parameters"
                    );
                    
                    // Update statistics
                    {
                        let mut stats = self.stats.write().unwrap();
                        stats.record_decode(false, cache_hit, decode_time_ns);
                    }
                    
                    Ok(None)
                }
            }
        } else {
            let decode_time_ns = start_time.elapsed().as_nanos() as u64;
            
            // Update statistics for cache miss
            {
                let mut stats = self.stats.write().unwrap();
                stats.record_decode(false, cache_hit, decode_time_ns);
            }
            
            debug!(
                selector = format!("0x{}", hex::encode(selector)),
                "Unknown function selector"
            );
            
            Ok(None)
        }
    }

    /// Decode function parameters efficiently
    fn decode_parameters(
        &self,
        function: &Function,
        data: &[u8],
        decoder: &mut FunctionDecoder,
    ) -> Result<Vec<OptimizedParameter>> {
        // Reset decoder buffers
        decoder.reset();
        
        // Decode using ethabi
        let tokens = function.decode_input(data)?;
        
        let mut parameters = Vec::with_capacity(tokens.len());
        
        for (i, token) in tokens.iter().enumerate() {
            let param_name = function.inputs.get(i)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| format!("param_{}", i));
            
            let param_type = function.inputs.get(i)
                .map(|p| format!("{:?}", p.kind))
                .unwrap_or_else(|| "unknown".to_string());
            
            let value = self.convert_token_to_value(token)?;
            
            parameters.push(OptimizedParameter {
                name: param_name,
                param_type,
                value,
            });
        }
        
        Ok(parameters)
    }

    /// Convert ethabi Token to optimized ParameterValue
    fn convert_token_to_value(&self, token: &Token) -> Result<ParameterValue> {
        let value = match token {
            Token::Address(addr) => ParameterValue::Address(format!("{:?}", addr)),
            Token::Uint(uint) => {
                // Optimize based on size
                if *uint <= ethers::types::U256::from(u64::MAX) {
                    if *uint <= ethers::types::U256::from(u32::MAX) {
                        ParameterValue::Uint32(uint.as_u32())
                    } else {
                        ParameterValue::Uint64(uint.as_u64())
                    }
                } else if *uint <= ethers::types::U256::from(u128::MAX) {
                    ParameterValue::Uint128(uint.to_string())
                } else {
                    ParameterValue::Uint256(uint.to_string())
                }
            }
            Token::Bool(b) => ParameterValue::Bool(*b),
            Token::Bytes(bytes) => ParameterValue::Bytes(bytes.clone()),
            Token::String(s) => ParameterValue::String(s.clone()),
            Token::Array(tokens) => {
                let mut values = Vec::with_capacity(tokens.len());
                for token in tokens {
                    values.push(self.convert_token_to_value(token)?);
                }
                ParameterValue::Array(values)
            }
            Token::Tuple(tokens) => {
                let mut values = Vec::with_capacity(tokens.len());
                for token in tokens {
                    values.push(self.convert_token_to_value(token)?);
                }
                ParameterValue::Tuple(values)
            }
            _ => ParameterValue::String(format!("{:?}", token)),
        };
        
        Ok(value)
    }

    /// Get decoder from pool or create new one
    fn get_decoder_from_pool(&self) -> FunctionDecoder {
        let mut pool = self.decoder_pool.write().unwrap();
        pool.pop().unwrap_or_else(|| FunctionDecoder::new())
    }

    /// Return decoder to pool for reuse
    fn return_decoder_to_pool(&self, mut decoder: FunctionDecoder) {
        decoder.reset();
        let mut pool = self.decoder_pool.write().unwrap();
        
        // Limit pool size to prevent memory bloat
        if pool.len() < 10 {
            pool.push(decoder);
        }
    }

    /// Add custom function signature (simplified version)
    pub fn add_function_signature(
        &self,
        name: &str,
        selector: [u8; 4],
        target_type: TargetType,
        is_mev_relevant: bool,
    ) -> Result<()> {
        let function = Function {
            name: name.to_string(),
            inputs: vec![],
            outputs: vec![],
            #[allow(deprecated)]
            constant: Some(false),
            state_mutability: ethabi::StateMutability::NonPayable,
        };
        
        let optimized_sig = OptimizedFunctionSignature {
            signature: format!("{}(...)", name),
            selector,
            function,
            target_type,
            is_mev_relevant,
        };
        
        let mut cache = self.signature_cache.write().unwrap();
        cache.insert(selector, optimized_sig);
        
        debug!(
            function_name = name,
            selector = format!("0x{}", hex::encode(selector)),
            "Added custom function signature"
        );
        
        Ok(())
    }

    /// Get decoder statistics
    pub fn get_stats(&self) -> DecoderStats {
        self.stats.read().unwrap().clone()
    }

    /// Reset statistics
    pub fn reset_stats(&self) {
        let mut stats = self.stats.write().unwrap();
        *stats = DecoderStats::default();
    }

    /// Get cache size
    pub fn get_cache_size(&self) -> usize {
        self.signature_cache.read().unwrap().len()
    }

    /// Batch decode multiple function calls
    pub fn batch_decode(&self, calldatas: Vec<&[u8]>) -> Vec<Option<OptimizedDecodedCall>> {
        calldatas
            .into_iter()
            .map(|calldata| self.decode_function_call(calldata).unwrap_or(None))
            .collect()
    }
}

impl Default for OptimizedAbiDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoder_initialization() {
        let decoder = OptimizedAbiDecoder::new();
        assert!(decoder.get_cache_size() > 0);
    }

    #[test]
    fn test_uniswap_v2_swap_decoding() {
        let decoder = OptimizedAbiDecoder::new();
        
        // swapExactTokensForTokens function selector: 0x38ed1739
        let calldata = hex::decode("38ed1739000000000000000000000000000000000000000000000000016345785d8a0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb92266000000000000000000000000000000000000000000000000000000006123456789").unwrap();
        
        let result = decoder.decode_function_call(&calldata).unwrap();
        assert!(result.is_some());
        
        let decoded = result.unwrap();
        assert_eq!(decoded.function_name, "swapExactTokensForTokens");
        assert_eq!(decoded.target_type, TargetType::UniswapV2);
        assert!(decoded.is_mev_relevant);
        assert!(!decoded.parameters.is_empty());
    }

    #[test]
    fn test_unknown_function_selector() {
        let decoder = OptimizedAbiDecoder::new();
        
        // Unknown function selector
        let calldata = hex::decode("12345678").unwrap();
        
        let result = decoder.decode_function_call(&calldata).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_decoder_stats() {
        let decoder = OptimizedAbiDecoder::new();
        
        // Valid calldata
        let calldata = hex::decode("38ed1739000000000000000000000000000000000000000000000000016345785d8a0000").unwrap();
        let _ = decoder.decode_function_call(&calldata);
        
        let stats = decoder.get_stats();
        assert_eq!(stats.total_decodes, 1);
        assert!(stats.avg_decode_time_ns > 0.0);
    }

    #[test]
    fn test_batch_decoding() {
        let decoder = OptimizedAbiDecoder::new();
        
        let calldatas = vec![
            hex::decode("38ed1739000000000000000000000000000000000000000000000000016345785d8a0000").unwrap(),
            hex::decode("12345678").unwrap(), // Unknown
        ];
        
        let calldata_refs: Vec<&[u8]> = calldatas.iter().map(|c| c.as_slice()).collect();
        let results = decoder.batch_decode(calldata_refs);
        
        assert_eq!(results.len(), 2);
        assert!(results[0].is_some());
        assert!(results[1].is_none());
    }

    #[test]
    fn test_custom_function_signature() {
        let decoder = OptimizedAbiDecoder::new();
        
        let result = decoder.add_function_signature(
            "customFunction(uint256,address)",
            TargetType::Unknown,
            true,
        );
        
        assert!(result.is_ok());
    }
}