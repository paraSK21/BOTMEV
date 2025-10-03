//! Secure key management system for transaction signing

use anyhow::{anyhow, Result};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::Address;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Encrypted key file format
#[derive(Debug, Serialize, Deserialize)]
struct EncryptedKeyFile {
    /// Encrypted private key
    encrypted_key: String,
    /// Key derivation parameters
    kdf_params: KdfParams,
    /// Encryption algorithm used
    cipher: String,
    /// Key version for future compatibility
    version: u32,
}

/// Key derivation function parameters
#[derive(Debug, Serialize, Deserialize)]
struct KdfParams {
    /// Salt for key derivation
    salt: String,
    /// Number of iterations
    iterations: u32,
    /// Key length in bytes
    key_length: u32,
}

/// Secure key management system
pub struct KeyManager {
    /// Loaded wallets by address
    wallets: Arc<RwLock<HashMap<Address, LocalWallet>>>,
    /// Default wallet for signing
    default_wallet: Arc<RwLock<Option<LocalWallet>>>,
    /// Key file directory
    key_dir: Option<std::path::PathBuf>,
}

impl KeyManager {
    /// Create a new key manager
    pub fn new() -> Self {
        Self {
            wallets: Arc::new(RwLock::new(HashMap::new())),
            default_wallet: Arc::new(RwLock::new(None)),
            key_dir: None,
        }
    }
    
    /// Create a key manager with a specific key directory
    pub fn with_key_dir<P: AsRef<Path>>(key_dir: P) -> Self {
        Self {
            wallets: Arc::new(RwLock::new(HashMap::new())),
            default_wallet: Arc::new(RwLock::new(None)),
            key_dir: Some(key_dir.as_ref().to_path_buf()),
        }
    }
    
    /// Load a wallet from an encrypted key file
    pub async fn load_encrypted_key<P: AsRef<Path>>(
        &self,
        key_file: P,
        password: &str,
    ) -> Result<Address> {
        let key_content = tokio::fs::read_to_string(key_file).await?;
        let encrypted_key: EncryptedKeyFile = serde_json::from_str(&key_content)?;
        
        // Decrypt the private key
        let private_key = self.decrypt_key(&encrypted_key, password)?;
        
        // Create wallet from private key
        let wallet = private_key.parse::<LocalWallet>()?;
        let address = wallet.address();
        
        // Store the wallet
        let mut wallets = self.wallets.write().await;
        wallets.insert(address, wallet.clone());
        
        // Set as default if it's the first wallet
        let mut default_wallet = self.default_wallet.write().await;
        if default_wallet.is_none() {
            *default_wallet = Some(wallet);
        }
        
        Ok(address)
    }
    
    /// Load a wallet from a plain private key (for development only)
    pub async fn load_private_key(&self, private_key: &str) -> Result<Address> {
        let wallet = private_key.parse::<LocalWallet>()?;
        let address = wallet.address();
        
        let mut wallets = self.wallets.write().await;
        wallets.insert(address, wallet.clone());
        
        // Set as default if it's the first wallet
        let mut default_wallet = self.default_wallet.write().await;
        if default_wallet.is_none() {
            *default_wallet = Some(wallet);
        }
        
        Ok(address)
    }
    
    /// Get a wallet by address
    pub async fn get_wallet(&self, address: &Address) -> Result<LocalWallet> {
        let wallets = self.wallets.read().await;
        wallets
            .get(address)
            .cloned()
            .ok_or_else(|| anyhow!("Wallet not found for address: {:?}", address))
    }
    
    /// Get the default wallet
    pub async fn get_default_wallet(&self) -> Result<LocalWallet> {
        let default_wallet = self.default_wallet.read().await;
        default_wallet
            .clone()
            .ok_or_else(|| anyhow!("No default wallet configured"))
    }
    
    /// Set the default wallet
    pub async fn set_default_wallet(&self, address: &Address) -> Result<()> {
        let wallets = self.wallets.read().await;
        let wallet = wallets
            .get(address)
            .ok_or_else(|| anyhow!("Wallet not found for address: {:?}", address))?;
        
        let mut default_wallet = self.default_wallet.write().await;
        *default_wallet = Some(wallet.clone());
        
        Ok(())
    }
    
    /// List all loaded wallet addresses
    pub async fn list_addresses(&self) -> Vec<Address> {
        let wallets = self.wallets.read().await;
        wallets.keys().cloned().collect()
    }
    
    /// Create and save an encrypted key file
    pub async fn create_encrypted_key<P: AsRef<Path>>(
        &self,
        key_file: P,
        password: &str,
    ) -> Result<Address> {
        // Generate a new wallet
        let wallet = LocalWallet::new(&mut rand::thread_rng());
        let address = wallet.address();
        
        // Encrypt the private key
        let private_key_hex = format!("{:x}", wallet.signer().to_bytes());
        let encrypted_key = self.encrypt_key(&private_key_hex, password)?;
        
        // Save to file
        let key_content = serde_json::to_string_pretty(&encrypted_key)?;
        tokio::fs::write(key_file, key_content).await?;
        
        // Load the wallet
        let mut wallets = self.wallets.write().await;
        wallets.insert(address, wallet.clone());
        
        // Set as default if it's the first wallet
        let mut default_wallet = self.default_wallet.write().await;
        if default_wallet.is_none() {
            *default_wallet = Some(wallet);
        }
        
        Ok(address)
    }
    
