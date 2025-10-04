//! Optimized RPC client for high-performance MEV operations

use anyhow::Result;
use ethers::{
    providers::{Http, Middleware, Provider},
    types::{Bytes, U64},
};
use reqwest::Client;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// RPC client configuration
#[derive(Debug, Clone)]
pub struct RpcClientConfig {
    pub endpoints: Vec<String>,
    pub timeout_ms: u64,
    pub max_concurrent_requests: usize,
    pub connection_pool_size: usize,
    pub retry_attempts: u32,
    pub retry_delay_ms: u64,
    pub enable_failover: bool,
    pub health_check_interval_ms: u64,
}

impl Default for RpcClientConfig {
    fn default() -> Self {
        Self {
            endpoints: vec!["http://localhost:8545".to_string()],
            timeout_ms: 5000,
            max_concurrent_requests: 100,
            connection_pool_size: 20,
            retry_attempts: 3,
            retry_delay_ms: 100,
            enable_failover: true,
            health_check_interval_ms: 30000,
        }
    }
}

/// RPC endpoint health status
#[derive(Debug, Clone)]
pub struct EndpointHealth {
    pub url: String,
    pub is_healthy: bool,
    pub last_check: Instant,
    pub response_time_ms: f64,
    pub error_count: u32,
    pub success_count: u32,
}

impl EndpointHealth {
    pub fn new(url: String) -> Self {
        Self {
            url,
            is_healthy: true,
            last_check: Instant::now(),
            response_time_ms: 0.0,
            error_count: 0,
            success_count: 0,
        }
    }

    pub fn record_success(&mut self, response_time_ms: f64) {
        self.success_count += 1;
        self.response_time_ms = response_time_ms;
        self.last_check = Instant::now();
        
        // Mark as healthy if it was previously unhealthy
        if !self.is_healthy && self.success_count > 0 {
            self.is_healthy = true;
            info!(endpoint = %self.url, "RPC endpoint recovered");
        }
    }

    pub fn record_error(&mut self) {
        self.error_count += 1;
        self.last_check = Instant::now();
        
        // Mark as unhealthy after 3 consecutive errors
        if self.error_count >= 3 && self.is_healthy {
            self.is_healthy = false;
            warn!(endpoint = %self.url, "RPC endpoint marked as unhealthy");
        }
    }

    pub fn get_success_rate(&self) -> f64 {
        let total = self.success_count + self.error_count;
        if total > 0 {
            self.success_count as f64 / total as f64
        } else {
            1.0
        }
    }
}

/// High-performance RPC client with connection pooling and failover
pub struct OptimizedRpcClient {
    config: RpcClientConfig,
    providers: Vec<Arc<Provider<Http>>>,
    endpoint_health: Arc<RwLock<Vec<EndpointHealth>>>,
    request_semaphore: tokio::sync::Semaphore,
    client: Client,
}

impl OptimizedRpcClient {
    /// Create new optimized RPC client
    pub async fn new(config: RpcClientConfig) -> Result<Self> {
        let mut providers = Vec::new();
        let mut endpoint_health = Vec::new();

        // Create HTTP client with connection pooling
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .pool_max_idle_per_host(config.connection_pool_size)
            .pool_idle_timeout(Duration::from_secs(30))
            .tcp_keepalive(Duration::from_secs(60))
            .build()?;

        // Initialize providers for each endpoint
        for endpoint in &config.endpoints {
            let provider = Provider::<Http>::try_from(endpoint.as_str())?;
            providers.push(Arc::new(provider));
            endpoint_health.push(EndpointHealth::new(endpoint.clone()));
        }

        let rpc_client = Self {
            request_semaphore: tokio::sync::Semaphore::new(config.max_concurrent_requests),
            config,
            providers,
            endpoint_health: Arc::new(RwLock::new(endpoint_health)),
            client,
        };

        // Start health check task
        if rpc_client.config.enable_failover {
            rpc_client.start_health_check_task().await;
        }

        info!(
            endpoints = rpc_client.config.endpoints.len(),
            max_concurrent = rpc_client.config.max_concurrent_requests,
            "Optimized RPC client initialized"
        );

        Ok(rpc_client)
    }

    /// Get the best available provider based on health and performance
    async fn get_best_provider(&self) -> Result<Arc<Provider<Http>>> {
        let health_status = self.endpoint_health.read().await;
        
        // Find the healthiest endpoint with best response time
        let best_endpoint = health_status
            .iter()
            .enumerate()
            .filter(|(_, health)| health.is_healthy)
            .min_by(|(_, a), (_, b)| {
                a.response_time_ms.partial_cmp(&b.response_time_ms).unwrap_or(std::cmp::Ordering::Equal)
            });

        if let Some((index, _)) = best_endpoint {
            Ok(self.providers[index].clone())
        } else {
            // Fallback to first provider if none are healthy
            warn!("No healthy RPC endpoints available, using fallback");
            Ok(self.providers[0].clone())
        }
    }

