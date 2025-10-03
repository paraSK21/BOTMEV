use mev_hyperliquid::{
    HyperLiquidConfig, HyperLiquidMetrics, HyperLiquidWsService, MessageData, TradeData,
    TradeSide,
};
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message};

/// Mock HyperLiquid WebSocket server for testing
struct MockHyperLiquidServer {
    listener: TcpListener,
}

impl MockHyperLiquidServer {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        Self { listener }
    }

    fn addr(&self) -> String {
        format!("ws://{}", self.listener.local_addr().unwrap())
    }

    async fn accept_connection(&self) -> tokio_tungstenite::WebSocketStream<tokio::net::TcpStream> {
        let (stream, _) = self.listener.accept().await.unwrap();
        accept_async(stream).await.unwrap()
    }

    async fn send_subscription_response(
        ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        coin: &str,
        subscription_type: &str,
        success: bool,
    ) {
        let response = serde_json::json!({
            "channel": "subscriptionResponse",
            "data": {
                "type": subscription_type,
                "coin": coin,
                "success": success
            }
        });
        ws.send(Message::Text(response.to_string()))
            .await
            .unwrap();
    }

    async fn send_trade(
        ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        coin: &str,
        side: &str,
        price: f64,
        size: f64,
    ) {
        let trade = serde_json::json!({
            "channel": "trades",
            "data": {
                "coin": coin,
                "side": side,
                "px": price.to_string(),
                "sz": size.to_string(),
                "time": 1696348800000u64,
                "tid": "test_123"
            }
        });
        ws.send(Message::Text(trade.to_string())).await.unwrap();
    }

    async fn send_orderbook(
        ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        coin: &str,
    ) {
        let orderbook = serde_json::json!({
            "channel": "l2Book",
            "data": {
                "coin": coin,
                "bids": [
                    {"px": "45000.0", "sz": "1.0"},
                    {"px": "44999.0", "sz": "2.0"}
                ],
                "asks": [
                    {"px": "45001.0", "sz": "1.5"},
                    {"px": "45002.0", "sz": "2.5"}
                ],
                "time": 1696348800000u64
            }
        });
        ws.send(Message::Text(orderbook.to_string()))
            .await
            .unwrap();
    }
}

fn create_test_config(ws_url: String) -> HyperLiquidConfig {
    HyperLiquidConfig {
        enabled: true,
        ws_url,
        trading_pairs: vec!["BTC".to_string()],
        subscribe_orderbook: false,
        reconnect_min_backoff_secs: 1,
        reconnect_max_backoff_secs: 5,
        max_consecutive_failures: 3,
        token_mapping: Default::default(),
    }
}

#[tokio::test]
async fn test_connection_establishment_and_subscription() {
    // Start mock server
    let server = MockHyperLiquidServer::new().await;
    let ws_url = server.addr();

    // Create service
    let config = create_test_config(ws_url);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let metrics = Arc::new(HyperLiquidMetrics::new().unwrap());
    let service = Arc::new(HyperLiquidWsService::new(config, tx, metrics).unwrap());

    // Start service in background
    let service_clone = service.clone();
    let service_handle = tokio::spawn(async move {
        service_clone.start().await
    });

    // Accept connection from service
    let mut ws = server.accept_connection().await;

    // Wait for subscription message
    let msg = timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("Timeout waiting for subscription")
        .expect("No message received")
        .expect("WebSocket error");

    // Verify subscription message
    if let Message::Text(text) = msg {
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["method"], "subscribe");
        assert_eq!(json["subscription"]["type"], "trades");
        assert_eq!(json["subscription"]["coin"], "BTC");
    } else {
        panic!("Expected text message");
    }

    // Send subscription confirmation
    server
        .send_subscription_response(&mut ws, "BTC", "trades", true)
        .await;

    // Verify active subscriptions
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(service.active_subscriptions().await, 1);

    // Cleanup
    drop(ws);
    drop(rx);
    service_handle.abort();
}

