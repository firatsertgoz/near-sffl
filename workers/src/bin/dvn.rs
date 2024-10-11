//! Main offchain workflow for Nuff DVN.

use eyre::Result;
use futures::stream::StreamExt;
use tracing::{debug, error, info};
use tracing_subscriber::EnvFilter;
use workers::{
    abi::{
        L0V2EndpointAbi::{self},
        SendLibraryAbi,
    },
    chain::{
        connections::{build_subscriptions, get_abi_from_path, get_http_provider},
        contracts::{create_contract_instance, query_already_verified, query_confirmations, verify},
    },
    data::Dvn,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let mut dvn_worker = Dvn::new_from_env()?;

    // Create the WS subscriptions for listening to the events.
    let (_provider, mut endpoint_stream, mut sendlib_stream) = build_subscriptions(dvn_worker.config()).await?;

    // Create an HTTP provider to call contract functions.
    let http_provider = get_http_provider(dvn_worker.config())?;

    // Get the relevant contract ABI, and create contract.
    //let sendlib_abi = get_abi_from_path("./abi/ArbitrumSendLibUln302.json")?;
    //let sendlib_contract = create_contract_instance(&config, http_provider.clone(), sendlib_abi)?;
    let receivelib_abi = get_abi_from_path("./abi/ArbitrumReceiveLibUln302.json")?;
    let receivelib_contract = create_contract_instance(dvn_worker.config(), http_provider, receivelib_abi)?;

    info!("Listening to chain events...");

    loop {
        dvn_worker.listening();
        tokio::select! {
            Some(log) = endpoint_stream.next() => {
                match log.log_decode::<L0V2EndpointAbi::PacketSent>() {
                    Ok(inner_log) => {
                        debug!("PacketSent event found and decoded.");
                        dvn_worker.packet_received(inner_log.data().clone());
                    },
                    Err(e) => {
                        error!("Failed to decode `PacketSent` event: {:?}", e);
                    }
                }
            }
            Some(log) = sendlib_stream.next() => {
                match log.log_decode::<SendLibraryAbi::DVNFeePaid>() {
                    Ok(inner_log) => {
                        info!("DVNFeePaid event found and decoded.");
                        let required_dvns = inner_log.inner.requiredDVNs.clone();

                        if required_dvns.contains(&dvn_worker.config().dvn_addr()?) {
                            info!("Found DVN in required DVNs.");

                            // NOTE: the docs' workflow require now to query L0's endpoint to
                            // get the address of the MessageLib, but we have already created
                            // the contract above to query it directly.

                            let required_confirmations =
                                query_confirmations(&receivelib_contract, dvn_worker.config().eid()).await?;

                            let already_verified = query_already_verified(
                                &receivelib_contract,
                                dvn_worker.config().dvn_addr()?,
                                &[1, 2, 3],
                                &[1, 2, 3],
                                required_confirmations,
                            )
                            .await?;

                            if already_verified {
                                debug!("Packet already verified.");
                            } else {
                                // If the packet was stored when emited in the PacketSent event.
                                if let Some(packet) = dvn_worker.packet() {
                                    dvn_worker.verifying();
                                    debug!("Packet NOT verified. Calling verification.");
                                    // FIXME: incorrect data
                                    verify(
                                        &receivelib_contract,
                                        &packet.options,
                                        &packet.encodedPayload,
                                        required_confirmations,
                                    )
                                    .await?;
                                } else {
                                    debug!("No packet data found. Skipping verification.");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to decode `DVNFeePaid` event: {:?}", e);
                    }
                }
            },
        }
        dvn_worker.reset_packet();
    }
}