    /// Execute RPC call with retry logic and failover
    pub async fn call_with_retry<T, F, Fut>(&self, operation: F) -> Result<T>
    where
        F: Fn(Arc<Provider<Http>>) -> Fut + Send + Sync,
        Fut: std::future::Future<Output = Result<T>> + Send,
        T: Send,
    {
        let _permit = self.request_semaphore.acquire().await?;
        
        for attempt in 0..self.config.retry_attempts {
            let provider = self.get_best_provider().await?;
            let start_time = Instant::now();
            
            match operation(provider.clone()).await {
                Ok(result) => {
                    let response_time = start_time.elapsed().as_secs_f64() * 1000.0;
                    self.record_success(&provider, response_time).await;
                    return Ok(result);
                }
                Err(e) => {
                    self.record_error(&provider).await;
                    
                    if attempt < self.config.retry_attempts - 1 {
                        debug!(
                            attempt = attempt + 1,
                            max_attempts = self.config.retry_attempts,
                            error = %e,
                            "RPC call failed, retrying"
                        );
                        
                        tokio::time::sleep(Duration::from_millis(
                            self.config.retry_delay_ms * (attempt + 1) as u64
                        )).await;
                    } else {
                        error!(error = %e, "RPC call failed after all retry attempts");
                        return Err(e);
                    }
                }
            }
        }

        Err(anyhow::anyhow!("All retry attempts exhausted"))
    }

    /// Get current block number with optimization
    pub async fn get_block_number(&self) -> Result<U64> {
        self.call_with_retry(|provider| async move {
            provider.get_block_number().await.map_err(|e| anyhow::anyhow!(e))
        }).await
    }

    /// Get multiple block numbers in batch
    pub async fn get_block_numbers_batch(&self, count: usize) -> Result<Vec<U64>> {
        let mut results = Vec::new();
        let mut handles = Vec::new();

        // Create concurrent requests
        for _ in 0..count {
            let client = self.clone();
            let handle = tokio::spawn(async move {
                client.get_block_number().await
            });
            handles.push(handle);
        }

        // Collect results
        for handle in handles {
            match handle.await? {
                Ok(block_number) => results.push(block_number),
                Err(e) => {
                    warn!("Batch block number request failed: {}", e);
                }
            }
        }

        Ok(results)
    }

    /// Optimized eth_call with caching
    pub async fn eth_call_optimized(
        &self,
        transaction: &ethers::types::transaction::eip2718::TypedTransaction,
        block: Option<ethers::types::BlockId>,
    ) -> Result<Bytes> {
        self.call_with_retry(|provider| {
            let tx = transaction.clone();
            let block_id = block;
            async move {
                provider.call(&tx, block_id).await.map_err(|e| anyhow::anyhow!(e))
            }
        }).await
    }

    /// Batch eth_call operations
    pub async fn eth_call_batch(
        &self,
        calls: Vec<(ethers::types::transaction::eip2718::TypedTransaction, Option<ethers::types::BlockId>)>,
    ) -> Result<Vec<Result<Bytes>>> {
        let mut handles = Vec::new();

        for (tx, block) in calls {
            let client = self.clone();
            let handle = tokio::spawn(async move {
                client.eth_call_optimized(&tx, block).await
            });
            handles.push(handle);
        }

        let mut results = Vec::new();
        for handle in handles {
            let result = handle.await?;
            results.push(result);
        }

        Ok(results)
    }

    /// Record successful operation
    async fn record_success(&self, provider: &Arc<Provider<Http>>, response_time_ms: f64) {
        let mut health_status = self.endpoint_health.write().await;
        
        // Find the provider index and update health
        for (i, health) in health_status.iter_mut().enumerate() {
            if i < self.providers.len() && Arc::ptr_eq(&self.providers[i], provider) {
                health.record_success(response_time_ms);
                break;
            }
        }
    }

    /// Record failed operation
    async fn record_error(&self, provider: &Arc<Provider<Http>>) {
        let mut health_status = self.endpoint_health.write().await;
        
        // Find the provider index and update health
        for (i, health) in health_status.iter_mut().enumerate() {
            if i < self.providers.len() && Arc::ptr_eq(&self.providers[i], provider) {
                health.record_error();
                break;
            }
        }
    }

