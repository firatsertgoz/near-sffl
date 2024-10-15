//! Main offchain workflow for Nuff DVN.

use alloy::{
    primitives::hex,
    sol_types::{SolCall, SolEventInterface, SolInterface},
};
use eyre::Result;
use futures::stream::StreamExt;
use tracing::{debug, error, info};
use tracing_subscriber::EnvFilter;
use workers::abi::{L0V2EndpointAbi::L0V2EndpointAbiEvents, SendLibraryAbi::SendLibraryAbiEvents};
use workers::{
    abi::{
        L0V2EndpointAbi::{self, L0V2EndpointAbiCalls, PacketSent},
        ReceiveLibraryAbi::{self, ReceiveLibraryAbiCalls},
        SendLibraryAbi::{self, DVNFeePaid, SendLibraryAbiCalls},
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

    // Initialize the DVN worker.
    let mut dvn_data = Dvn::new_from_env()?;

    // Create the WS subscriptions for listening to the events.
    let (_provider, mut endpoint_stream, mut sendlib_stream) = build_subscriptions(dvn_data.config()).await?;

    // Create an HTTP provider to call contract functions.
    let http_provider = get_http_provider(dvn_data.config())?;

    // Get the relevant contract ABI, and create contract.
    let receivelib_abi = get_abi_from_path("./abi/ArbitrumReceiveLibUln302.json")?;
    let receivelib_contract = create_contract_instance(dvn_data.config(), http_provider, receivelib_abi)?;

    info!("Listening to chain events...");

    loop {
        dvn_data.listening();
        tokio::select! {
            Some(log) = endpoint_stream.next() => {
                match log.log_decode::<PacketSent>() {
                    Err(e) => {
                        error!("Received an event but failed to decode as `PacketSent`: {:?}", e);
                    }
                    Ok(inner_log) => {
                        debug!("PacketSent event found and decoded: {:?}", inner_log);
                        dvn_data.packet_received(inner_log.data().clone());
                    },
                }
            }
            Some(log) = sendlib_stream.next() => {
                match log.log_decode::<DVNFeePaid>() {
                    Err(e) => {
                        error!("Received an event but failed to decode as `DVNFeePaid`: {:?}", e);
                    }
                    Ok(inner_log) => {
                        if let Some(packet) = dvn_data.packet() {

                            info!("DVNFeePaid event found and decoded.");
                            let required_dvns = inner_log.inner.requiredDVNs.clone();

                            if required_dvns.contains(&dvn_data.config().dvn_addr()?) {
                                debug!("Found DVN in required DVNs.");

                                // NOTE: the docs' workflow require now to query L0's endpoint to
                                // get the address of the MessageLib, but we have already created
                                // the contract above to query it directly.

                                let required_confirmations =
                                    query_confirmations(&receivelib_contract, dvn_data.config().eid()).await?;

                                // Prepare the header
                                let header: Vec<u8> = vec![1,2,3,4];
                                // Prepate the payload.
                                let payload: Vec<u8> = packet.encodedPayload.to_vec();

                                // Check
                                let already_verified = query_already_verified(
                                    &receivelib_contract,
                                    dvn_data.config().dvn_addr()?,
                                    header.clone(),
                                    payload.clone(),
                                    required_confirmations,
                                )
                                .await?;

                                if already_verified {
                                    debug!("Packet already verified.");
                                } else {
                                    // If the packet was stored when emited in the PacketSent event.
                                    if let Some(packet) = dvn_data.packet() {
                                        dvn_data.verifying();
                                        debug!("Packet NOT verified. Calling verification.");
                                        println!("->> Packet: {:?}", packet);
                                        // FIXME: incorrect data
                                        verify(
                                            &receivelib_contract,
                                            header,
                                            payload,
                                            required_confirmations,
                                        )
                                        .await?;
                                    } else {
                                        debug!("No packet data found. Skipping verification.");
                                    }
                                }
                            }
                        }
                    }
                }
            },
        }
        dvn_data.reset_packet();
    }
}
