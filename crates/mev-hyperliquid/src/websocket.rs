use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, Duration, Instant};
use tokio_tungstenite::{
    connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream,
};
use tracing::{debug, error, info, warn};

use crate::config::HyperLiquidConfig;
use crate::metrics::HyperLiquidMetrics;
use crate::parser::TradeMessageParser;
use crate::types::{HyperLiquidMessage, MessageData, ReconnectConfig, SubscriptionResponse, TradingPair};

/// Subscription state for a single trading pair
#[derive(Debug, Clone)]
struct SubscriptionState {
    /// Trading pair configuration
    pair: TradingPair,
    
    /// Number of subscription attempts for trades
    trades_attempts: u32,
    
    /// Number of subscription attempts for order book
    orderbook_attempts: u32,
    
    /// Whether trades subscription is confirmed
    trades_confirmed: bool,
    
    /// Whether order book subscription is confirmed
    orderbook_confirmed: bool,
    
    /// Timestamp of last subscription attempt
    last_attempt: Option<Instant>,
}

impl SubscriptionState {
    fn new(pair: TradingPair) -> Self {
        Self {
            pair,
            trades_attempts: 0,
            orderbook_attempts: 0,
            trades_confirmed: false,
            orderbook_confirmed: false,
            last_attempt: None,
        }
    }
    
    /// Check if all required subscriptions are confirmed
    fn is_fully_subscribed(&self) -> bool {
        let trades_ok = !self.pair.subscribe_trades || self.trades_confirmed;
        let orderbook_ok = !self.pair.subscribe_orderbook || self.orderbook_confirmed;
        trades_ok && orderbook_ok
    }
    
    /// Check if we should retry subscription (after 5 seconds, max 3 attempts)
    fn should_retry(&self, subscription_type: &str) -> bool {
        let attempts = match subscription_type {
            "trades" => self.trades_attempts,
            "l2Book" => self.orderbook_attempts,
            _ => return false,
        };
        
        // Max 3 attempts
        if attempts >= 3 {
            return false;
        }
        
        // Wait at least 5 seconds since last attempt
        match self.last_attempt {
            Some(last) => last.elapsed() >= Duration::from_secs(5),
            None => true,
        }
    }
    
    /// Record a subscription attempt
    fn record_attempt(&mut self, subscription_type: &str) {
        match subscription_type {
            "trades" => self.trades_attempts += 1,
            "l2Book" => self.orderbook_attempts += 1,
            _ => {}
        }
        self.last_attempt = Some(Instant::now());
    }
    
    /// Record a successful subscription confirmation
    fn record_confirmation(&mut self, subscription_type: &str) {
        match subscription_type {
            "trades" => self.trades_confirmed = true,
            "l2Book" => self.orderbook_confirmed = true,
            _ => {}
        }
    }
    
    /// Reset subscription state (for reconnection)
    fn reset(&mut self) {
        self.trades_attempts = 0;
        self.orderbook_attempts = 0;
        self.trades_confirmed = false;
        self.orderbook_confirmed = false;
        self.last_attempt = None;
    }
}

/// Reconnection state tracking
#[derive(Debug)]
struct ReconnectionState {
    /// Number of consecutive connection failures
    consecutive_failures: u32,
    
    /// Timestamp of last successful connection
    last_success: Instant,
    
    /// Whether the service is in degraded state
    degraded: bool,
    
    /// Timestamp of last degraded state warning
    last_degraded_warning: Option<Instant>,
}

impl ReconnectionState {
    fn new() -> Self {
        Self {
            consecutive_failures: 0,
            last_success: Instant::now(),
            degraded: false,
            last_degraded_warning: None,
        }
    }
    
    /// Record a successful connection
    fn record_success(&mut self) {
        self.last_success = Instant::now();
        
        // Reset failure count if connection was stable for 5 minutes
        if self.last_success.elapsed() >= Duration::from_secs(300) {
            if self.consecutive_failures > 0 {
                info!(
                    "Connection stable for 5 minutes, resetting failure count from {}",
                    self.consecutive_failures
                );
            }
            self.consecutive_failures = 0;
        }
        
        // Exit degraded state on successful connection
        if self.degraded {
            info!("Exiting degraded state after successful connection");
            self.degraded = false;
            self.last_degraded_warning = None;
        }
    }
    
    /// Record a connection failure
    fn record_failure(&mut self, max_failures: u32) {
        self.consecutive_failures += 1;
        
        // Enter degraded state if max failures reached
        if self.consecutive_failures >= max_failures && !self.degraded {
            warn!(
                "Reached maximum consecutive failures ({}), entering degraded state",
                max_failures
            );
            self.degraded = true;
            self.last_degraded_warning = Some(Instant::now());
        }
    }
    
    /// Check if we should log a degraded state warning
    fn should_log_degraded_warning(&mut self) -> bool {
        if !self.degraded {
            return false;
        }
        
        match self.last_degraded_warning {
            None => {
                self.last_degraded_warning = Some(Instant::now());
                true
            }
            Some(last) => {
                if last.elapsed() >= Duration::from_secs(1800) {
                    // 30 minutes
                    self.last_degraded_warning = Some(Instant::now());
                    true
                } else {
                    false
                }
            }
        }
    }
    
    /// Get the current backoff duration
    fn get_backoff(&self, config: &ReconnectConfig) -> Duration {
        if self.degraded {
            // In degraded state, retry once per 5 minutes
            Duration::from_secs(300)
        } else {
            // Normal exponential backoff based on consecutive failures
            // Use consecutive_failures directly as the exponent
            let backoff_secs = if self.consecutive_failures == 0 {
                config.min_backoff_secs
            } else {
                let backoff = config.min_backoff_secs * 2_u64.pow(self.consecutive_failures);
                backoff.min(config.max_backoff_secs)
            };
            Duration::from_secs(backoff_secs)
        }
    }
}

