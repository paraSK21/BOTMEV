//! Bundle construction and signing system
//! 
//! This module provides the core bundle management functionality including:
//! - Bundle format definition (preTx, targetTx, postTx)
//! - Transaction signing with EIP-1559 support
//! - Secure key management
//! - Bundle builder with deterministic nonce rules

pub mod builder;
pub mod signer;
pub mod types;
pub mod key_manager;
pub mod submitter;
pub mod execution_tracker;
pub mod victim_generator;
pub mod mainnet_dry_run;
pub mod failure_handler;

#[cfg(test)]
mod integration_tests;

pub use builder::BundleBuilder;
pub use signer::TransactionSigner;
pub use types::*;
pub use key_manager::KeyManager;
pub use submitter::{BundleSubmitter, SubmissionConfig, SubmissionPath, FastResubmitter};
pub use execution_tracker::{ExecutionTracker, TrackerConfig, InclusionMetrics, OrderingVerification};
pub use victim_generator::{VictimGenerator, VictimGeneratorConfig, VictimTransaction, VictimDeployment, TestScenario, TestResults};
pub use builder::{ArbitrageBundleBuilder, BackrunBundleBuilder, SandwichBundleBuilder};
pub use mainnet_dry_run::{MainnetDryRun, DryRunConfig, DryRunMetrics};
pub use failure_handler::{FailureHandler, FailureConfig, FailureType, FailureResolution, SecurityChecklist};

use anyhow::Result;
use async_trait::async_trait;

/// Main bundle management interface
#[async_trait]
pub trait BundleManager: Send + Sync {
    /// Build a bundle from an opportunity
    async fn build_bundle(&self, opportunity: &crate::strategy_types::Opportunity) -> Result<Bundle>;
    
    /// Submit a bundle for execution
    async fn submit_bundle(&self, bundle: &Bundle) -> Result<SubmissionResult>;
    
    /// Track bundle execution status
    async fn track_execution(&self, bundle_id: &str) -> Result<ExecutionStatus>;
}

/// Bundle submission result
#[derive(Debug, Clone)]
pub struct SubmissionResult {
    pub bundle_id: String,
    pub submission_hash: String,
    pub submitted_at: chrono::DateTime<chrono::Utc>,
    pub gas_price: ethers::types::U256,
    pub estimated_inclusion_block: u64,
}

/// Bundle execution status
#[derive(Debug, Clone)]
pub enum ExecutionStatus {
    Pending,
    Included { block_number: u64, transaction_hashes: Vec<String> },
    Failed { reason: String },
    Expired,
}