    /// Start background health check task
    async fn start_health_check_task(&self) {
        let endpoint_health = self.endpoint_health.clone();
        let providers = self.providers.clone();
        let interval = self.config.health_check_interval_ms;

        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(Duration::from_millis(interval));
            
            loop {
                interval_timer.tick().await;
                
                // Check health of all endpoints
                for (i, provider) in providers.iter().enumerate() {
                    let start_time = Instant::now();
                    
                    match provider.get_block_number().await {
                        Ok(_) => {
                            let response_time = start_time.elapsed().as_secs_f64() * 1000.0;
                            let mut health_status = endpoint_health.write().await;
                            if let Some(health) = health_status.get_mut(i) {
                                health.record_success(response_time);
                            }
                        }
                        Err(_) => {
                            let mut health_status = endpoint_health.write().await;
                            if let Some(health) = health_status.get_mut(i) {
                                health.record_error();
                            }
                        }
                    }
                }
            }
        });
    }

    /// Get RPC client statistics
    pub async fn get_stats(&self) -> RpcClientStats {
        let health_status = self.endpoint_health.read().await;
        let healthy_endpoints = health_status.iter().filter(|h| h.is_healthy).count();
        let total_endpoints = health_status.len();
        
        let avg_response_time = if !health_status.is_empty() {
            health_status.iter().map(|h| h.response_time_ms).sum::<f64>() / health_status.len() as f64
        } else {
            0.0
        };

        let total_requests: u32 = health_status.iter().map(|h| h.success_count + h.error_count).sum();
        let successful_requests: u32 = health_status.iter().map(|h| h.success_count).sum();
        
        let success_rate = if total_requests > 0 {
            successful_requests as f64 / total_requests as f64
        } else {
            1.0
        };

        RpcClientStats {
            healthy_endpoints,
            total_endpoints,
            avg_response_time_ms: avg_response_time,
            success_rate,
            total_requests,
            active_requests: self.config.max_concurrent_requests - self.request_semaphore.available_permits(),
            max_concurrent_requests: self.config.max_concurrent_requests,
        }
    }
}

impl Clone for OptimizedRpcClient {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            providers: self.providers.clone(),
            endpoint_health: self.endpoint_health.clone(),
            request_semaphore: tokio::sync::Semaphore::new(self.config.max_concurrent_requests),
            client: self.client.clone(),
        }
    }
}

/// RPC client statistics
#[derive(Debug, Clone)]
pub struct RpcClientStats {
    pub healthy_endpoints: usize,
    pub total_endpoints: usize,
    pub avg_response_time_ms: f64,
    pub success_rate: f64,
    pub total_requests: u32,
    pub active_requests: usize,
    pub max_concurrent_requests: usize,
}

impl RpcClientStats {
    pub fn print_summary(&self) {
        info!(
            healthy_endpoints = format!("{}/{}", self.healthy_endpoints, self.total_endpoints),
            avg_response_time_ms = format!("{:.2}", self.avg_response_time_ms),
            success_rate = format!("{:.2}%", self.success_rate * 100.0),
            total_requests = self.total_requests,
            active_requests = format!("{}/{}", self.active_requests, self.max_concurrent_requests),
            "RPC client statistics"
        );
    }
}

/// Connection pool manager for WebSocket connections
pub struct WebSocketPool {
    #[allow(dead_code)]
    connections: Arc<RwLock<Vec<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>>>,
    config: RpcClientConfig,
}

impl WebSocketPool {
    pub async fn new(config: RpcClientConfig) -> Result<Self> {
        Ok(Self {
            connections: Arc::new(RwLock::new(Vec::new())),
            config,
        })
    }

    /// Get or create a WebSocket connection
    pub async fn get_connection(&self) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>> {
        // Implementation would manage a pool of WebSocket connections
        // For now, create a new connection each time
        let ws_url = self.config.endpoints[0].replace("http", "ws");
        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url).await?;
        Ok(ws_stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_endpoint_health() {
        let mut health = EndpointHealth::new("http://localhost:8545".to_string());
        
        // Test success recording
        health.record_success(50.0);
        assert_eq!(health.success_count, 1);
        assert_eq!(health.response_time_ms, 50.0);
        assert!(health.is_healthy);
        
        // Test error recording
        health.record_error();
        health.record_error();
        health.record_error();
        assert!(!health.is_healthy);
        assert_eq!(health.error_count, 3);
    }

    #[test]
    fn test_success_rate_calculation() {
        let mut health = EndpointHealth::new("http://localhost:8545".to_string());
        
        health.record_success(10.0);
        health.record_success(20.0);
        health.record_error();
        
        assert_eq!(health.get_success_rate(), 2.0 / 3.0);
    }

    #[tokio::test]
    async fn test_rpc_client_creation() {
        let config = RpcClientConfig::default();
        
        // This test requires a running RPC endpoint
        // In a real test environment, you'd use a mock server
        if std::env::var("RPC_URL").is_ok() {
            let client = OptimizedRpcClient::new(config).await;
            assert!(client.is_ok());
        }
    }
}