/// Message queue for backpressure handling
struct MessageQueue {
    /// Queue of pending messages
    queue: VecDeque<HyperLiquidMessage>,
    
    /// Maximum queue size before dropping oldest messages
    max_size: usize,
    
    /// Counter for dropped messages
    dropped_count: u64,
}

impl MessageQueue {
    fn new(max_size: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(max_size),
            max_size,
            dropped_count: 0,
        }
    }
    
    /// Push a message to the queue, dropping oldest if full
    fn push(&mut self, message: HyperLiquidMessage) {
        if self.queue.len() >= self.max_size {
            // Drop oldest message
            self.queue.pop_front();
            self.dropped_count += 1;
            
            if self.dropped_count % 100 == 0 {
                warn!(
                    "Message queue full, dropped {} messages total",
                    self.dropped_count
                );
            }
        }
        
        self.queue.push_back(message);
    }
    
    /// Pop a message from the queue
    fn pop(&mut self) -> Option<HyperLiquidMessage> {
        self.queue.pop_front()
    }
    
    /// Get the current queue size
    fn len(&self) -> usize {
        self.queue.len()
    }
    
    /// Get the total number of dropped messages
    fn dropped_count(&self) -> u64 {
        self.dropped_count
    }
}

/// WebSocket service for HyperLiquid API
pub struct HyperLiquidWsService {
    /// WebSocket URL
    ws_url: String,
    
    /// Trading pairs to subscribe to
    subscriptions: Vec<TradingPair>,
    
    /// Reconnection configuration
    reconnect_config: ReconnectConfig,
    
    /// Channel sender for parsed messages
    message_tx: mpsc::UnboundedSender<HyperLiquidMessage>,
    
    /// Shutdown signal
    shutdown: Arc<AtomicBool>,
    
    /// Reconnection state
    reconnection_state: Arc<Mutex<ReconnectionState>>,
    
    /// Subscription state tracking
    subscription_states: Arc<Mutex<HashMap<String, SubscriptionState>>>,
    
    /// Total reconnection attempts counter (for metrics)
    total_reconnection_attempts: Arc<AtomicU32>,
    
    /// Message parser
    parser: TradeMessageParser,
    
    /// Message queue for backpressure handling
    message_queue: Arc<Mutex<MessageQueue>>,
    
    /// Total messages received counter (for metrics)
    total_messages_received: Arc<AtomicU64>,
    
    /// Total messages processed counter (for metrics)
    total_messages_processed: Arc<AtomicU64>,
    
    /// Total parse errors counter (for metrics)
    total_parse_errors: Arc<AtomicU64>,
    
    /// Prometheus metrics
    metrics: Arc<HyperLiquidMetrics>,
}

impl HyperLiquidWsService {
    /// Create a new HyperLiquid WebSocket service
    pub fn new(
        config: HyperLiquidConfig,
        message_tx: mpsc::UnboundedSender<HyperLiquidMessage>,
        metrics: Arc<HyperLiquidMetrics>,
    ) -> Result<Self> {
        // Validate configuration
        config.validate()?;
        
        // Convert trading pairs from config
        let subscriptions: Vec<TradingPair> = config
            .trading_pairs
            .iter()
            .map(|coin| {
                if config.subscribe_orderbook {
                    TradingPair::with_orderbook(coin.clone())
                } else {
                    TradingPair::trades_only(coin.clone())
                }
            })
            .collect();
        
        let reconnect_config = ReconnectConfig::new(
            config.reconnect_min_backoff_secs,
            config.reconnect_max_backoff_secs,
            config.max_consecutive_failures,
        );
        
        // Initialize subscription states
        let mut subscription_states = HashMap::new();
        for pair in &subscriptions {
            subscription_states.insert(pair.coin.clone(), SubscriptionState::new(pair.clone()));
        }
        
        // Initialize metrics - start with disconnected state
        metrics.set_ws_connected(&config.ws_url, false);
        metrics.set_degraded_state("max_failures", false);
        
        Ok(Self {
            ws_url: config.ws_url,
            subscriptions,
            reconnect_config,
            message_tx,
            shutdown: Arc::new(AtomicBool::new(false)),
            reconnection_state: Arc::new(Mutex::new(ReconnectionState::new())),
            subscription_states: Arc::new(Mutex::new(subscription_states)),
            total_reconnection_attempts: Arc::new(AtomicU32::new(0)),
            parser: TradeMessageParser::new(),
            message_queue: Arc::new(Mutex::new(MessageQueue::new(1000))),
            total_messages_received: Arc::new(AtomicU64::new(0)),
            total_messages_processed: Arc::new(AtomicU64::new(0)),
            total_parse_errors: Arc::new(AtomicU64::new(0)),
            metrics,
        })
    }
    
