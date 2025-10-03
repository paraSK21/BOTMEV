use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

/// WebSocket RPC client for Ethereum node communication
pub struct WebSocketRpcClient {
    url: String,
    request_id: std::sync::atomic::AtomicU64,
}

impl WebSocketRpcClient {
    pub fn new(url: String) -> Self {
        Self {
            url,
            request_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Subscribe to pending transactions using eth_subscribe
    pub async fn subscribe_pending_transactions(
        &self,
    ) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>> {
        info!("Connecting to WebSocket: {}", self.url);
        
        let (ws_stream, _) = connect_async(&self.url)
            .await
            .map_err(|e| anyhow!("Failed to connect to WebSocket: {}", e))?;

        info!("WebSocket connected successfully");
        Ok(ws_stream)
    }

    /// Send eth_subscribe request for pending transactions with timeout
    pub async fn send_subscribe_request(
        &self,
        ws_stream: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        timeout_secs: u64,
    ) -> Result<String> {
        let request_id = self.request_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        
        let subscribe_request = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "eth_subscribe",
            "params": ["newPendingTransactions", true]
        });

        debug!("Sending subscribe request: {}", subscribe_request);

        ws_stream
            .send(Message::Text(subscribe_request.to_string()))
            .await
            .map_err(|e| anyhow!("Failed to send subscribe request: {}", e))?;

        // Wait for subscription response with timeout
        let timeout_duration = tokio::time::Duration::from_secs(timeout_secs);
        
        match tokio::time::timeout(timeout_duration, ws_stream.next()).await {
            Ok(Some(message)) => {
                match message {
                    Ok(Message::Text(text)) => {
                        debug!("Received subscription response: {}", text);
                        let response: Value = serde_json::from_str(&text)
                            .map_err(|e| anyhow!("Failed to parse subscription response: {}", e))?;
                        
                        if let Some(result) = response.get("result") {
                            let subscription_id = result.as_str()
                                .ok_or_else(|| anyhow!("Invalid subscription ID format"))?;
                            
                            info!("Successfully subscribed with ID: {}", subscription_id);
                            return Ok(subscription_id.to_string());
                        } else if let Some(error) = response.get("error") {
                            return Err(anyhow!("Subscription error: {}", error));
                        }
                    }
                    Ok(Message::Close(_)) => {
                        return Err(anyhow!("WebSocket connection closed during subscription"));
                    }
                    Err(e) => {
                        return Err(anyhow!("WebSocket error during subscription: {}", e));
                    }
                    _ => {
                        warn!("Received unexpected message type during subscription");
                    }
                }
            }
            Ok(None) => {
                return Err(anyhow!("WebSocket stream ended before receiving subscription response"));
            }
            Err(_) => {
                return Err(anyhow!("Timeout waiting for subscription response after {} seconds", timeout_secs));
            }
        }

        Err(anyhow!("No response received for subscription request"))
    }

    /// Parse incoming transaction notification
    pub fn parse_transaction_notification(&self, message: &str) -> Result<Option<Value>> {
        let notification: Value = serde_json::from_str(message)
            .map_err(|e| anyhow!("Failed to parse notification: {}", e))?;

        // Check if this is a subscription notification
        if notification.get("method").and_then(|m| m.as_str()) == Some("eth_subscription") {
            if let Some(params) = notification.get("params") {
                if let Some(result) = params.get("result") {
                    return Ok(Some(result.clone()));
                }
            }
        }

        Ok(None)
    }

    /// Get next request ID
    pub fn next_request_id(&self) -> u64 {
        self.request_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_transaction_notification() {
        let client = WebSocketRpcClient::new("ws://localhost:8546".to_string());
        
        let notification = r#"{
            "jsonrpc": "2.0",
            "method": "eth_subscription",
            "params": {
                "subscription": "0x123",
                "result": {
                    "hash": "0xabc123",
                    "from": "0x456def",
                    "to": "0x789ghi",
                    "value": "0x1000"
                }
            }
        }"#;

        let result = client.parse_transaction_notification(notification).unwrap();
        assert!(result.is_some());
        
        let tx = result.unwrap();
        assert_eq!(tx.get("hash").unwrap().as_str().unwrap(), "0xabc123");
    }
}
