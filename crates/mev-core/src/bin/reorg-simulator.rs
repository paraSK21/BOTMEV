//! Blockchain reorganization simulator for testing MEV bot resilience

use anyhow::Result;
use chrono::Utc;
use mev_core::{BlockHeader, PendingTransactionState, StateManager, StateManagerConfig, TransactionStatus};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Reorg simulation scenarios
#[derive(Debug, Clone)]
enum ReorgScenario {
    SimpleReorg { depth: u64 },
    DeepReorg { depth: u64 },
    MultipleReorgs { count: u32, depth: u64 },
    ChainSplit { duration_blocks: u64 },
}

/// Reorg simulator for testing state management
struct ReorgSimulator {
    state_manager: StateManager,
    current_block_number: u64,
    scenario: ReorgScenario,
}

impl ReorgSimulator {
    fn new(scenario: ReorgScenario) -> Self {
        let config = StateManagerConfig {
            max_pending_transactions: 1000,
            max_block_history: 100,
            transaction_ttl_seconds: 300,
            reorg_detection_depth: 64,
            checkpoint_interval_seconds: 30,
            enable_persistence: false, // Disable for simulation
            database_path: "test_reorg.db".to_string(),
        };

        Self {
            state_manager: StateManager::new(config),
            current_block_number: 0,
            scenario,
        }
    }

    /// Run the reorg simulation
    async fn run_simulation(&mut self) -> Result<()> {
        info!("Starting reorg simulation: {:?}", self.scenario);

        // Initialize with genesis block
        let genesis = BlockHeader::new(0, "0x000".to_string(), "0x".to_string());
        self.state_manager.initialize(genesis).await?;

        // Add some initial pending transactions
        self.add_test_transactions(10).await?;

        // Build initial chain
        self.build_initial_chain(10).await?;

        // Execute the specific scenario
        match &self.scenario {
            ReorgScenario::SimpleReorg { depth } => {
                self.simulate_simple_reorg(*depth).await?;
            }
            ReorgScenario::DeepReorg { depth } => {
                self.simulate_deep_reorg(*depth).await?;
            }
            ReorgScenario::MultipleReorgs { count, depth } => {
                self.simulate_multiple_reorgs(*count, *depth).await?;
            }
            ReorgScenario::ChainSplit { duration_blocks } => {
                self.simulate_chain_split(*duration_blocks).await?;
            }
        }

        // Print final statistics
        self.print_simulation_results();

        Ok(())
    }

    /// Build initial blockchain
    async fn build_initial_chain(&mut self, blocks: u64) -> Result<()> {
        info!("Building initial chain with {} blocks", blocks);

        let mut parent_hash = "0x000".to_string();

        for i in 1..=blocks {
            let block_hash = format!("0x{:064x}", i);
            let block = BlockHeader::new(i, block_hash.clone(), parent_hash);
            
            self.state_manager.update_block_header(block).await?;
            self.current_block_number = i;
            parent_hash = block_hash;

            // Add some transactions for each block
            if i % 3 == 0 {
                self.add_test_transactions(2).await?;
            }

            sleep(Duration::from_millis(100)).await;
        }

        info!("Initial chain built to block {}", self.current_block_number);
        Ok(())
    }

    /// Simulate a simple reorg
    async fn simulate_simple_reorg(&mut self, depth: u64) -> Result<()> {
        info!("Simulating simple reorg of depth {}", depth);

        let reorg_point = self.current_block_number - depth;
        info!("Reorg point: block {}", reorg_point);

        // Create alternative chain from reorg point
        let mut parent_hash = format!("0x{:064x}", reorg_point);
        
        for i in (reorg_point + 1)..=(self.current_block_number + 2) {
            let alt_block_hash = format!("0xALT{:060x}", i);
            let alt_block = BlockHeader::new(i, alt_block_hash.clone(), parent_hash);
            
            info!("Adding alternative block {}: {}", i, alt_block_hash);
            
            let reorg_event = self.state_manager.update_block_header(alt_block).await?;
            
            if let Some(event) = reorg_event {
                warn!("Reorg detected at depth {}", event.depth);
            }
            
            parent_hash = alt_block_hash;
            sleep(Duration::from_millis(200)).await;
        }

        Ok(())
    }

    /// Simulate a deep reorg
    async fn simulate_deep_reorg(&mut self, depth: u64) -> Result<()> {
        info!("Simulating deep reorg of depth {}", depth);

        // Similar to simple reorg but with more blocks
        let reorg_point = self.current_block_number.saturating_sub(depth);
        let mut parent_hash = format!("0x{:064x}", reorg_point);
        
        for i in (reorg_point + 1)..=(self.current_block_number + depth) {
            let alt_block_hash = format!("0xDEEP{:059x}", i);
            let alt_block = BlockHeader::new(i, alt_block_hash.clone(), parent_hash);
            
            let reorg_event = self.state_manager.update_block_header(alt_block).await?;
            
            if let Some(event) = reorg_event {
                warn!("Deep reorg detected: depth {}, affected txs: {}", 
                      event.depth, event.affected_transactions.len());
            }
            
            parent_hash = alt_block_hash;
            
            // Add transactions during reorg
            if i % 2 == 0 {
                self.add_test_transactions(1).await?;
            }
            
            sleep(Duration::from_millis(150)).await;
        }

        Ok(())
    }