    /// Start the WebSocket service
    pub async fn start(&self) -> Result<()> {
        info!("Starting HyperLiquid WebSocket service");
        
        // Initial connection attempt
        let mut is_first_attempt = true;
        
        while !self.shutdown.load(Ordering::Relaxed) {
            // Check if we should log degraded state warning
            {
                let mut state = self.reconnection_state.lock().await;
                if state.should_log_degraded_warning() {
                    warn!(
                        "Service in degraded state: {} consecutive failures, retrying every 5 minutes",
                        state.consecutive_failures
                    );
                }
            }
            
            // Attempt to connect and run
            match self.connect_and_run().await {
                Ok(_) => {
                    info!("WebSocket connection closed gracefully");
                    
                    // Update metrics - disconnected
                    self.metrics.set_ws_connected(&self.ws_url, false);
                    
                    // Record successful connection
                    let mut state = self.reconnection_state.lock().await;
                    state.record_success();
                    
                    // Update degraded state metric if exiting degraded state
                    if state.degraded {
                        self.metrics.set_degraded_state("max_failures", false);
                    }
                }
                Err(e) => {
                    error!("WebSocket connection error: {:#}", e);
                    
                    // Update metrics - connection error
                    self.metrics.set_ws_connected(&self.ws_url, false);
                    self.metrics.inc_connection_errors("connection_failed");
                    
                    // Record failure
                    let mut state = self.reconnection_state.lock().await;
                    let was_degraded = state.degraded;
                    state.record_failure(self.reconnect_config.max_consecutive_failures);
                    
                    // Update degraded state metric if entering degraded state
                    if !was_degraded && state.degraded {
                        self.metrics.set_degraded_state("max_failures", true);
                    }
                    
                    info!(
                        "Connection failure {} of {} before degraded state",
                        state.consecutive_failures,
                        self.reconnect_config.max_consecutive_failures
                    );
                }
            }
            
            // Don't reconnect if shutting down
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }
            
            // Skip backoff on first attempt if it was successful
            if is_first_attempt {
                is_first_attempt = false;
            }
            
            // Calculate backoff and wait before reconnecting
            let backoff = {
                let state = self.reconnection_state.lock().await;
                state.get_backoff(&self.reconnect_config)
            };
            
            // Increment reconnection attempts counter
            let attempt_num = self.total_reconnection_attempts.fetch_add(1, Ordering::Relaxed) + 1;
            
            // Update metrics - reconnection attempt
            let reason = if self.reconnection_state.lock().await.degraded {
                "degraded_state"
            } else {
                "connection_lost"
            };
            self.metrics.inc_reconnection_attempts(reason);
            
            info!(
                "Reconnecting in {} seconds (total attempts: {})",
                backoff.as_secs(),
                attempt_num
            );
            
            // Wait with periodic shutdown checks
            let mut remaining = backoff;
            while remaining > Duration::ZERO && !self.shutdown.load(Ordering::Relaxed) {
                let sleep_duration = remaining.min(Duration::from_secs(1));
                sleep(sleep_duration).await;
                remaining = remaining.saturating_sub(sleep_duration);
            }
        }
        