    /// Encrypt a private key with a password
    fn encrypt_key(&self, private_key: &str, password: &str) -> Result<EncryptedKeyFile> {
        use aes_gcm::{Aes256Gcm, Nonce, KeyInit};
        use aes_gcm::aead::Aead;
        use pbkdf2::pbkdf2;
        use hmac::Hmac;
        use sha2::Sha256;
        use rand::RngCore;
        
        // Generate random salt
        let mut salt = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut salt);
        
        // Derive key from password using PBKDF2
        let iterations = 100_000;
        let mut derived_key = [0u8; 32];
        pbkdf2::<Hmac<Sha256>>(password.as_bytes(), &salt, iterations, &mut derived_key);
        
        // Encrypt the private key
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&derived_key);
        let cipher = Aes256Gcm::new(key);
        
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        
        let ciphertext = cipher
            .encrypt(nonce, private_key.as_bytes())
            .map_err(|e| anyhow!("Encryption failed: {}", e))?;
        
        // Combine nonce and ciphertext
        let mut encrypted_data = nonce_bytes.to_vec();
        encrypted_data.extend_from_slice(&ciphertext);
        
        Ok(EncryptedKeyFile {
            encrypted_key: hex::encode(encrypted_data),
            kdf_params: KdfParams {
                salt: hex::encode(salt),
                iterations,
                key_length: 32,
            },
            cipher: "aes-256-gcm".to_string(),
            version: 1,
        })
    }
    
    /// Decrypt a private key with a password
    fn decrypt_key(&self, encrypted_key: &EncryptedKeyFile, password: &str) -> Result<String> {
        use aes_gcm::{Aes256Gcm, Nonce, KeyInit};
        use aes_gcm::aead::Aead;
        use pbkdf2::pbkdf2;
        use hmac::Hmac;
        use sha2::Sha256;
        
        if encrypted_key.cipher != "aes-256-gcm" {
            return Err(anyhow!("Unsupported cipher: {}", encrypted_key.cipher));
        }
        
        // Decode salt and encrypted data
        let salt = hex::decode(&encrypted_key.kdf_params.salt)?;
        let encrypted_data = hex::decode(&encrypted_key.encrypted_key)?;
        
        if encrypted_data.len() < 12 {
            return Err(anyhow!("Invalid encrypted data length"));
        }
        
        // Derive key from password
        let mut derived_key = [0u8; 32];
        pbkdf2::<Hmac<Sha256>>(
            password.as_bytes(),
            &salt,
            encrypted_key.kdf_params.iterations,
            &mut derived_key,
        );
        
        // Split nonce and ciphertext
        let (nonce_bytes, ciphertext) = encrypted_data.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        
        // Decrypt
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&derived_key);
        let cipher = Aes256Gcm::new(key);
        
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow!("Decryption failed: {}", e))?;
        
        String::from_utf8(plaintext).map_err(|e| anyhow!("Invalid UTF-8: {}", e))
    }
}

impl Default for KeyManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    
    #[tokio::test]
    async fn test_key_manager_private_key() {
        let key_manager = KeyManager::new();
        
        // Test private key (DO NOT use in production)
        let private_key = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        
        let address = key_manager.load_private_key(private_key).await.unwrap();
        
        // Should be able to retrieve the wallet
        let wallet = key_manager.get_wallet(&address).await.unwrap();
        assert_eq!(wallet.address(), address);
        
        // Should be the default wallet
        let default_wallet = key_manager.get_default_wallet().await.unwrap();
        assert_eq!(default_wallet.address(), address);
    }
    
    #[tokio::test]
    async fn test_encrypted_key_file() {
        let temp_dir = tempdir().unwrap();
        let key_file = temp_dir.path().join("test_key.json");
        
        let key_manager = KeyManager::with_key_dir(&temp_dir);
        let password = "test_password_123";
        
        // Create encrypted key
        let address = key_manager
            .create_encrypted_key(&key_file, password)
            .await
            .unwrap();
        
        // Verify file exists
        assert!(key_file.exists());
        
        // Create new key manager and load the key
        let key_manager2 = KeyManager::new();
        let loaded_address = key_manager2
            .load_encrypted_key(&key_file, password)
            .await
            .unwrap();
        
        assert_eq!(address, loaded_address);
    }
    
    #[tokio::test]
    async fn test_multiple_wallets() {
        let key_manager = KeyManager::new();
        
        let private_key1 = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        let private_key2 = "0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba";
        
        let address1 = key_manager.load_private_key(private_key1).await.unwrap();
        let address2 = key_manager.load_private_key(private_key2).await.unwrap();
        
        let addresses = key_manager.list_addresses().await;
        assert_eq!(addresses.len(), 2);
        assert!(addresses.contains(&address1));
        assert!(addresses.contains(&address2));
        
        // Change default wallet
        key_manager.set_default_wallet(&address2).await.unwrap();
        let default_wallet = key_manager.get_default_wallet().await.unwrap();
        assert_eq!(default_wallet.address(), address2);
    }
}