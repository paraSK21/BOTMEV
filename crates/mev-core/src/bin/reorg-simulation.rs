//! Blockchain reorganization simulation and handling demonstration
//! 
//! This tool demonstrates reorg detection and handling using a local fork
//! and validates the state management system's ability to handle chain reorgs.

use anyhow::Result;
use chrono::Utc;
use mev_core::{
    BlockHeader, PendingTransactionState, PersistenceManager, ReorgEvent, StateManager,
    StateManagerConfig, TransactionStatus,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::{fs::File, io::AsyncWriteExt, time::sleep};
use tracing::{info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorgSimulationResult {
    pub simulation_name: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub original_chain: Vec<BlockHeader>,
    pub reorg_chain: Vec<BlockHeader>,
    pub detected_reorgs: Vec<ReorgEvent>,
    pub affected_transactions: usize,
    pub detection_latency_ms: f64,
    pub recovery_time_ms: f64,
    pub success: bool,
}

/// Blockchain simulator for reorg testing
pub struct BlockchainSimulator {
    current_block_number: u64,
    chains: HashMap<String, Vec<BlockHeader>>, // chain_id -> blocks
    active_chain: String,
}

impl BlockchainSimulator {
    pub fn new() -> Self {
        Self {
            current_block_number: 0,
            chains: HashMap::new(),
            active_chain: "main".to_string(),
        }
    }

    /// Create initial blockchain with genesis block
    pub fn initialize_genesis(&mut self) -> BlockHeader {
        let genesis = BlockHeader::new(0, "0x0000000000000000".to_string(), "0x0000000000000000".to_string());
        
        self.chains.insert("main".to_string(), vec![genesis.clone()]);
        self.current_block_number = 0;
        
        info!("Initialized genesis block");
        genesis
    }

    /// Mine a new block on the active chain
    pub fn mine_block(&mut self) -> Result<BlockHeader> {
        let chain = self.chains.get_mut(&self.active_chain)
            .ok_or_else(|| anyhow::anyhow!("Active chain not found"))?;

        let parent = chain.last()
            .ok_or_else(|| anyhow::anyhow!("No parent block found"))?;

        self.current_block_number += 1;
        let new_block = BlockHeader::new(
            self.current_block_number,
            format!("0x{:016x}", self.current_block_number * 1000 + rand::random::<u16>() as u64),
            parent.hash.clone(),
        );

        chain.push(new_block.clone());
        
        info!(
            chain = %self.active_chain,
            block_number = new_block.number,
            block_hash = %new_block.hash,
            "Mined new block"
        );

        Ok(new_block)
    }

    /// Create a fork at a specific block number
    pub fn create_fork(&mut self, fork_name: &str, fork_point: u64) -> Result<()> {
        let main_chain = self.chains.get("main")
            .ok_or_else(|| anyhow::anyhow!("Main chain not found"))?;

        // Find the fork point
        let fork_blocks: Vec<BlockHeader> = main_chain
            .iter()
            .take_while(|block| block.number <= fork_point)
            .cloned()
            .collect();

        if fork_blocks.is_empty() {
            return Err(anyhow::anyhow!("Fork point {} not found", fork_point));
        }

        self.chains.insert(fork_name.to_string(), fork_blocks);
        
        info!(
            fork_name = fork_name,
            fork_point = fork_point,
            "Created blockchain fork"
        );

        Ok(())
    }

    /// Switch to a different chain
    pub fn switch_chain(&mut self, chain_name: &str) -> Result<()> {
        if !self.chains.contains_key(chain_name) {
            return Err(anyhow::anyhow!("Chain {} not found", chain_name));
        }

        let old_chain = self.active_chain.clone();
        self.active_chain = chain_name.to_string();

        // Update current block number to match the new chain
        if let Some(chain) = self.chains.get(chain_name) {
            if let Some(last_block) = chain.last() {
                self.current_block_number = last_block.number;
            }
        }

        info!(
            old_chain = %old_chain,
            new_chain = %chain_name,
            current_block = self.current_block_number,
            "Switched active chain"
        );

        Ok(())
    }

    /// Get the current head block
    pub fn get_head(&self) -> Option<BlockHeader> {
        self.chains.get(&self.active_chain)
            .and_then(|chain| chain.last())
            .cloned()
    }

    /// Get chain information
    pub fn get_chain_info(&self, chain_name: &str) -> Option<(usize, Option<BlockHeader>)> {
        self.chains.get(chain_name).map(|chain| {
            (chain.len(), chain.last().cloned())
        })
    }

    /// Mine blocks on a specific chain
    pub fn mine_blocks_on_chain(&mut self, chain_name: &str, count: usize) -> Result<Vec<BlockHeader>> {
        let original_chain = self.active_chain.clone();
        self.switch_chain(chain_name)?;

        let mut mined_blocks = Vec::new();
        for _ in 0..count {
            let block = self.mine_block()?;
            mined_blocks.push(block);
        }

        // Switch back to original chain
        self.switch_chain(&original_chain)?;

        Ok(mined_blocks)
    }
}

/// Reorg simulation test suite
pub struct ReorgSimulationSuite {
    simulator: BlockchainSimulator,
    state_manager: StateManager,
    persistence: Option<PersistenceManager>,
}

impl ReorgSimulationSuite {
    pub fn new() -> Result<Self> {
        let config = StateManagerConfig {
            max_pending_transactions: 10000,
            max_block_history: 100,
            transaction_ttl_seconds: 300,
            reorg_detection_depth: 10,
            checkpoint_interval_seconds: 30,
            enable_persistence: true,
            database_path: "reorg_test.db".to_string(),
        };

        let state_manager = StateManager::new(config.clone());
        let persistence = if config.enable_persistence {
            Some(PersistenceManager::new(&config.database_path)?)
        } else {
            None
        };

        Ok(Self {
            simulator: BlockchainSimulator::new(),
            state_manager,
            persistence,
        })
    }

    /// Run comprehensive reorg simulation tests
    pub async fn run_comprehensive_tests(&mut self) -> Result<Vec<ReorgSimulationResult>> {
        info!("Starting comprehensive reorg simulation tests");

        let mut results = Vec::new();

        // Test 1: Simple 1-block reorg
        results.push(self.test_simple_reorg().await?);

        // Test 2: Deep reorg (5 blocks)
        results.push(self.test_deep_reorg().await?);

        // Test 3: Reorg with pending transactions
        results.push(self.test_reorg_with_pending_transactions().await?);

        // Test 4: Multiple competing chains
        results.push(self.test_multiple_competing_chains().await?);

        // Test 5: Rapid reorg sequence
        results.push(self.test_rapid_reorg_sequence().await?);

        // Generate summary report
        self.generate_simulation_report(&results).await?;

        Ok(results)
    }

    /// Test simple 1-block reorganization
    async fn test_simple_reorg(&mut self) -> Result<ReorgSimulationResult> {
        info!("Testing simple 1-block reorganization");

        let start_time = Instant::now();

        // Initialize blockchain
        let genesis = self.simulator.initialize_genesis();
        self.state_manager.initialize(genesis).await?;

        // Mine initial chain: 0 -> 1 -> 2
        let block1 = self.simulator.mine_block()?;
        self.state_manager.update_block_header(block1.clone()).await?;

        let block2 = self.simulator.mine_block()?;
        self.state_manager.update_block_header(block2.clone()).await?;

        let original_chain = vec![block1.clone(), block2.clone()];

        // Create fork at block 1 and mine alternative block 2
        self.simulator.create_fork("alt", 1)?;
        let alt_blocks = self.simulator.mine_blocks_on_chain("alt", 1)?;
        let alt_block2 = alt_blocks[0].clone();

        // Simulate reorg by updating with alternative block 2
        let detection_start = Instant::now();
        let reorg_result = self.state_manager.update_block_header(alt_block2.clone()).await?;
        let detection_latency = detection_start.elapsed().as_millis() as f64;

        let reorg_chain = vec![block1, alt_block2];

        // Verify reorg was detected
        let detected_reorgs = self.state_manager.get_reorg_events(10);
        let success = reorg_result.is_some() && !detected_reorgs.is_empty();

        let total_time = start_time.elapsed().as_millis() as f64;

        Ok(ReorgSimulationResult {
            simulation_name: "simple_reorg".to_string(),
            timestamp: Utc::now(),
            original_chain,
            reorg_chain,
            detected_reorgs,
            affected_transactions: 0,
            detection_latency_ms: detection_latency,
            recovery_time_ms: total_time,
            success,
        })
    }

    /// Test deep reorganization (5 blocks)
    async fn test_deep_reorg(&mut self) -> Result<ReorgSimulationResult> {
        info!("Testing deep reorganization (5 blocks)");

        let start_time = Instant::now();

        // Reset simulator
        self.simulator = BlockchainSimulator::new();
        let genesis = self.simulator.initialize_genesis();
        self.state_manager.initialize(genesis).await?;

        // Mine main chain: 0 -> 1 -> 2 -> 3 -> 4 -> 5
        let mut original_chain = Vec::new();
        for _ in 0..5 {
            let block = self.simulator.mine_block()?;
            self.state_manager.update_block_header(block.clone()).await?;
            original_chain.push(block);
        }

        // Create fork at block 1 and mine longer alternative chain
        self.simulator.create_fork("deep_alt", 1)?;
        let alt_blocks = self.simulator.mine_blocks_on_chain("deep_alt", 6)?; // Make it longer

        let mut reorg_chain = vec![original_chain[0].clone()]; // Include fork point
        reorg_chain.extend(alt_blocks.clone());

        // Simulate deep reorg by updating with alternative chain
        let detection_start = Instant::now();
        let mut detected_reorgs = Vec::new();
        
        for alt_block in &alt_blocks {
            if let Some(reorg) = self.state_manager.update_block_header(alt_block.clone()).await? {
                detected_reorgs.push(reorg);
            }
        }

        let detection_latency = detection_start.elapsed().as_millis() as f64;
        let total_time = start_time.elapsed().as_millis() as f64;

        let success = !detected_reorgs.is_empty();

        Ok(ReorgSimulationResult {
            simulation_name: "deep_reorg".to_string(),
            timestamp: Utc::now(),
            original_chain,
            reorg_chain,
            detected_reorgs,
            affected_transactions: 0,
            detection_latency_ms: detection_latency,
            recovery_time_ms: total_time,
            success,
        })
    }

    /// Test reorg with pending transactions
    async fn test_reorg_with_pending_transactions(&mut self) -> Result<ReorgSimulationResult> {
        info!("Testing reorg with pending transactions");

        let start_time = Instant::now();

        // Reset and initialize
        self.simulator = BlockchainSimulator::new();
        let genesis = self.simulator.initialize_genesis();
        self.state_manager.initialize(genesis).await?;

        // Mine initial blocks
        let block1 = self.simulator.mine_block()?;
        self.state_manager.update_block_header(block1.clone()).await?;

        // Add some pending transactions
        let pending_txs = self.create_test_transactions(5);
        for tx in &pending_txs {
            self.state_manager.update_pending_transaction(tx.hash.clone(), tx.clone()).await?;
        }

        let block2 = self.simulator.mine_block()?;
        self.state_manager.update_block_header(block2.clone()).await?;

        let original_chain = vec![block1.clone(), block2];

        // Create alternative chain
        self.simulator.create_fork("tx_alt", 1)?;
        let alt_blocks = self.simulator.mine_blocks_on_chain("tx_alt", 2)?;

        let mut reorg_chain = vec![block1];
        reorg_chain.extend(alt_blocks);

        // Simulate reorg
        let detection_start = Instant::now();
        let mut detected_reorgs = Vec::new();
        
        for alt_block in &reorg_chain[1..] {
            if let Some(reorg) = self.state_manager.update_block_header(alt_block.clone()).await? {
                detected_reorgs.push(reorg);
            }
        }

        let detection_latency = detection_start.elapsed().as_millis() as f64;
        let total_time = start_time.elapsed().as_millis() as f64;

        let success = !detected_reorgs.is_empty();
        let affected_transactions = pending_txs.len();

        Ok(ReorgSimulationResult {
            simulation_name: "reorg_with_transactions".to_string(),
            timestamp: Utc::now(),
            original_chain,
            reorg_chain,
            detected_reorgs,
            affected_transactions,
            detection_latency_ms: detection_latency,
            recovery_time_ms: total_time,
            success,
        })
    }

    /// Test multiple competing chains
    async fn test_multiple_competing_chains(&mut self) -> Result<ReorgSimulationResult> {
        info!("Testing multiple competing chains");

        let start_time = Instant::now();

        // Reset and initialize
        self.simulator = BlockchainSimulator::new();
        let genesis = self.simulator.initialize_genesis();
        self.state_manager.initialize(genesis).await?;

        // Mine base chain
        let block1 = self.simulator.mine_block()?;
        self.state_manager.update_block_header(block1.clone()).await?;

        let original_chain = vec![block1.clone()];

        // Create multiple competing forks
        self.simulator.create_fork("chain_a", 1)?;
        self.simulator.create_fork("chain_b", 1)?;
        self.simulator.create_fork("chain_c", 1)?;

        // Mine blocks on each chain
        let chain_a_blocks = self.simulator.mine_blocks_on_chain("chain_a", 3)?;
        let chain_b_blocks = self.simulator.mine_blocks_on_chain("chain_b", 4)?; // Longest
        let _chain_c_blocks = self.simulator.mine_blocks_on_chain("chain_c", 2)?;

        // Simulate switching between chains (longest chain wins)
        let detection_start = Instant::now();
        let mut detected_reorgs = Vec::new();

        // First, update with chain A
        for block in &chain_a_blocks {
            if let Some(reorg) = self.state_manager.update_block_header(block.clone()).await? {
                detected_reorgs.push(reorg);
            }
        }

        // Then switch to chain B (longer)
        for block in &chain_b_blocks {
            if let Some(reorg) = self.state_manager.update_block_header(block.clone()).await? {
                detected_reorgs.push(reorg);
            }
        }

        let detection_latency = detection_start.elapsed().as_millis() as f64;
        let total_time = start_time.elapsed().as_millis() as f64;

        let mut reorg_chain = vec![block1];
        reorg_chain.extend(chain_b_blocks); // Final winning chain

        let success = detected_reorgs.len() >= 2; // Should detect multiple reorgs

        Ok(ReorgSimulationResult {
            simulation_name: "multiple_competing_chains".to_string(),
            timestamp: Utc::now(),
            original_chain,
            reorg_chain,
            detected_reorgs,
            affected_transactions: 0,
            detection_latency_ms: detection_latency,
            recovery_time_ms: total_time,
            success,
        })
    }

    /// Test rapid reorg sequence
    async fn test_rapid_reorg_sequence(&mut self) -> Result<ReorgSimulationResult> {
        info!("Testing rapid reorg sequence");

        let start_time = Instant::now();

        // Reset and initialize
        self.simulator = BlockchainSimulator::new();
        let genesis = self.simulator.initialize_genesis();
        self.state_manager.initialize(genesis).await?;

        // Mine initial chain
        let mut original_chain = Vec::new();
        for _ in 0..3 {
            let block = self.simulator.mine_block()?;
            self.state_manager.update_block_header(block.clone()).await?;
            original_chain.push(block);
        }

        // Create multiple rapid reorgs
        let detection_start = Instant::now();
        let mut detected_reorgs = Vec::new();
        let mut final_chain = original_chain.clone();

        for i in 0..5 {
            // Create fork and mine alternative blocks rapidly
            let fork_name = format!("rapid_{}", i);
            self.simulator.create_fork(&fork_name, 2)?; // Fork at block 2
            
            let alt_blocks = self.simulator.mine_blocks_on_chain(&fork_name, 2)?;
            
            // Update state manager with rapid reorgs
            for alt_block in &alt_blocks {
                if let Some(reorg) = self.state_manager.update_block_header(alt_block.clone()).await? {
                    detected_reorgs.push(reorg);
                }
                
                // Small delay to simulate network propagation
                sleep(Duration::from_millis(10)).await;
            }

            final_chain = vec![original_chain[0].clone(), original_chain[1].clone()];
            final_chain.extend(alt_blocks);
        }

        let detection_latency = detection_start.elapsed().as_millis() as f64;
        let total_time = start_time.elapsed().as_millis() as f64;

        let success = detected_reorgs.len() >= 3; // Should detect multiple rapid reorgs

        Ok(ReorgSimulationResult {
            simulation_name: "rapid_reorg_sequence".to_string(),
            timestamp: Utc::now(),
            original_chain,
            reorg_chain: final_chain,
            detected_reorgs,
            affected_transactions: 0,
            detection_latency_ms: detection_latency,
            recovery_time_ms: total_time,
            success,
        })
    }

    /// Create test transactions
    fn create_test_transactions(&self, count: usize) -> Vec<PendingTransactionState> {
        let mut transactions = Vec::new();
        
        for i in 0..count {
            let tx = PendingTransactionState {
                hash: format!("0x{:064x}", i),
                from: format!("0x{:040x}", i * 100),
                to: Some(format!("0x{:040x}", i * 100 + 1)),
                nonce: i as u64,
                gas_price: 20_000_000_000,
                gas_limit: 21_000,
                value: "1000000000000000000".to_string(),
                data: "0x".to_string(),
                first_seen: Utc::now(),
                last_seen: Utc::now(),
                seen_count: 1,
                block_number: None,
                block_hash: None,
                status: TransactionStatus::Pending,
            };
            transactions.push(tx);
        }

        transactions
    }

    /// Generate simulation report
    async fn generate_simulation_report(&self, results: &[ReorgSimulationResult]) -> Result<()> {
        let filename = format!(
            "reorg_simulation_report_{}.json",
            Utc::now().format("%Y%m%d_%H%M%S")
        );

        let json_data = serde_json::to_string_pretty(results)?;
        let mut file = File::create(&filename).await?;
        file.write_all(json_data.as_bytes()).await?;

        info!("Reorg simulation report saved to: {}", filename);

        // Print summary to console
        println!("\n=== Blockchain Reorg Simulation Results ===");
        for result in results {
            let status = if result.success { "✅ PASS" } else { "❌ FAIL" };
            println!(
                "{}: {} - Detection: {:.2}ms, Recovery: {:.2}ms, Reorgs: {}",
                result.simulation_name,
                status,
                result.detection_latency_ms,
                result.recovery_time_ms,
                result.detected_reorgs.len()
            );
        }

        // Overall assessment
        let passed = results.iter().filter(|r| r.success).count();
        let total = results.len();
        println!("\nOverall: {}/{} tests passed ({:.1}%)", 
                passed, total, (passed as f64 / total as f64) * 100.0);

        if passed == total {
            println!("🎉 All reorg simulation tests passed!");
        } else {
            println!("⚠️  Some reorg simulation tests failed - review implementation");
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("info,mev_core=debug")
        .with_target(false)
        .with_thread_ids(true)
        .init();

    info!("Starting Blockchain Reorg Simulation");

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    let test_type = args.get(1).map(|s| s.as_str()).unwrap_or("comprehensive");

    let mut simulation_suite = ReorgSimulationSuite::new()?;

    match test_type {
        "comprehensive" => {
            let results = simulation_suite.run_comprehensive_tests().await?;
            info!("Comprehensive reorg simulation completed with {} results", results.len());
        }
        "simple" => {
            let result = simulation_suite.test_simple_reorg().await?;
            info!("Simple reorg test completed: {}", if result.success { "PASS" } else { "FAIL" });
        }
        "deep" => {
            let result = simulation_suite.test_deep_reorg().await?;
            info!("Deep reorg test completed: {}", if result.success { "PASS" } else { "FAIL" });
        }
        "transactions" => {
            let result = simulation_suite.test_reorg_with_pending_transactions().await?;
            info!("Reorg with transactions test completed: {}", if result.success { "PASS" } else { "FAIL" });
        }
        _ => {
            println!("Usage: reorg-simulation [comprehensive|simple|deep|transactions]");
            return Ok(());
        }
    }

    Ok(())
}