    /// Simulate multiple reorgs
    async fn simulate_multiple_reorgs(&mut self, count: u32, depth: u64) -> Result<()> {
        info!("Simulating {} reorgs of depth {}", count, depth);

        for reorg_num in 1..=count {
            info!("Starting reorg #{}", reorg_num);
            
            // Build some blocks normally
            self.build_normal_blocks(5).await?;
            
            // Then cause a reorg
            let reorg_point = self.current_block_number - depth;
            let mut parent_hash = format!("0x{:064x}", reorg_point);
            
            for i in (reorg_point + 1)..=(self.current_block_number + 1) {
                let alt_block_hash = format!("0xR{}{:058x}", reorg_num, i);
                let alt_block = BlockHeader::new(i, alt_block_hash.clone(), parent_hash);
                
                let reorg_event = self.state_manager.update_block_header(alt_block).await?;
                
                if let Some(event) = reorg_event {
                    warn!("Reorg #{} detected: depth {}", reorg_num, event.depth);
                }
                
                parent_hash = alt_block_hash;
                sleep(Duration::from_millis(100)).await;
            }
            
            sleep(Duration::from_secs(1)).await;
        }

        Ok(())
    }

    /// Simulate chain split scenario
    async fn simulate_chain_split(&mut self, duration_blocks: u64) -> Result<()> {
        info!("Simulating chain split for {} blocks", duration_blocks);

        let split_point = self.current_block_number;
        info!("Chain split at block {}", split_point);

        // Create two competing chains
        let mut chain_a_parent = format!("0x{:064x}", split_point);
        let mut chain_b_parent = format!("0x{:064x}", split_point);

        for i in 1..=duration_blocks {
            let block_num = split_point + i;
            
            // Chain A block
            let chain_a_hash = format!("0xCHA{:060x}", block_num);
            let chain_a_block = BlockHeader::new(block_num, chain_a_hash.clone(), chain_a_parent.clone());
            
            // Chain B block (alternative)
            let chain_b_hash = format!("0xCHB{:060x}", block_num);
            let chain_b_block = BlockHeader::new(block_num, chain_b_hash.clone(), chain_b_parent.clone());
            
            // Randomly choose which chain to follow
            let follow_chain_a = (i + split_point) % 3 != 0;
            
            if follow_chain_a {
                info!("Following chain A at block {}", block_num);
                let reorg_event = self.state_manager.update_block_header(chain_a_block).await?;
                if let Some(event) = reorg_event {
                    warn!("Chain split reorg: depth {}", event.depth);
                }
                chain_a_parent = chain_a_hash;
            } else {
                info!("Switching to chain B at block {}", block_num);
                let reorg_event = self.state_manager.update_block_header(chain_b_block).await?;
                if let Some(event) = reorg_event {
                    warn!("Chain split reorg: depth {}", event.depth);
                }
                chain_b_parent = chain_b_hash;
            }
            
            self.current_block_number = block_num;
            
            // Add transactions during split
            self.add_test_transactions(1).await?;
            sleep(Duration::from_millis(300)).await;
        }

        Ok(())
    }

    /// Build normal blocks without reorgs
    async fn build_normal_blocks(&mut self, count: u64) -> Result<()> {
        let mut parent_hash = format!("0x{:064x}", self.current_block_number);

        for i in 1..=count {
            let block_num = self.current_block_number + i;
            let block_hash = format!("0x{:064x}", block_num);
            let block = BlockHeader::new(block_num, block_hash.clone(), parent_hash);
            
            self.state_manager.update_block_header(block).await?;
            parent_hash = block_hash;
        }

        self.current_block_number += count;
        Ok(())
    }

    /// Add test transactions to the state
    async fn add_test_transactions(&self, count: u32) -> Result<()> {
        for i in 0..count {
            let tx_hash = format!("0x{:064x}", 
                (self.current_block_number * 1000 + i as u64) * 12345);
            
            let tx = PendingTransactionState {
                hash: tx_hash.clone(),
                from: format!("0x{:040x}", i % 10),
                to: Some(format!("0x{:040x}", (i + 1) % 10)),
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

            self.state_manager.update_pending_transaction(tx_hash, tx).await?;
        }

        Ok(())
    }

    /// Print simulation results
    fn print_simulation_results(&self) {
        let stats = self.state_manager.get_stats();
        let reorg_events = self.state_manager.get_reorg_events(100);

        println!("\n=== Reorg Simulation Results ===");
        println!("Scenario: {:?}", self.scenario);
        println!("Final block number: {}", self.current_block_number);
        
        stats.print_summary();
        
        println!("\nReorg Events:");
        for (i, event) in reorg_events.iter().enumerate() {
            println!("  {}. Depth: {}, Affected TXs: {}, Time: {}", 
                     i + 1, event.depth, event.affected_transactions.len(), event.detected_at);
        }

        println!("\nSimulation completed successfully!");
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("MEV Bot Reorg Simulator");

    // Run different scenarios
    let scenarios = vec![
        ReorgScenario::SimpleReorg { depth: 3 },
        ReorgScenario::DeepReorg { depth: 10 },
        ReorgScenario::MultipleReorgs { count: 3, depth: 2 },
        ReorgScenario::ChainSplit { duration_blocks: 8 },
    ];

    for scenario in scenarios {
        let mut simulator = ReorgSimulator::new(scenario);
        
        if let Err(e) = simulator.run_simulation().await {
            warn!("Simulation failed: {}", e);
        }
        
        // Wait between scenarios
        sleep(Duration::from_secs(2)).await;
    }

    info!("All reorg simulations completed!");
    Ok(())
}