#[tokio::test]
async fn test_message_reception_and_parsing() {
    // Start mock server
    let server = MockHyperLiquidServer::new().await;
    let ws_url = server.addr();

    // Create service
    let config = create_test_config(ws_url);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let metrics = Arc::new(HyperLiquidMetrics::new().unwrap());
    let service = Arc::new(HyperLiquidWsService::new(config, tx, metrics).unwrap());

    // Start service in background
    let service_clone = service.clone();
    let service_handle = tokio::spawn(async move {
        service_clone.start().await
    });

    // Accept connection and handle subscription
    let mut ws = server.accept_connection().await;
    let _ = ws.next().await; // Skip subscription message
    server
        .send_subscription_response(&mut ws, "BTC", "trades", true)
        .await;

    // Send a trade message
    server.send_trade(&mut ws, "BTC", "B", 45000.5, 0.5).await;

    // Receive and verify the parsed message
    let received = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("Timeout waiting for message")
        .expect("Channel closed");

    assert!(received.is_trade());
    if let MessageData::Trade(trade) = received.data {
        assert_eq!(trade.coin, "BTC");
        assert_eq!(trade.side, TradeSide::Buy);
        assert_eq!(trade.price, 45000.5);
        assert_eq!(trade.size, 0.5);
    } else {
        panic!("Expected trade message");
    }

    // Cleanup
    drop(ws);
    drop(rx);
    service_handle.abort();
}

#[tokio::test]
async fn test_reconnection_on_disconnect() {
    // Start mock server
    let server = MockHyperLiquidServer::new().await;
    let ws_url = server.addr();

    // Create service with short backoff for testing
    let mut config = create_test_config(ws_url);
    config.reconnect_min_backoff_secs = 1;
    config.reconnect_max_backoff_secs = 2;

    let (tx, _rx) = mpsc::unbounded_channel();
    let metrics = Arc::new(HyperLiquidMetrics::new().unwrap());
    let service = Arc::new(HyperLiquidWsService::new(config, tx, metrics.clone()).unwrap());

    // Start service in background
    let service_clone = service.clone();
    let service_handle = tokio::spawn(async move {
        service_clone.start().await
    });

    // Accept first connection
    let mut ws1 = server.accept_connection().await;
    let _ = ws1.next().await; // Skip subscription message

    // Close connection to trigger reconnection
    drop(ws1);

    // Wait for reconnection attempt
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Accept second connection
    let result = timeout(Duration::from_secs(5), server.accept_connection()).await;
    assert!(result.is_ok(), "Service should reconnect after disconnect");

    // Verify reconnection metrics
    // Note: In a real test, you'd check the metrics here

    // Cleanup
    service_handle.abort();
}

#[tokio::test]
async fn test_orderbook_subscription() {
    // Start mock server
    let server = MockHyperLiquidServer::new().await;
    let ws_url = server.addr();

    // Create service with orderbook enabled
    let mut config = create_test_config(ws_url);
    config.subscribe_orderbook = true;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let metrics = Arc::new(HyperLiquidMetrics::new().unwrap());
    let service = Arc::new(HyperLiquidWsService::new(config, tx, metrics).unwrap());

    // Start service in background
    let service_clone = service.clone();
    let service_handle = tokio::spawn(async move {
        service_clone.start().await
    });

    // Accept connection
    let mut ws = server.accept_connection().await;

    // Should receive two subscription messages (trades and l2Book)
    let msg1 = timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("Timeout")
        .expect("No message")
        .expect("WebSocket error");

    let msg2 = timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("Timeout")
        .expect("No message")
        .expect("WebSocket error");

    // Verify both subscription types
    let mut found_trades = false;
    let mut found_orderbook = false;

    for msg in [msg1, msg2] {
        if let Message::Text(text) = msg {
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            let sub_type = json["subscription"]["type"].as_str().unwrap();
            if sub_type == "trades" {
                found_trades = true;
            } else if sub_type == "l2Book" {
                found_orderbook = true;
            }
        }
    }

    assert!(found_trades, "Should subscribe to trades");
    assert!(found_orderbook, "Should subscribe to orderbook");

    // Send confirmations
    server
        .send_subscription_response(&mut ws, "BTC", "trades", true)
        .await;
    server
        .send_subscription_response(&mut ws, "BTC", "l2Book", true)
        .await;

    // Send orderbook data
    server.send_orderbook(&mut ws, "BTC").await;

    // Receive and verify orderbook message
    let received = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("Timeout")
        .expect("Channel closed");

    assert!(received.is_orderbook());

    // Cleanup
    drop(ws);
    drop(rx);
    service_handle.abort();
}

#[tokio::test]
async fn test_error_handling_malformed_message() {
    // Start mock server
    let server = MockHyperLiquidServer::new().await;
    let ws_url = server.addr();

    // Create service
    let config = create_test_config(ws_url);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let metrics = Arc::new(HyperLiquidMetrics::new().unwrap());
    let service = Arc::new(HyperLiquidWsService::new(config, tx, metrics.clone()).unwrap());

    // Start service in background
    let service_clone = service.clone();
    let service_handle = tokio::spawn(async move {
        service_clone.start().await
    });

    // Accept connection and handle subscription
    let mut ws = server.accept_connection().await;
    let _ = ws.next().await; // Skip subscription message
    server
        .send_subscription_response(&mut ws, "BTC", "trades", true)
        .await;

    // Send malformed message
    ws.send(Message::Text("{ invalid json }".to_string()))
        .await
        .unwrap();

    // Send valid message after malformed one
    server.send_trade(&mut ws, "BTC", "B", 45000.0, 1.0).await;

    // Should still receive the valid message (error handling should not crash)
    let received = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("Timeout")
        .expect("Channel closed");

    assert!(received.is_trade());

    // Cleanup
    drop(ws);
    drop(rx);
    service_handle.abort();
}

