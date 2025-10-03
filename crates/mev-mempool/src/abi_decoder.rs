use anyhow::Result;
use mev_core::{DecodedInput, DecodedParameter, TargetType};
use std::collections::HashMap;
use tracing::debug;

/// ABI decoder for common AMM and DEX function calls
pub struct AbiDecoder {
    function_signatures: HashMap<String, (String, TargetType)>,
}

impl AbiDecoder {
    pub fn new() -> Self {
        let mut decoder = Self {
            function_signatures: HashMap::new(),
        };
        
        decoder.load_precompiled_abis();
        decoder
    }

    /// Load precompiled ABIs for major AMMs and DEXs
    fn load_precompiled_abis(&mut self) {
        // Uniswap V2 Router common functions
        self.add_function_signature(
            "0xa9059cbb", // transfer(address,uint256)
            "transfer".to_string(),
            TargetType::UniswapV2,
        );
        
        self.add_function_signature(
            "0x7ff36ab5", // swapExactETHForTokens(uint256,address[],address,uint256)
            "swapExactETHForTokens".to_string(),
            TargetType::UniswapV2,
        );
        
        self.add_function_signature(
            "0x38ed1739", // swapExactTokensForTokens(uint256,uint256,address[],address,uint256)
            "swapExactTokensForTokens".to_string(),
            TargetType::UniswapV2,
        );
        
        self.add_function_signature(
            "0x18cbafe5", // swapExactTokensForETH(uint256,uint256,address[],address,uint256)
            "swapExactTokensForETH".to_string(),
            TargetType::UniswapV2,
        );

        // Uniswap V3 Router
        self.add_function_signature(
            "0x414bf389", // exactInputSingle((address,address,uint24,address,uint256,uint256,uint256,uint160))
            "exactInputSingle".to_string(),
            TargetType::UniswapV3,
        );
        
        self.add_function_signature(
            "0xc04b8d59", // exactInput((bytes,address,uint256,uint256,uint256))
            "exactInput".to_string(),
            TargetType::UniswapV3,
        );

        // SushiSwap (similar to Uniswap V2)
        self.add_function_signature(
            "0x38ed1739", // swapExactTokensForTokens
            "swapExactTokensForTokens".to_string(),
            TargetType::SushiSwap,
        );

        // Curve
        self.add_function_signature(
            "0x3df02124", // exchange(int128,int128,uint256,uint256)
            "exchange".to_string(),
            TargetType::Curve,
        );
        
        self.add_function_signature(
            "0xa6417ed6", // exchange_underlying(int128,int128,uint256,uint256)
            "exchange_underlying".to_string(),
            TargetType::Curve,
        );

        // Balancer
        self.add_function_signature(
            "0x52bbbe29", // swap((bytes32,uint8,address,address,uint256,bytes),(address,bool,address,bool),uint256,uint256)
            "swap".to_string(),
            TargetType::Balancer,
        );

        debug!("Loaded {} function signatures", self.function_signatures.len());
    }

    fn add_function_signature(&mut self, signature: &str, name: String, target_type: TargetType) {
        self.function_signatures.insert(
            signature.to_string(),
            (name, target_type),
        );
    }

    /// Decode transaction input data
    pub fn decode_input(&self, input: &str, _to_address: Option<&str>) -> Result<Option<DecodedInput>> {
        if input.len() < 10 {
            return Ok(None); // Not enough data for function signature
        }

        let function_signature = &input[0..10]; // First 4 bytes (8 hex chars + 0x)
        
        debug!("Decoding function signature: {}", function_signature);

        if let Some((function_name, _target_type)) = self.function_signatures.get(function_signature) {
            let parameters = self.decode_parameters(input, function_signature)?;
            
            return Ok(Some(DecodedInput {
                function_name: function_name.clone(),
                function_signature: function_signature.to_string(),
                parameters,
            }));
        }

        // Try to decode as generic function call
        if let Ok(decoded) = self.decode_generic_function(input) {
            return Ok(Some(decoded));
        }

        Ok(None)
    }

