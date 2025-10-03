//! gRPC service implementation for simulation engine

use anyhow::Result;
use std::{collections::HashMap, str::FromStr, sync::Arc};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

use crate::{ForkSimulator, SimulationBundle as CoreBundle, SimulationTransaction as CoreTransaction, StateOverride as CoreStateOverride, SimulationResult as CoreResult, TransactionResult as CoreTxResult, SimulationLog as CoreLog, ProfitEstimate as CoreProfit};

// Include the generated protobuf code
pub mod simulation {
    tonic::include_proto!("simulation");
}

use simulation::{
    simulation_service_server::{SimulationService, SimulationServiceServer},
    *,
};

/// gRPC service implementation
pub struct SimulationGrpcService {
    simulator: Arc<ForkSimulator>,
}

impl SimulationGrpcService {
    pub fn new(simulator: Arc<ForkSimulator>) -> Self {
        Self { simulator }
    }

    /// Create a gRPC server
    pub fn create_server(self) -> SimulationServiceServer<Self> {
        SimulationServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl SimulationService for SimulationGrpcService {
    async fn simulate_bundle(
        &self,
        request: Request<SimulateBundleRequest>,
    ) -> Result<Response<SimulateBundleResponse>, Status> {
        let req = request.into_inner();
        
        debug!("Received gRPC simulate bundle request");

        let bundle = match req.bundle {
            Some(bundle) => convert_proto_to_core_bundle(bundle)?,
            None => return Err(Status::invalid_argument("Bundle is required")),
        };

        match self.simulator.simulate_bundle(bundle).await {
            Ok(result) => {
                let proto_result = convert_core_to_proto_result(result)?;
                Ok(Response::new(SimulateBundleResponse {
                    result: Some(proto_result),
                }))
            }
            Err(e) => {
                error!("Bundle simulation failed: {}", e);
                Err(Status::internal(format!("Simulation failed: {}", e)))
            }
        }
    }

    async fn simulate_bundles(
        &self,
        request: Request<SimulateBundlesRequest>,
    ) -> Result<Response<SimulateBundlesResponse>, Status> {
        let req = request.into_inner();
        
        debug!("Received gRPC simulate bundles request with {} bundles", req.bundles.len());

        let mut results = Vec::new();

        // Process bundles concurrently
        let futures: Vec<_> = req.bundles
            .into_iter()
            .map(|proto_bundle| {
                let simulator = self.simulator.clone();
                async move {
                    let bundle = convert_proto_to_core_bundle(proto_bundle)?;
                    simulator.simulate_bundle(bundle).await
                }
            })
            .collect();

        let simulation_results = futures::future::join_all(futures).await;

        for result in simulation_results {
            match result {
                Ok(core_result) => {
                    let proto_result = convert_core_to_proto_result(core_result)?;
                    results.push(proto_result);
                }
                Err(e) => {
                    error!("Batch simulation error: {}", e);
                    // Continue with other simulations, but log the error
                }
            }
        }

        Ok(Response::new(SimulateBundlesResponse { results }))
    }

    async fn get_health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let stats = self.simulator.get_stats();
        
        let proto_stats = SimulationStats {
            active_simulations: stats.active_simulations as u32,
            max_concurrent: stats.max_concurrent as u32,
            timeout_ms: stats.timeout_ms,
            rpc_url: stats.rpc_url.clone(),
            total_simulations: 0, // Would need to track this in the simulator
            successful_simulations: 0, // Would need to track this in the simulator
            failed_simulations: 0, // Would need to track this in the simulator
            average_simulation_time_ms: 0.0, // Would need to track this in the simulator
        };

        Ok(Response::new(HealthResponse {
            healthy: true,
            status: "healthy".to_string(),
            stats: Some(proto_stats),
        }))
    }

    async fn get_stats(
        &self,
        _request: Request<StatsRequest>,
    ) -> Result<Response<StatsResponse>, Status> {
        let stats = self.simulator.get_stats();
        
        let proto_stats = SimulationStats {
            active_simulations: stats.active_simulations as u32,
            max_concurrent: stats.max_concurrent as u32,
            timeout_ms: stats.timeout_ms,
            rpc_url: stats.rpc_url.clone(),
            total_simulations: 0, // Would need to track this in the simulator
            successful_simulations: 0, // Would need to track this in the simulator
            failed_simulations: 0, // Would need to track this in the simulator
            average_simulation_time_ms: 0.0, // Would need to track this in the simulator
        };

        Ok(Response::new(StatsResponse {
            stats: Some(proto_stats),
        }))
    }
}

/// Convert protobuf bundle to core bundle
fn convert_proto_to_core_bundle(proto_bundle: SimulationBundle) -> Result<CoreBundle, Status> {
    let mut transactions = Vec::new();
    
    for proto_tx in proto_bundle.transactions {
        let tx = convert_proto_to_core_transaction(proto_tx)?;
        transactions.push(tx);
    }

    let state_overrides = if proto_bundle.state_overrides.is_empty() {
        None
    } else {
        let mut overrides = HashMap::new();
        for (addr_str, proto_override) in proto_bundle.state_overrides {
            let address = ethers::types::Address::from_str(&addr_str)
                .map_err(|e| Status::invalid_argument(format!("Invalid address: {}", e)))?;
            let override_obj = convert_proto_to_core_state_override(proto_override)?;
            overrides.insert(address, override_obj);
        }
        Some(overrides)
    };

    Ok(CoreBundle {
        id: proto_bundle.id,
        transactions,
        block_number: proto_bundle.block_number,
        timestamp: proto_bundle.timestamp,
        base_fee: proto_bundle.base_fee.and_then(|s| ethers::types::U256::from_dec_str(&s).ok()),
        state_overrides,
    })
}

/// Convert protobuf transaction to core transaction
fn convert_proto_to_core_transaction(proto_tx: SimulationTransaction) -> Result<CoreTransaction, Status> {
    let from = ethers::types::Address::from_str(&proto_tx.from)
        .map_err(|e| Status::invalid_argument(format!("Invalid from address: {}", e)))?;
    
    let to = if let Some(to_str) = proto_tx.to {
        Some(ethers::types::Address::from_str(&to_str)
            .map_err(|e| Status::invalid_argument(format!("Invalid to address: {}", e)))?)
    } else {
        None
    };

    let value = ethers::types::U256::from_dec_str(&proto_tx.value)
        .map_err(|e| Status::invalid_argument(format!("Invalid value: {}", e)))?;
    
    let gas_limit = ethers::types::U256::from_dec_str(&proto_tx.gas_limit)
        .map_err(|e| Status::invalid_argument(format!("Invalid gas_limit: {}", e)))?;
    
    let gas_price = ethers::types::U256::from_dec_str(&proto_tx.gas_price)
        .map_err(|e| Status::invalid_argument(format!("Invalid gas_price: {}", e)))?;

    let data = ethers::types::Bytes::from_str(&proto_tx.data)
        .map_err(|e| Status::invalid_argument(format!("Invalid data: {}", e)))?;

    let nonce = if let Some(nonce_str) = proto_tx.nonce {
        Some(ethers::types::U256::from_dec_str(&nonce_str)
            .map_err(|e| Status::invalid_argument(format!("Invalid nonce: {}", e)))?)
    } else {
        None
    };

    Ok(CoreTransaction {
        from,
        to,
        value,
        gas_limit,
        gas_price,
        data,
        nonce,
    })
}

/// Convert protobuf state override to core state override
fn convert_proto_to_core_state_override(proto_override: StateOverride) -> Result<CoreStateOverride, Status> {
    let balance = if let Some(balance_str) = proto_override.balance {
        Some(ethers::types::U256::from_dec_str(&balance_str)
            .map_err(|e| Status::invalid_argument(format!("Invalid balance: {}", e)))?)
    } else {
        None
    };

    let nonce = if let Some(nonce_str) = proto_override.nonce {
        Some(ethers::types::U256::from_dec_str(&nonce_str)
            .map_err(|e| Status::invalid_argument(format!("Invalid nonce: {}", e)))?)
    } else {
        None
    };

    let code = if let Some(code_str) = proto_override.code {
        Some(ethers::types::Bytes::from_str(&code_str)
            .map_err(|e| Status::invalid_argument(format!("Invalid code: {}", e)))?)
    } else {
        None
    };

    let state = if proto_override.state.is_empty() {
        None
    } else {
        let mut state_map = HashMap::new();
        for (key_str, value_str) in proto_override.state {
            let key = ethers::types::H256::from_str(&key_str)
                .map_err(|e| Status::invalid_argument(format!("Invalid state key: {}", e)))?;
            let value = ethers::types::H256::from_str(&value_str)
                .map_err(|e| Status::invalid_argument(format!("Invalid state value: {}", e)))?;
            state_map.insert(key, value);
        }
        Some(state_map)
    };

    let state_diff = if proto_override.state_diff.is_empty() {
        None
    } else {
        let mut state_diff_map = HashMap::new();
        for (key_str, value_str) in proto_override.state_diff {
            let key = ethers::types::H256::from_str(&key_str)
                .map_err(|e| Status::invalid_argument(format!("Invalid state_diff key: {}", e)))?;
            let value = ethers::types::H256::from_str(&value_str)
                .map_err(|e| Status::invalid_argument(format!("Invalid state_diff value: {}", e)))?;
            state_diff_map.insert(key, value);
        }
        Some(state_diff_map)
    };

    Ok(CoreStateOverride {
        balance,
        nonce,
        code,
        state,
        state_diff,
    })
}

/// Convert core result to protobuf result
fn convert_core_to_proto_result(core_result: CoreResult) -> Result<SimulationResult, Status> {
    let mut proto_tx_results = Vec::new();
    
    for core_tx_result in core_result.transaction_results {
        let proto_tx_result = convert_core_to_proto_tx_result(core_tx_result)?;
        proto_tx_results.push(proto_tx_result);
    }

    let proto_profit = convert_core_to_proto_profit(core_result.profit_estimate)?;

    Ok(SimulationResult {
        bundle_id: core_result.bundle_id,
        success: core_result.success,
        gas_used: core_result.gas_used.to_string(),
        gas_cost: core_result.gas_cost.to_string(),
        profit_estimate: Some(proto_profit),
        transaction_results: proto_tx_results,
        simulation_time_ms: core_result.simulation_time_ms,
        error_message: core_result.error_message,
        block_number: core_result.block_number,
        effective_gas_price: core_result.effective_gas_price.to_string(),
        revert_reason: core_result.revert_reason,
    })
}

/// Convert core transaction result to protobuf transaction result
fn convert_core_to_proto_tx_result(core_tx_result: CoreTxResult) -> Result<TransactionResult, Status> {
    let mut proto_logs = Vec::new();
    
    for core_log in core_tx_result.logs {
        let proto_log = convert_core_to_proto_log(core_log)?;
        proto_logs.push(proto_log);
    }

    Ok(TransactionResult {
        success: core_tx_result.success,
        gas_used: core_tx_result.gas_used.to_string(),
        gas_estimate: core_tx_result.gas_estimate.to_string(),
        return_data: core_tx_result.return_data.map(|data| format!("0x{}", hex::encode(data))),
        logs: proto_logs,
        error: core_tx_result.error,
        revert_reason: core_tx_result.revert_reason,
        effective_gas_price: core_tx_result.effective_gas_price.to_string(),
    })
}

/// Convert core log to protobuf log
fn convert_core_to_proto_log(core_log: CoreLog) -> Result<SimulationLog, Status> {
    let topics: Vec<String> = core_log.topics
        .into_iter()
        .map(|topic| format!("0x{}", hex::encode(topic)))
        .collect();

    Ok(SimulationLog {
        address: format!("0x{}", hex::encode(core_log.address)),
        topics,
        data: format!("0x{}", hex::encode(core_log.data)),
    })
}

/// Convert core profit estimate to protobuf profit estimate
fn convert_core_to_proto_profit(core_profit: CoreProfit) -> Result<ProfitEstimate, Status> {
    Ok(ProfitEstimate {
        gross_profit_wei: core_profit.gross_profit_wei.to_string(),
        gas_cost_wei: core_profit.gas_cost_wei.to_string(),
        net_profit_wei: core_profit.net_profit_wei.to_string(),
        profit_margin: core_profit.profit_margin,
        confidence: core_profit.confidence,
    })
}

/// Start gRPC server
pub async fn start_grpc_server(
    simulator: Arc<ForkSimulator>,
    port: u16,
) -> Result<()> {
    let addr = format!("0.0.0.0:{}", port).parse()?;
    let service = SimulationGrpcService::new(simulator);
    let server = service.create_server();

    info!("Starting gRPC simulation server on {}", addr);

    tonic::transport::Server::builder()
        .add_service(server)
        .serve(addr)
        .await?;

    Ok(())
}