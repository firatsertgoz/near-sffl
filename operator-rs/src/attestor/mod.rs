mod notifier;
mod event_listener;
use crate::consumer::Consumer;
use crate::consumer::config::ConsumerConfig;

use std::collections::HashMap;
use std::sync::Arc;
use alloy_rpc_types::Header;
use tokio::sync::mpsc;
use tokio::time::Duration;
use anyhow::{Result, anyhow};
use tracing::{error, info, warn};
use prometheus::Registry;
use alloy_rpc_types::Block;
use tokio::sync::Mutex;
use core_rs::safeclient::{SafeClient, SafeEthClient, SafeEthClientOptions};

use crate::types::{BlockData, NFFLNodeConfig, SignedStateRootUpdateMessage, StateRootUpdateMessage};
use self::notifier::Notifier;
use self::event_listener::{EventListener, SelectiveEventListener};
use eigensdk::crypto_bls::{BlsKeyPair, BlsSignature};
use tokio::sync::broadcast;

// Constants
const MQ_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

// TODO: Replace with actual types from eigensdk-rs
type OperatorId = eigensdk::types::operator::OperatorId;

struct SharedState {
    notifier: Arc<Notifier>,
    consumer: Mutex<Consumer>,
    listener: Box<dyn EventListener + Send + Sync>,
    signed_root_tx: broadcast::Sender<SignedStateRootUpdateMessage>,
}

pub struct Attestor {
    shared: Arc<SharedState>,
    rollup_ids_to_urls: HashMap<u32, String>,
    clients: HashMap<u32, Arc<dyn SafeClient>>,
    rpc_calls_collectors: HashMap<u32, ()>, // Replace with actual RPC calls collector
    config: NFFLNodeConfig,
    bls_keypair: BlsKeyPair,
    operator_id: OperatorId,
    registry: Registry,
}

