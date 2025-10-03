use anyhow::Result;

/// Utility functions for MEV operations
pub fn calculate_gas_price(base_fee: u64, priority_fee: u64) -> u64 {
    base_fee + priority_fee
}

/// Format ethereum address
pub fn format_address(address: &str) -> Result<String> {
    if address.starts_with("0x") && address.len() == 42 {
        Ok(address.to_lowercase())
    } else {
        Err(anyhow::anyhow!("Invalid ethereum address format"))
    }
}
