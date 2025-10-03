# Requirements Document

## Introduction

This document outlines the requirements for fixing the WebSocket subscription issue with Hyperliquid's HyperEVM network. The current implementation attempts to use standard Ethereum `eth_subscribe` on Hyperliquid's general WebSocket endpoint, which doesn't support this method. The fix needs to either use the correct EVM WebSocket endpoint or implement an alternative mempool monitoring approach compatible with Hyperliquid's infrastructure.

## Requirements

### Requirement 1: Identify Correct Hyperliquid EVM WebSocket Endpoint

**User Story:** As a MEV bot operator, I want to connect to the correct Hyperliquid EVM WebSocket endpoint, so that I can successfully subscribe to pending transactions.

#### Acceptance Criteria

1. WHEN researching Hyperliquid's API THEN the system SHALL identify if HyperEVM supports standard Ethereum WebSocket subscriptions
2. WHEN a correct WebSocket endpoint exists THEN the configuration SHALL be updated to use the proper EVM-compatible endpoint
3. WHEN testing the connection THEN the system SHALL successfully receive a subscription confirmation response
4. WHEN no EVM WebSocket is available THEN the system SHALL document alternative approaches for mempool monitoring
5. WHEN updating configuration THEN the system SHALL maintain backward compatibility with existing config structure

### Requirement 2: Implement Alternative Mempool Monitoring Strategy

**User Story:** As a MEV bot developer, I want a fallback strategy for mempool monitoring, so that the bot can function even if WebSocket subscriptions are not available.

#### Acceptance Criteria

1. WHEN WebSocket subscriptions are unavailable THEN the system SHALL implement HTTP polling as a fallback mechanism
2. WHEN using HTTP polling THEN the system SHALL call `eth_getBlockByNumber` with pending parameter to fetch mempool transactions
3. WHEN polling THEN the system SHALL implement configurable polling intervals (default 100-500ms) to balance latency and rate limits
4. WHEN detecting new transactions THEN the system SHALL deduplicate transactions across polling cycles
5. WHEN switching strategies THEN the system SHALL allow configuration-based selection between WebSocket and polling modes

### Requirement 3: Update WebSocket Client Implementation

**User Story:** As a system maintainer, I want the WebSocket client to handle Hyperliquid-specific requirements, so that connections are stable and reliable.

#### Acceptance Criteria

1. WHEN connecting to WebSocket THEN the system SHALL use the correct endpoint format for HyperEVM
2. WHEN sending subscription requests THEN the system SHALL use the correct JSON-RPC format expected by the endpoint
3. WHEN handling responses THEN the system SHALL properly parse Hyperliquid's response format
4. WHEN connection fails THEN the system SHALL provide clear error messages indicating the issue
5. WHEN implementing fixes THEN the system SHALL add integration tests to verify subscription success

### Requirement 4: Enhance Error Handling and Diagnostics

**User Story:** As a bot operator, I want clear diagnostic information when connections fail, so that I can quickly identify and resolve issues.

#### Acceptance Criteria

1. WHEN subscription fails THEN the system SHALL log the exact request and response (or timeout) for debugging
2. WHEN testing endpoints THEN the system SHALL implement a diagnostic mode that validates WebSocket connectivity
3. WHEN errors occur THEN the system SHALL distinguish between connection errors, subscription errors, and timeout errors
4. WHEN retrying THEN the system SHALL implement exponential backoff with maximum retry limits
5. WHEN operating THEN the system SHALL expose connection health metrics via Prometheus

### Requirement 5: Configuration Flexibility

**User Story:** As a deployment engineer, I want flexible configuration options, so that I can adapt to different Hyperliquid endpoint configurations.

#### Acceptance Criteria

1. WHEN configuring endpoints THEN the system SHALL support separate RPC and WebSocket URLs
2. WHEN WebSocket is unavailable THEN the system SHALL allow disabling WebSocket and using polling-only mode
3. WHEN using QuickNode or other providers THEN the system SHALL support provider-specific endpoint formats
4. WHEN testing THEN the system SHALL support environment-specific configurations (dev/testnet/mainnet)
5. WHEN deploying THEN the system SHALL validate configuration on startup and fail fast with clear errors