impl Attestor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(config: NFFLNodeConfig, bls_keypair: BlsKeyPair,
         operator_id: OperatorId, registry: Registry) -> Result<Self> {
        info!("Creating new Attestor instance");
        let consumer = Consumer::new(ConsumerConfig {
            rollup_ids: config.near_da_indexer_rollup_ids.clone(),
            id: hex::encode(&operator_id),
        });

        let mut clients = HashMap::new();
        let mut rpc_calls_collectors = HashMap::new();

        for (rollup_id, url) in &config.rollup_ids_to_rpc_urls {
            info!("Creating SafeClient for rollup_id: {}, url: {}", rollup_id, url);
            let client: Arc<dyn SafeClient> = Arc::new(create_safe_client(url)?);
            clients.insert(*rollup_id, client);

            if config.enable_metrics {
                info!("Metrics enabled, creating RPC calls collector for rollup_id: {}", rollup_id);
                // Create and add RPC calls collector (mock for now)
                rpc_calls_collectors.insert(*rollup_id, ());
            }
        }

        let (signed_root_tx, _) = broadcast::channel(100);

        let shared = Arc::new(SharedState {
            notifier: Arc::new(Notifier::new()),
            consumer: Mutex::new(consumer),
            listener: Box::new(SelectiveEventListener::default()),
            signed_root_tx,
        });

        info!("Attestor instance created successfully");
        Ok(Self {
            shared,
            rollup_ids_to_urls: config.rollup_ids_to_rpc_urls.clone(),
            clients,
            rpc_calls_collectors,
            config,
            bls_keypair,
            operator_id,
            registry,
        })
    }

    pub fn enable_metrics(&mut self, registry: &Registry) -> Result<()> {
        info!("Enabling metrics for Attestor");
        let _listener = event_listener::make_attestor_metrics(registry)?;
        // TODO: Implement metrics enabling logic
        info!("Metrics enabled successfully");
        Ok(())
    }

    pub async fn start(&self) -> Result<()> {
        info!("Starting Attestor");
        let addr = self.config.near_da_indexer_rmq_ip_port_address.clone();
        
        let shared = Arc::clone(&self.shared);
        tokio::spawn(async move {
            loop {
                let mut consumer = shared.consumer.lock().await;
                if let Err(e) = consumer.start(&addr).await {
                    error!("Consumer error: {:?}", e);
                    info!("Retrying consumer start in 5 seconds");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        });

        let mut headers_rxs = HashMap::new();

        for (rollup_id, client) in &self.clients {
            info!("Setting up header subscription for rollup_id: {}", rollup_id);
            let headers_rx = client.subscribe_new_heads().await?;
            let block_number = client.block_number().await?;

            info!("Observing initial block number {} for rollup_id: {}", block_number, rollup_id);
            self.shared.listener.observe_initialization_initial_block_number(*rollup_id, block_number.to::<u64>());

            headers_rxs.insert(*rollup_id, headers_rx);
        }

        let shared = Arc::clone(&self.shared);
        tokio::spawn(async move {
            if let Err(e) = Self::process_mq_blocks(shared).await {
                error!("Error processing MQ blocks: {:?}", e);
            }
        });

        for (rollup_id, headers_rx) in headers_rxs {
            let cloned_operator_id = self.operator_id.clone();
            let cloned_keypair = self.bls_keypair.clone();
            let self_ref = Arc::clone(&self.shared);
            info!("Spawning task to process rollup headers for rollup_id: {}", rollup_id);
            tokio::spawn(async move {
                if let Err(e) = Self::process_rollup_headers(&self_ref, rollup_id, cloned_operator_id, &cloned_keypair, headers_rx).await {
                    error!("Error processing rollup headers for rollup {}: {:?}", rollup_id, e);
                }
            });
        }

        info!("Attestor started successfully");
        Ok(())
    }

    async fn process_mq_blocks(shared: Arc<SharedState>) -> Result<()> {
        info!("Starting MQ blocks processing");
        loop {
            let consumer = shared.consumer.lock().await;
            let mut mq_block_rx = consumer.get_block_stream();
            drop(consumer); // Release the lock
            
            loop {
                match mq_block_rx.recv().await {
                    Ok(mq_block) => {
                        info!("Notifying - rollupId: {}, height: {}", mq_block.rollup_id, get_block_number(&mq_block.block));
                        if let Err(e) = shared.notifier.notify(mq_block.rollup_id, mq_block) {
                            error!("Notifier error: {}", e);
                        }
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        warn!("MQ block channel closed");
                        break; // Break the inner loop to reconnect
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("Skipped {} messages due to slow processing", skipped);
                        // Continue processing
                    }
                }
            }
            
            // If we've broken out of the inner loop, wait a bit before trying to reconnect
            info!("Waiting 5 seconds before attempting to reconnect to MQ");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    async fn process_rollup_headers(shared: &Arc<SharedState>, rollup_id: u32, operator_id: OperatorId, keypair: &BlsKeyPair, mut headers_rx: broadcast::Receiver<Header>) -> Result<()> {
        info!("Starting to process rollup headers for rollup_id: {}", rollup_id);
        while let Ok(header) = headers_rx.recv().await {
            if let Err(e) = Self::process_header(shared, rollup_id, operator_id, keypair, header).await {
                error!("Error processing header for rollup {}: {:?}", rollup_id, e);
            }
        }
        warn!("Finished processing rollup headers for rollup_id: {}", rollup_id);
        Ok(())
    }

    async fn process_header(shared: &Arc<SharedState>, rollup_id: u32, operator_id: OperatorId, keypair: &BlsKeyPair, rollup_header: Header) -> Result<()> {
        let header_number = get_header_number(&rollup_header);
        let header_timestamp = get_header_timestamp(&rollup_header);
        let header_root = get_header_root(&rollup_header);
    
        info!("Processing header - rollup_id: {}, header_number: {}", rollup_id, header_number);
    
        shared.listener.observe_last_block_received(rollup_id, header_number);
        shared.listener.observe_last_block_received_timestamp(rollup_id, header_timestamp);
        shared.listener.on_block_received(rollup_id);
    
        let predicate = move |mq_block: &BlockData| {
            mq_block.rollup_id == rollup_id
                && header_number == get_block_number(&mq_block.block)
                && get_block_root(&mq_block.block) == header_root
        };
    
        let notifier = Arc::clone(&shared.notifier);
        let (mut mq_blocks_rx, id) = notifier.subscribe(rollup_id, predicate);
    
        let mut transaction_id = [0u8; 32];
        let mut da_commitment = [0u8; 32];
    
        let result = tokio::time::timeout(MQ_WAIT_TIMEOUT, mq_blocks_rx.recv()).await;
    
        match result {
            Ok(Some(mq_block)) => {
                info!("MQ block found - height: {}, rollupId: {}", get_block_number(&mq_block.block), mq_block.rollup_id);
                transaction_id = mq_block.transaction_id;
                da_commitment = mq_block.commitment;
            }
            Ok(None) => {
                warn!("MQ channel closed unexpectedly - rollupId: {}, height: {}", rollup_id, header_number);
            }
            Err(_) => {
                warn!("MQ timeout - rollupId: {}, height: {}", rollup_id, header_number);
                shared.listener.on_missed_mq_block(rollup_id);
            }
        }
    
        notifier.unsubscribe(rollup_id, id);
    
        let message = StateRootUpdateMessage {
            rollup_id,
            block_height: header_number,
            timestamp: header_timestamp,
            state_root: header_root,
            near_da_transaction_id: transaction_id,
            near_da_commitment: da_commitment,
        };
    
        match sign_state_root_update_message(keypair, &message) {
            Ok(signature) => {
                let signed_message = SignedStateRootUpdateMessage {
                    message,
                    bls_signature: signature,
                    operator_id,
                };
                if let Err(e) = shared.signed_root_tx.send(signed_message) {
                    warn!("Failed to send signed state root update: {}", e);
                } else {
                    info!("Successfully sent signed state root update for rollup_id: {}, height: {}", rollup_id, header_number);
                }
            }
            Err(e) => {
                error!("State root sign failed: {}", e);
                return Err(anyhow!("State root sign failed: {}", e));
            }
        }
        Ok(())
    }

    pub fn get_signed_root_rx(&self) -> broadcast::Receiver<SignedStateRootUpdateMessage> {
        info!("Getting signed root receiver");
        self.shared.signed_root_tx.subscribe()
    }

    pub async fn close(&self) -> Result<()> {
        info!("Closing Attestor");
        let mut consumer = self.shared.consumer.lock().await;
        consumer.close().await?;
        for (rollup_id, client) in &self.clients {
            info!("Closing client for rollup_id: {}", rollup_id);
            client.close();
        }
        info!("Attestor closed successfully");
        Ok(())
    }
}

// Helper functions implementations
fn create_safe_client(url: &str) -> Result<SafeEthClient> {
    let options = SafeEthClientOptions {
        log_resub_interval: Duration::from_secs(300),
        header_timeout: Duration::from_secs(30),
        block_chunk_size: 100,
        block_max_range: 100,
    };

    let client = tokio::runtime::Runtime::new()?.block_on(async {
        SafeEthClient::new(url, options).await
    })?;

    info!("Created SafeEthClient");
    Ok(client)
}

fn sign_state_root_update_message(_keypair: &BlsKeyPair, _message: &StateRootUpdateMessage) -> Result<BlsSignature> {
    // In a real implementation, you'd use the actual BLS signing logic
    // For now, we'll create a mock signature
    info!("Creating mock BLS signature");
    let mock_signature = BlsSignature::default(); // Assuming BlsSignature has a default implementation
    Ok(mock_signature)
}

fn get_header_number(header: &Header) -> u64 {
    header.number
}

fn get_header_timestamp(header: &Header) -> u64 {
    header.timestamp
}

fn get_header_root(header: &Header) -> [u8; 32] {
    header.state_root.into()
}

fn get_block_number(block: &Block) -> u64 {
    block.header.number
}

fn get_block_root(block: &Block) -> [u8; 32] {
    block.header.state_root.into()
}