#[tokio::test]
async fn test_subscription_retry_on_failure() {
    // Start mock server
    let server = MockHyperLiquidServer::new().await;
    let ws_url = server.addr();

    // Create service
    let config = create_test_config(ws_url);
    let (tx, _rx) = mpsc::unbounded_channel();
    let metrics = Arc::new(HyperLiquidMetrics::new().unwrap());
    let service = Arc::new(HyperLiquidWsService::new(config, tx, metrics).unwrap());

    // Start service in background
    let service_clone = service.clone();
    let service_handle = tokio::spawn(async move {
        service_clone.start().await
    });

    // Accept connection
    let mut ws = server.accept_connection().await;
    let _ = ws.next().await; // Skip first subscription message

    // Send failure response
    server
        .send_subscription_response(&mut ws, "BTC", "trades", false)
        .await;

    // Wait for retry (should happen after 5 seconds)
    let retry_msg = timeout(Duration::from_secs(7), ws.next())
        .await
        .expect("Timeout waiting for retry")
        .expect("No message")
        .expect("WebSocket error");

    // Verify it's a subscription retry
    if let Message::Text(text) = retry_msg {
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["method"], "subscribe");
    } else {
        panic!("Expected subscription retry");
    }

    // Cleanup
    drop(ws);
    service_handle.abort();
}

#[tokio::test]
async fn test_multiple_trading_pairs() {
    // Start mock server
    let server = MockHyperLiquidServer::new().await;
    let ws_url = server.addr();

    // Create service with multiple pairs
    let mut config = create_test_config(ws_url);
    config.trading_pairs = vec!["BTC".to_string(), "ETH".to_string()];

    let (tx, mut rx) = mpsc::unbounded_channel();
    let metrics = Arc::new(HyperLiquidMetrics::new().unwrap());
    let service = Arc::new(HyperLiquidWsService::new(config, tx, metrics).unwrap());

    // Start service in background
    let service_clone = service.clone();
    let service_handle = tokio::spawn(async move {
        service_clone.start().await
    });

    // Accept connection
    let mut ws = server.accept_connection().await;

    // Should receive subscription messages for both pairs
    let msg1 = timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("Timeout")
        .expect("No message")
        .expect("WebSocket error");

    let msg2 = timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("Timeout")
        .expect("No message")
        .expect("WebSocket error");

    // Verify both coins are subscribed
    let mut found_btc = false;
    let mut found_eth = false;

    for msg in [msg1, msg2] {
        if let Message::Text(text) = msg {
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            let coin = json["subscription"]["coin"].as_str().unwrap();
            if coin == "BTC" {
                found_btc = true;
            } else if coin == "ETH" {
                found_eth = true;
            }
        }
    }

    assert!(found_btc, "Should subscribe to BTC");
    assert!(found_eth, "Should subscribe to ETH");

    // Send confirmations
    server
        .send_subscription_response(&mut ws, "BTC", "trades", true)
        .await;
    server
        .send_subscription_response(&mut ws, "ETH", "trades", true)
        .await;

    // Send trades for both pairs
    server.send_trade(&mut ws, "BTC", "B", 45000.0, 1.0).await;
    server.send_trade(&mut ws, "ETH", "A", 3000.0, 2.0).await;

    // Receive both trades
    let trade1 = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("Timeout")
        .expect("Channel closed");
    let trade2 = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("Timeout")
        .expect("Channel closed");

    // Verify both trades received
    let mut found_btc_trade = false;
    let mut found_eth_trade = false;

    for trade_msg in [trade1, trade2] {
        if let MessageData::Trade(trade) = trade_msg.data {
            if trade.coin == "BTC" {
                found_btc_trade = true;
            } else if trade.coin == "ETH" {
                found_eth_trade = true;
            }
        }
    }

    assert!(found_btc_trade, "Should receive BTC trade");
    assert!(found_eth_trade, "Should receive ETH trade");

    // Cleanup
    drop(ws);
    drop(rx);
    service_handle.abort();
}