        info!("HyperLiquid WebSocket service stopped");
        Ok(())
    }

    /// Establish WebSocket connection and run the message loop
    async fn connect_and_run(&self) -> Result<()> {
        info!("Connecting to HyperLiquid WebSocket: {}", self.ws_url);
        
        // Establish connection
        let ws_stream = self.connect().await?;
        
        info!("Successfully connected to HyperLiquid WebSocket");
        
        // Update metrics - connected
        self.metrics.set_ws_connected(&self.ws_url, true);
        
        // Run the message processing loop
        let result = self.run_message_loop(ws_stream).await;
        
        // Update metrics - disconnected
        self.metrics.set_ws_connected(&self.ws_url, false);
        
        result
    }
    
    /// Establish WebSocket connection
    async fn connect(&self) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>> {
        let (ws_stream, response) = connect_async(&self.ws_url)
            .await
            .context("Failed to connect to WebSocket")?;
        
        debug!("WebSocket handshake response: {:?}", response);
        
        Ok(ws_stream)
    }
    
    /// Run the main message processing loop
    async fn run_message_loop(
        &self,
        mut ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    ) -> Result<()> {
        // Reset subscription states on new connection
        {
            let mut states = self.subscription_states.lock().await;
            for state in states.values_mut() {
                state.reset();
            }
        }
        
        // Subscribe to all configured trading pairs
        for pair in &self.subscriptions {
            if let Err(e) = self.subscribe_with_retry(&mut ws_stream, pair).await {
                error!("Failed to subscribe to {}: {:#}", pair.coin, e);
            }
        }
        
        // Set up ping interval for connection health check
        let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
        ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        
        // Set up subscription retry interval
        let mut retry_interval = tokio::time::interval(Duration::from_secs(5));
        retry_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        
        loop {
            tokio::select! {
                // Handle incoming messages
                msg = ws_stream.next() => {
                    match msg {
                        Some(Ok(message)) => {
                            if let Err(e) = self.handle_message(message).await {
                                error!("Error handling message: {:#}", e);
                            }
                        }
                        Some(Err(e)) => {
                            error!("WebSocket error: {:#}", e);
                            self.metrics.inc_network_errors("stream_error");
                            return Err(anyhow!("WebSocket stream error: {}", e));
                        }
                        None => {
                            info!("WebSocket stream closed");
                            return Ok(());
                        }
                    }
                }
                
                // Send periodic pings for connection health check
                _ = ping_interval.tick() => {
                    debug!("Sending ping to keep connection alive");
                    if let Err(e) = ws_stream.send(Message::Ping(vec![])).await {
                        error!("Failed to send ping: {:#}", e);
                        self.metrics.inc_network_errors("ping_failed");
                        return Err(anyhow!("Failed to send ping: {}", e));
                    }
                }
                
                // Check for subscription retries
                _ = retry_interval.tick() => {
                    if let Err(e) = self.retry_failed_subscriptions(&mut ws_stream).await {
                        error!("Error retrying subscriptions: {:#}", e);
                    }
                }
                
                // Check for shutdown signal
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if self.shutdown.load(Ordering::Relaxed) {
                        info!("Shutdown signal received, closing connection");
                        return self.close(ws_stream).await;
                    }
                }
            }
        }
    }
    
    /// Subscribe to a trading pair with retry logic
    async fn subscribe_with_retry(
        &self,
        ws_stream: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        pair: &TradingPair,
    ) -> Result<()> {
        // Subscribe to trades if enabled
        if pair.subscribe_trades {
            self.send_subscription(ws_stream, &pair.coin, "trades").await?;
        }
        
        // Subscribe to order book if enabled
        if pair.subscribe_orderbook {
            self.send_subscription(ws_stream, &pair.coin, "l2Book").await?;
        }
        
        Ok(())
    }
    
    /// Send a subscription message for a specific type
    async fn send_subscription(
        &self,
        ws_stream: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
        coin: &str,
        subscription_type: &str,
    ) -> Result<()> {
        // Record the attempt
        {
            let mut states = self.subscription_states.lock().await;
            if let Some(state) = states.get_mut(coin) {
                state.record_attempt(subscription_type);
            }
        }
        
        let subscription_msg = serde_json::json!({
            "method": "subscribe",
            "subscription": {
                "type": subscription_type,
                "coin": coin
            }
        });
        
        let msg_str = serde_json::to_string(&subscription_msg)?;
        info!("Subscribing to {} for {}: {}", subscription_type, coin, msg_str);
        
        ws_stream
            .send(Message::Text(msg_str))
            .await
            .context(format!("Failed to send {} subscription for {}", subscription_type, coin))?;
        
        Ok(())
    }
    
    /// Retry failed subscriptions
    async fn retry_failed_subscriptions(
        &self,
        ws_stream: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    ) -> Result<()> {
        let mut to_retry = Vec::new();
        
        // Check which subscriptions need retry
        {
            let states = self.subscription_states.lock().await;
            for (coin, state) in states.iter() {
                // Check trades subscription
                if state.pair.subscribe_trades && !state.trades_confirmed && state.should_retry("trades") {
                    to_retry.push((coin.clone(), "trades"));
                }
                
                // Check order book subscription
                if state.pair.subscribe_orderbook && !state.orderbook_confirmed && state.should_retry("l2Book") {
                    to_retry.push((coin.clone(), "l2Book"));
                }
            }
        }
        
        // Retry subscriptions
        for (coin, subscription_type) in to_retry {
            info!("Retrying {} subscription for {}", subscription_type, coin);
            if let Err(e) = self.send_subscription(ws_stream, &coin, subscription_type).await {
                error!("Failed to retry {} subscription for {}: {:#}", subscription_type, coin, e);
            }
        }
        
        Ok(())
    }
    
    /// Handle subscription confirmation
    async fn handle_subscription_response(&self, response: SubscriptionResponse) {
        let mut states = self.subscription_states.lock().await;
        
        if let Some(state) = states.get_mut(&response.coin) {
            if response.success {
                info!(
                    "Subscription confirmed for {} ({})",
                    response.coin, response.subscription_type
                );
                state.record_confirmation(&response.subscription_type);
                
                // Update active subscriptions metric
                let active_count = states.values().filter(|s| s.is_fully_subscribed()).count() as i64;
                self.metrics.set_active_subscriptions("all", active_count);
            } else {
                warn!(
                    "Subscription failed for {} ({}): {:?}",
                    response.coin, response.subscription_type, response.error
                );
                
                // Update metrics - subscription error
                self.metrics.inc_subscription_errors(&response.coin, &response.subscription_type);
            }
        }
    }
    
    /// Get the number of active subscriptions
    pub async fn active_subscriptions(&self) -> usize {
        let states = self.subscription_states.lock().await;
        states.values().filter(|s| s.is_fully_subscribed()).count()
    }
    
    /// Handle incoming WebSocket message
    async fn handle_message(&self, message: Message) -> Result<()> {
        match message {
            Message::Text(text) => {
                // Track message receipt
                self.total_messages_received.fetch_add(1, Ordering::Relaxed);
                
                // Start latency tracking
                let start_time = Instant::now();
                
                debug!("Received text message: {}", text);
                
                // Parse the message using TradeMessageParser
                match self.parser.parse_message(&text) {
                    Ok(parsed_message) => {
                        // Handle different message types
                        let message_type = match &parsed_message.data {
                            MessageData::SubscriptionResponse(response) => {
                                // Handle subscription confirmation internally
                                self.handle_subscription_response(response.clone()).await;
                                "subscription_response"
                            }
                            MessageData::Error(error) => {
                                // Log error messages from HyperLiquid
                                error!(
                                    "Received error from HyperLiquid: code={:?}, message={}",
                                    error.code, error.message
                                );
                                "error"
                            }
                            MessageData::Trade(trade) => {
                                // Update metrics - trade received
                                let side = match trade.side {
                                    crate::types::TradeSide::Buy => "buy",
                                    crate::types::TradeSide::Sell => "sell",
                                };
                                self.metrics.inc_trades_received(&trade.coin, side);
                                
                                // Add to message queue with backpressure handling
                                {
                                    let mut queue = self.message_queue.lock().await;
                                    queue.push(parsed_message.clone());
                                }
                                
                                // Try to send queued messages to channel
                                self.process_message_queue().await;
                                "trade"
                            }
                            MessageData::OrderBook(_) => {
                                // Add to message queue with backpressure handling
                                {
                                    let mut queue = self.message_queue.lock().await;
                                    queue.push(parsed_message.clone());
                                }
                                
                                // Try to send queued messages to channel
                                self.process_message_queue().await;
                                "orderbook"
                            }
                        };
                        
                        // Track processing latency
                        let latency = start_time.elapsed();
                        self.metrics.record_message_processing_duration(message_type, latency);
                        
                        if latency > Duration::from_millis(10) {
                            warn!(
                                "Message processing took {}ms (target: <10ms)",
                                latency.as_millis()
                            );
                        }
                        
                        debug!(
                            "Message processed in {}μs",
                            latency.as_micros()
                        );
                    }
                    Err(e) => {
                        // Track parse errors
                        self.total_parse_errors.fetch_add(1, Ordering::Relaxed);
                        self.metrics.inc_parse_errors("unknown");
                        
                        // Log error with raw message for debugging
                        error!(
                            "Failed to parse message: {:#}. Raw message: {}",
                            e, text
                        );
                        
                        // Continue processing other messages
                    }
                }
            }
            Message::Binary(data) => {
                debug!("Received binary message: {} bytes", data.len());
            }
            Message::Ping(data) => {
                debug!("Received ping: {} bytes", data.len());
            }
            Message::Pong(data) => {
                debug!("Received pong: {} bytes", data.len());
            }
            Message::Close(frame) => {
                info!("Received close frame: {:?}", frame);
                return Err(anyhow!("Connection closed by server"));
            }
            Message::Frame(_) => {
                debug!("Received raw frame");
            }
        }
        
        Ok(())
    }
    
    /// Process messages from the queue and send to channel
    async fn process_message_queue(&self) {
        let mut queue = self.message_queue.lock().await;
        
        // Process all queued messages
        while let Some(message) = queue.pop() {
            // Try to send to channel (non-blocking)
            match self.message_tx.send(message) {
                Ok(_) => {
                    self.total_messages_processed.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    error!("Failed to send message to channel: {:#}", e);
                    // If channel is closed, we should stop processing
                    break;
                }
            }
        }
    }
    
    /// Try to parse a message as a subscription response
    #[allow(dead_code)]
    fn try_parse_subscription_response(&self, text: &str) -> Result<SubscriptionResponse> {
        // Parse the JSON message
        let value: serde_json::Value = serde_json::from_str(text)?;
        
        // Check if this is a subscription response
        // HyperLiquid sends responses in format: {"channel": "subscriptions", "data": {...}}
        if let Some(channel) = value.get("channel").and_then(|c| c.as_str()) {
            if channel == "subscriptions" || channel == "subscriptionResponse" {
                if let Some(data) = value.get("data") {
                    // Extract subscription details
                    let subscription_type = data
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    
                    let coin = data
                        .get("coin")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string();
                    
                    let success = data
                        .get("success")
                        .and_then(|s| s.as_bool())
                        .unwrap_or(true); // Assume success if not specified
                    
                    let error = data
                        .get("error")
                        .and_then(|e| e.as_str())
                        .map(|s| s.to_string());
                    
                    return Ok(SubscriptionResponse {
                        subscription_type,
                        coin,
                        success,
                        error,
                    });
                }
            }
        }
        
        Err(anyhow!("Not a subscription response"))
    }
    
    /// Gracefully close the WebSocket connection
    async fn close(
        &self,
        mut ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    ) -> Result<()> {
        info!("Closing WebSocket connection gracefully");
        
        if let Err(e) = ws_stream.close(None).await {
            warn!("Error closing WebSocket: {:#}", e);
        } else {
            info!("WebSocket connection closed successfully");
        }
        
        Ok(())
    }
    
    /// Signal the service to shut down
    pub fn shutdown(&self) {
        info!("Shutdown requested for HyperLiquid WebSocket service");
        self.shutdown.store(true, Ordering::Relaxed);
    }
    
    /// Check if the service is in degraded state
    pub async fn is_degraded(&self) -> bool {
        let state = self.reconnection_state.lock().await;
        state.degraded
    }
    
    /// Get the number of consecutive failures
    pub async fn consecutive_failures(&self) -> u32 {
        let state = self.reconnection_state.lock().await;
        state.consecutive_failures
    }
    
    /// Get the total number of reconnection attempts
    pub fn total_reconnection_attempts(&self) -> u32 {
        self.total_reconnection_attempts.load(Ordering::Relaxed)
    }
    
    /// Get the total number of messages received
    pub fn total_messages_received(&self) -> u64 {
        self.total_messages_received.load(Ordering::Relaxed)
    }
    
    /// Get the total number of messages processed
    pub fn total_messages_processed(&self) -> u64 {
        self.total_messages_processed.load(Ordering::Relaxed)
    }
    
    /// Get the total number of parse errors
    pub fn total_parse_errors(&self) -> u64 {
        self.total_parse_errors.load(Ordering::Relaxed)
    }
    
    /// Get the current message queue size
    pub async fn message_queue_size(&self) -> usize {
        let queue = self.message_queue.lock().await;
        queue.len()
    }
    
    /// Get the total number of dropped messages due to backpressure
    pub async fn dropped_messages(&self) -> u64 {
        let queue = self.message_queue.lock().await;
        queue.dropped_count()
    }
    
    /// Manually trigger a reconnection (for testing)
    pub async fn reconnect(&self) -> Result<()> {
        info!("Manual reconnection triggered");
        
        // Increment reconnection attempts counter
        self.total_reconnection_attempts.fetch_add(1, Ordering::Relaxed);
        
        // Attempt to connect
        match self.connect_and_run().await {
            Ok(_) => {
                info!("Manual reconnection successful");
                
                // Record success
                let mut state = self.reconnection_state.lock().await;
                state.record_success();
                
                Ok(())
            }
            Err(e) => {
                error!("Manual reconnection failed: {:#}", e);
                
                // Record failure
                let mut state = self.reconnection_state.lock().await;
                state.record_failure(self.reconnect_config.max_consecutive_failures);
                
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_config() -> HyperLiquidConfig {
        HyperLiquidConfig {
            enabled: true,
            ws_url: "wss://api.hyperliquid.xyz/ws".to_string(),
            rpc_url: None,
            polling_interval_ms: Some(1000),
            private_key: None,
            trading_pairs: vec!["BTC".to_string(), "ETH".to_string()],
            subscribe_orderbook: false,
            reconnect_min_backoff_secs: 1,
            reconnect_max_backoff_secs: 60,
            max_consecutive_failures: 10,
            token_mapping: HashMap::new(),
        }
    }
    
    fn create_test_metrics() -> Arc<HyperLiquidMetrics> {
        Arc::new(HyperLiquidMetrics::new_or_default())
    }

    #[tokio::test]
    async fn test_service_creation() {
        let config = create_test_config();
        let (tx, _rx) = mpsc::unbounded_channel();
        let metrics = create_test_metrics();
        
        let service = HyperLiquidWsService::new(config, tx, metrics);
        assert!(service.is_ok());
        
        let service = service.unwrap();
        assert_eq!(service.subscriptions.len(), 2);
        assert_eq!(service.subscriptions[0].coin, "BTC");
        assert_eq!(service.subscriptions[1].coin, "ETH");
        assert!(!service.is_degraded().await);
        assert_eq!(service.consecutive_failures().await, 0);
        assert_eq!(service.total_reconnection_attempts(), 0);
    }

    #[test]
    fn test_service_creation_with_orderbook() {
        let mut config = create_test_config();
        config.subscribe_orderbook = true;
        let (tx, _rx) = mpsc::unbounded_channel();
        let metrics = create_test_metrics();
        
        let service = HyperLiquidWsService::new(config, tx, metrics).unwrap();
        
        assert!(service.subscriptions[0].subscribe_trades);
        assert!(service.subscriptions[0].subscribe_orderbook);
    }

    #[test]
    fn test_service_creation_invalid_config() {
        let mut config = create_test_config();
        config.ws_url = "invalid_url".to_string();
        let (tx, _rx) = mpsc::unbounded_channel();
        let metrics = create_test_metrics();
        
        let service = HyperLiquidWsService::new(config, tx, metrics);
        assert!(service.is_err());
    }

    #[test]
    fn test_shutdown_signal() {
        let config = create_test_config();
        let (tx, _rx) = mpsc::unbounded_channel();
        let metrics = create_test_metrics();
        
        let service = HyperLiquidWsService::new(config, tx, metrics).unwrap();
        assert!(!service.shutdown.load(Ordering::Relaxed));
        
        service.shutdown();
        assert!(service.shutdown.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn test_reconnection_state_success() {
        let mut state = ReconnectionState::new();
        
        // Initially not degraded
        assert_eq!(state.consecutive_failures, 0);
        assert!(!state.degraded);
        
        // Record a failure
        state.record_failure(10);
        assert_eq!(state.consecutive_failures, 1);
        assert!(!state.degraded);
        
        // Record success - should reset after 5 minutes
        sleep(Duration::from_millis(100)).await;
        state.record_success();
        
        // Failures not reset yet (not 5 minutes)
        assert_eq!(state.consecutive_failures, 1);
    }

    #[tokio::test]
    async fn test_reconnection_state_degraded() {
        let mut state = ReconnectionState::new();
        let max_failures = 3;
        
        // Record failures up to max
        for i in 1..=max_failures {
            state.record_failure(max_failures);
            assert_eq!(state.consecutive_failures, i);
            
            if i < max_failures {
                assert!(!state.degraded);
            } else {
                assert!(state.degraded);
            }
        }
        
        // Should be in degraded state
        assert!(state.degraded);
        assert_eq!(state.consecutive_failures, max_failures);
        
        // Record success should exit degraded state
        state.record_success();
        assert!(!state.degraded);
    }

    #[tokio::test]
    async fn test_reconnection_backoff_calculation() {
        let config = ReconnectConfig::new(1, 60, 10);
        let mut state = ReconnectionState::new();
        
        // Normal backoff - starts at min backoff with 0 failures
        assert_eq!(state.get_backoff(&config), Duration::from_secs(1));
        
        // After 1 failure, backoff is calculated based on consecutive_failures
        state.record_failure(10);
        assert_eq!(state.consecutive_failures, 1);
        assert_eq!(state.get_backoff(&config), Duration::from_secs(2)); // 1 * 2^1 = 2
        
        state.record_failure(10);
        assert_eq!(state.consecutive_failures, 2);
        assert_eq!(state.get_backoff(&config), Duration::from_secs(4)); // 1 * 2^2 = 4
        
        state.record_failure(10);
        assert_eq!(state.consecutive_failures, 3);
        assert_eq!(state.get_backoff(&config), Duration::from_secs(8)); // 1 * 2^3 = 8
        
        // Enter degraded state (need 10 total failures)
        for _ in 0..7 {
            state.record_failure(10);
        }
        assert_eq!(state.consecutive_failures, 10);
        assert!(state.degraded);
        
        // In degraded state, backoff should be 5 minutes
        assert_eq!(state.get_backoff(&config), Duration::from_secs(300));
    }

    #[tokio::test]
    async fn test_reconnection_attempts_counter() {
        let config = create_test_config();
        let (tx, _rx) = mpsc::unbounded_channel();
        let metrics = create_test_metrics();
        
        let service = HyperLiquidWsService::new(config, tx, metrics).unwrap();
        
        // Initially zero
        assert_eq!(service.total_reconnection_attempts(), 0);
        
        // Simulate reconnection attempts by incrementing counter
        service.total_reconnection_attempts.fetch_add(1, Ordering::Relaxed);
        assert_eq!(service.total_reconnection_attempts(), 1);
        
        service.total_reconnection_attempts.fetch_add(1, Ordering::Relaxed);
        assert_eq!(service.total_reconnection_attempts(), 2);
    }

    #[tokio::test]
    async fn test_degraded_warning_throttling() {
        let mut state = ReconnectionState::new();
        
        // Not in degraded state initially
        assert!(!state.should_log_degraded_warning());
        
        // Enter degraded state
        for _ in 0..10 {
            state.record_failure(10);
        }
        assert!(state.degraded);
        
        // First call after entering degraded state should return true
        // (the warning timestamp is set during record_failure when entering degraded state)
        // So subsequent calls should return false
        assert!(!state.should_log_degraded_warning());
        assert!(!state.should_log_degraded_warning());
    }

    #[tokio::test]
    async fn test_subscription_state_tracking() {
        let pair = TradingPair::with_orderbook("BTC".to_string());
        let mut state = SubscriptionState::new(pair);
        
        // Initially not subscribed
        assert!(!state.is_fully_subscribed());
        assert_eq!(state.trades_attempts, 0);
        assert_eq!(state.orderbook_attempts, 0);
        
        // Record attempts
        state.record_attempt("trades");
        assert_eq!(state.trades_attempts, 1);
        assert!(!state.is_fully_subscribed());
        
        state.record_attempt("l2Book");
        assert_eq!(state.orderbook_attempts, 1);
        assert!(!state.is_fully_subscribed());
        
        // Confirm subscriptions
        state.record_confirmation("trades");
        assert!(state.trades_confirmed);
        assert!(!state.is_fully_subscribed()); // Still need orderbook
        
        state.record_confirmation("l2Book");
        assert!(state.orderbook_confirmed);
        assert!(state.is_fully_subscribed()); // Now fully subscribed
    }

    #[tokio::test]
    async fn test_subscription_retry_logic() {
        let pair = TradingPair::trades_only("ETH".to_string());
        let mut state = SubscriptionState::new(pair);
        
        // Should retry initially
        assert!(state.should_retry("trades"));
        
        // Record first attempt
        state.record_attempt("trades");
        assert_eq!(state.trades_attempts, 1);
        
        // Should not retry immediately (need 5 seconds)
        assert!(!state.should_retry("trades"));
        
        // Wait and should be able to retry
        sleep(Duration::from_millis(100)).await;
        state.last_attempt = Some(Instant::now() - Duration::from_secs(6));
        assert!(state.should_retry("trades"));
        
        // After 3 attempts, should not retry
        state.trades_attempts = 3;
        assert!(!state.should_retry("trades"));
    }

    #[tokio::test]
    async fn test_subscription_state_reset() {
        let pair = TradingPair::with_orderbook("BTC".to_string());
        let mut state = SubscriptionState::new(pair);
        
        // Set some state
        state.record_attempt("trades");
        state.record_attempt("l2Book");
        state.record_confirmation("trades");
        
        assert_eq!(state.trades_attempts, 1);
        assert_eq!(state.orderbook_attempts, 1);
        assert!(state.trades_confirmed);
        
        // Reset
        state.reset();
        
        assert_eq!(state.trades_attempts, 0);
        assert_eq!(state.orderbook_attempts, 0);
        assert!(!state.trades_confirmed);
        assert!(!state.orderbook_confirmed);
        assert!(state.last_attempt.is_none());
    }

    #[tokio::test]
    async fn test_active_subscriptions_count() {
        let config = create_test_config();
        let (tx, _rx) = mpsc::unbounded_channel();        let metrics = create_test_metrics();
        let service = HyperLiquidWsService::new(config, tx, metrics).unwrap();
        
        // Initially no confirmed subscriptions
        assert_eq!(service.active_subscriptions().await, 0);
        
        // Confirm one subscription
        {
            let mut states = service.subscription_states.lock().await;
            if let Some(state) = states.get_mut("BTC") {
                state.record_confirmation("trades");
            }
        }
        
        assert_eq!(service.active_subscriptions().await, 1);
        
        // Confirm another
        {
            let mut states = service.subscription_states.lock().await;
            if let Some(state) = states.get_mut("ETH") {
                state.record_confirmation("trades");
            }
        }
        
        assert_eq!(service.active_subscriptions().await, 2);
    }

    #[test]
    fn test_parse_subscription_response() {
        let config = create_test_config();
        let (tx, _rx) = mpsc::unbounded_channel();        let metrics = create_test_metrics();
        let service = HyperLiquidWsService::new(config, tx, metrics).unwrap();
        
        // Test successful subscription response
        let json = r#"{"channel": "subscriptions", "data": {"type": "trades", "coin": "BTC", "success": true}}"#;
        let response = service.try_parse_subscription_response(json);
        assert!(response.is_ok());
        
        let response = response.unwrap();
        assert_eq!(response.subscription_type, "trades");
        assert_eq!(response.coin, "BTC");
        assert!(response.success);
        assert!(response.error.is_none());
        
        // Test failed subscription response
        let json = r#"{"channel": "subscriptions", "data": {"type": "l2Book", "coin": "ETH", "success": false, "error": "Invalid coin"}}"#;
        let response = service.try_parse_subscription_response(json);
        assert!(response.is_ok());
        
        let response = response.unwrap();
        assert_eq!(response.subscription_type, "l2Book");
        assert_eq!(response.coin, "ETH");
        assert!(!response.success);
        assert_eq!(response.error, Some("Invalid coin".to_string()));
        
        // Test non-subscription message
        let json = r#"{"channel": "trades", "data": {"coin": "BTC", "price": 45000}}"#;
        let response = service.try_parse_subscription_response(json);
        assert!(response.is_err());
    }

    #[tokio::test]
    async fn test_message_queue_backpressure() {
        let mut queue = MessageQueue::new(3);
        
        // Initially empty
        assert_eq!(queue.len(), 0);
        assert_eq!(queue.dropped_count(), 0);
        
        // Add messages up to capacity
        for i in 0..3 {
            let msg = HyperLiquidMessage::new(
                "trades".to_string(),
                MessageData::Trade(crate::types::TradeData {
                    coin: "BTC".to_string(),
                    side: crate::types::TradeSide::Buy,
                    price: 45000.0 + i as f64,
                    size: 1.0,
                    timestamp: 1696348800000,
                    trade_id: format!("trade_{}", i),
                }),
            );
            queue.push(msg);
        }
        
        assert_eq!(queue.len(), 3);
        assert_eq!(queue.dropped_count(), 0);
        
        // Add one more - should drop oldest
        let msg = HyperLiquidMessage::new(
            "trades".to_string(),
            MessageData::Trade(crate::types::TradeData {
                coin: "ETH".to_string(),
                side: crate::types::TradeSide::Sell,
                price: 3000.0,
                size: 2.0,
                timestamp: 1696348800000,
                trade_id: "trade_3".to_string(),
            }),
        );
        queue.push(msg);
        
        // Queue should still be at capacity, but one message dropped
        assert_eq!(queue.len(), 3);
        assert_eq!(queue.dropped_count(), 1);
        
        // Pop messages - first one should be the second message we added (first was dropped)
        let first = queue.pop().unwrap();
        if let MessageData::Trade(trade) = first.data {
            assert_eq!(trade.price, 45001.0); // Second message
        } else {
            panic!("Expected trade message");
        }
    }

    #[tokio::test]
    async fn test_message_processing_metrics() {
        let config = create_test_config();
        let (tx, mut rx) = mpsc::unbounded_channel();        let metrics = create_test_metrics();
        let service = HyperLiquidWsService::new(config, tx, metrics).unwrap();
        
        // Initial metrics should be 0
        assert_eq!(service.total_messages_received(), 0);
        assert_eq!(service.total_messages_processed(), 0);
        assert_eq!(service.total_parse_errors(), 0);
        assert_eq!(service.message_queue_size().await, 0);
        assert_eq!(service.dropped_messages().await, 0);
        
        // Simulate receiving a valid trade message
        let trade_msg = Message::Text(r#"{
            "channel": "trades",
            "data": {
                "coin": "BTC",
                "side": "B",
                "px": "45000.5",
                "sz": "0.5",
                "time": 1696348800000,
                "tid": "12345"
            }
        }"#.to_string());
        
        service.handle_message(trade_msg).await.unwrap();
        
        // Should have received and processed one message
        assert_eq!(service.total_messages_received(), 1);
        assert_eq!(service.total_messages_processed(), 1);
        assert_eq!(service.total_parse_errors(), 0);
        
        // Should have sent message to channel
        let received = rx.try_recv();
        assert!(received.is_ok());
        let msg = received.unwrap();
        assert!(msg.is_trade());
        
        // Simulate receiving a malformed message
        let bad_msg = Message::Text(r#"{ invalid json }"#.to_string());
        service.handle_message(bad_msg).await.unwrap();
        
        // Should have received 2 messages, but only processed 1, with 1 parse error
        assert_eq!(service.total_messages_received(), 2);
        assert_eq!(service.total_messages_processed(), 1);
        assert_eq!(service.total_parse_errors(), 1);
    }

    #[tokio::test]
    async fn test_message_ordering_preservation() {
        let config = create_test_config();
        let (tx, mut rx) = mpsc::unbounded_channel();        let metrics = create_test_metrics();
        let service = HyperLiquidWsService::new(config, tx, metrics).unwrap();
        
        // Send multiple trade messages in order
        for i in 0..5 {
            let trade_msg = Message::Text(format!(r#"{{
                "channel": "trades",
                "data": {{
                    "coin": "BTC",
                    "side": "B",
                    "px": "{}",
                    "sz": "0.5",
                    "time": {},
                    "tid": "trade_{}"
                }}
            }}"#, 45000.0 + i as f64, 1696348800000u64 + i as u64, i));
            
            service.handle_message(trade_msg).await.unwrap();
        }
        
        // Verify messages are received in order
        for i in 0..5 {
            let msg = rx.try_recv().unwrap();
            assert!(msg.is_trade());
            
            if let MessageData::Trade(trade) = msg.data {
                assert_eq!(trade.price, 45000.0 + i as f64);
                assert_eq!(trade.trade_id, format!("trade_{}", i));
            } else {
                panic!("Expected trade message");
            }
        }
    }

    #[tokio::test]
    async fn test_handle_subscription_response_message() {
        let config = create_test_config();
        let (tx, mut rx) = mpsc::unbounded_channel();        let metrics = create_test_metrics();
        let service = HyperLiquidWsService::new(config, tx, metrics).unwrap();
        
        // Send subscription response message
        let sub_msg = Message::Text(r#"{
            "channel": "subscriptionResponse",
            "data": {
                "type": "trades",
                "coin": "BTC",
                "success": true
            }
        }"#.to_string());
        
        service.handle_message(sub_msg).await.unwrap();
        
        // Subscription responses should not be sent to channel
        assert!(rx.try_recv().is_err());
        
        // But should be tracked as received
        assert_eq!(service.total_messages_received(), 1);
    }

    #[tokio::test]
    async fn test_handle_error_message() {
        let config = create_test_config();
        let (tx, mut rx) = mpsc::unbounded_channel();        let metrics = create_test_metrics();
        let service = HyperLiquidWsService::new(config, tx, metrics).unwrap();
        
        // Send error message
        let error_msg = Message::Text(r#"{
            "channel": "error",
            "data": {
                "code": 400,
                "message": "Invalid request"
            }
        }"#.to_string());
        
        service.handle_message(error_msg).await.unwrap();
        
        // Error messages should not be sent to channel
        assert!(rx.try_recv().is_err());
        
        // But should be tracked as received
        assert_eq!(service.total_messages_received(), 1);
    }

    #[tokio::test]
    async fn test_handle_orderbook_message() {
        let config = create_test_config();
        let (tx, mut rx) = mpsc::unbounded_channel();        let metrics = create_test_metrics();
        let service = HyperLiquidWsService::new(config, tx, metrics).unwrap();
        
        // Send order book message
        let orderbook_msg = Message::Text(r#"{
            "channel": "l2Book",
            "data": {
                "coin": "BTC",
                "bids": [
                    {"px": "45000.0", "sz": "1.0"},
                    {"px": "44999.0", "sz": "2.0"}
                ],
                "asks": [
                    {"px": "45001.0", "sz": "1.5"}
                ],
                "time": 1696348800000
            }
        }"#.to_string());
        
        service.handle_message(orderbook_msg).await.unwrap();
        
        // Should have sent message to channel
        let received = rx.try_recv();
        assert!(received.is_ok());
        let msg = received.unwrap();
        assert!(msg.is_orderbook());
        
        if let MessageData::OrderBook(orderbook) = msg.data {
            assert_eq!(orderbook.coin, "BTC");
            assert_eq!(orderbook.bids.len(), 2);
            assert_eq!(orderbook.asks.len(), 1);
        } else {
            panic!("Expected orderbook message");
        }
    }

    #[test]
    fn test_subscription_state_max_retries() {
        let pair = TradingPair::trades_only("BTC".to_string());
        let mut state = SubscriptionState::new(pair);
        
        // Record 3 attempts
        for _ in 0..3 {
            state.record_attempt("trades");
        }
        
        assert_eq!(state.trades_attempts, 3);
        
        // Should not allow more retries (max is 3)
        assert!(!state.should_retry("trades"));
    }
}