    /// Decode function parameters (simplified version)
    fn decode_parameters(&self, input: &str, function_signature: &str) -> Result<Vec<DecodedParameter>> {
        let mut parameters = Vec::new();

        // This is a simplified decoder - in production you'd use proper ABI decoding
        match function_signature {
            "0xa9059cbb" => {
                // transfer(address,uint256)
                if input.len() >= 74 {
                    let to_param = DecodedParameter {
                        name: "to".to_string(),
                        param_type: "address".to_string(),
                        value: format!("0x{}", &input[34..74]),
                    };
                    
                    let amount_hex = &input[74..];
                    let amount_param = DecodedParameter {
                        name: "amount".to_string(),
                        param_type: "uint256".to_string(),
                        value: format!("0x{}", amount_hex),
                    };
                    
                    parameters.push(to_param);
                    parameters.push(amount_param);
                }
            }
            "0x38ed1739" => {
                // swapExactTokensForTokens - simplified
                parameters.push(DecodedParameter {
                    name: "amountIn".to_string(),
                    param_type: "uint256".to_string(),
                    value: "decoded_amount_in".to_string(),
                });
                parameters.push(DecodedParameter {
                    name: "amountOutMin".to_string(),
                    param_type: "uint256".to_string(),
                    value: "decoded_amount_out_min".to_string(),
                });
            }
            _ => {
                // Generic parameter extraction
                parameters.push(DecodedParameter {
                    name: "data".to_string(),
                    param_type: "bytes".to_string(),
                    value: input[10..].to_string(),
                });
            }
        }

        Ok(parameters)
    }

    /// Attempt to decode as generic function call
    fn decode_generic_function(&self, input: &str) -> Result<DecodedInput> {
        let function_signature = &input[0..10];
        
        Ok(DecodedInput {
            function_name: "unknown".to_string(),
            function_signature: function_signature.to_string(),
            parameters: vec![DecodedParameter {
                name: "data".to_string(),
                param_type: "bytes".to_string(),
                value: input[10..].to_string(),
            }],
        })
    }

    /// Determine target type based on contract address and function signature
    pub fn determine_target_type(&self, to_address: Option<&str>, function_signature: &str) -> TargetType {
        // First check by function signature
        if let Some((_, target_type)) = self.function_signatures.get(function_signature) {
            return target_type.clone();
        }

        // Then check by known contract addresses (simplified)
        if let Some(address) = to_address {
            match address.to_lowercase().as_str() {
                // Uniswap V2 Router
                "0x7a250d5630b4cf539739df2c5dacb4c659f2488d" => TargetType::UniswapV2,
                // Uniswap V3 Router
                "0xe592427a0aece92de3edee1f18e0157c05861564" => TargetType::UniswapV3,
                // SushiSwap Router
                "0xd9e1ce17f2641f24ae83637ab66a2cca9c378b9f" => TargetType::SushiSwap,
                _ => TargetType::Unknown,
            }
        } else {
            TargetType::Unknown
        }
    }
}

impl Default for AbiDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_transfer() {
        let decoder = AbiDecoder::new();
        
        let input = "0xa9059cbb000000000000000000000000742d35cc6634c0532925a3b8d4c9db7c4c4c4c4c0000000000000000000000000000000000000000000000000de0b6b3a7640000";
        
        let result = decoder.decode_input(input, None).unwrap();
        assert!(result.is_some());
        
        let decoded = result.unwrap();
        assert_eq!(decoded.function_name, "transfer");
        assert_eq!(decoded.parameters.len(), 2);
    }

    #[test]
    fn test_determine_target_type() {
        let decoder = AbiDecoder::new();
        
        // Test with function signature that maps to SushiSwap (since it's the last one added)
        let target_type = decoder.determine_target_type(None, "0x38ed1739");
        match target_type {
            TargetType::SushiSwap => {},
            _ => panic!("Expected SushiSwap target type for function signature"),
        }
        
        // Test with Uniswap V2 router address
        let uniswap_v2_router = "0x7a250d5630b4cf539739df2c5dacb4c659f2488d";
        let target_type = decoder.determine_target_type(Some(uniswap_v2_router), "0x12345678");
        match target_type {
            TargetType::UniswapV2 => {},
            _ => panic!("Expected UniswapV2 target type for address"),
        }
    }
}
