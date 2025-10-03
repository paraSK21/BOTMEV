use anyhow::Result;
use mev_mempool::MempoolService;
use std::env;
use tokio::time::{sleep, Duration};
use tracing::{info, Level};
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("Starting MEV Mempool Consumer Example");

    // Get WebSocket endpoint from environment or use default
    let ws_endpoint = env::var("WS_ENDPOINT")
        .unwrap_or_else(|_| "ws://localhost:8546".to_string());

    // Create mempool service
    let mempool_service = MempoolService::new(ws_endpoint);
    
    // Get a consumer for the ring buffer
    let consumer = mempool_service.get_consumer();

    // Start the mempool service in a background task
    let service_handle = {
        let service = mempool_service.clone();
        tokio::spawn(async move {
            if let Err(e) = service.start().await {
                tracing::error!("Mempool service error: {}", e);
            }
        })
    };

    // Consumer loop - process transactions from the ring buffer
    let consumer_handle = tokio::spawn(async move {
        loop {
            // Try to consume a batch of transactions
            let transactions = consumer.consume_batch(10);
            
            if !transactions.is_empty() {
                info!("Consumed {} transactions from ring buffer", transactions.len());
                
                for tx in transactions {
                    info!(
                        "Processing tx: {} from {} to {:?} (type: {:?})",
                        tx.transaction.hash,
                        tx.transaction.from,
                        tx.transaction.to,
                        tx.target_type
                    );
                }
            }
            
            // Sleep briefly to avoid busy waiting
            sleep(Duration::from_millis(100)).await;
        }
    });

    // Wait for either task to complete (they shouldn't under normal circumstances)
    tokio::select! {
        _ = service_handle => {
            info!("Mempool service task completed");
        }
        _ = consumer_handle => {
            info!("Consumer task completed");
        }
    }

    Ok(())
}
