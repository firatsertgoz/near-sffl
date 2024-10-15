//! Types create from the JSON ABI files.
//!
//! For example, to be able to decode the logs' data, or call contracts' methods.

use alloy::sol;
use serde::{Deserialize, Serialize};

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    #[derive(Debug, Serialize, Deserialize)]
    SendLibraryAbi,
    "abi/ArbitrumSendLibUln302.json"
);

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    #[derive(Debug, Serialize, Deserialize)]
    ReceiveLibraryAbi,
    "abi/ArbitrumReceiveLibUln302.json"
);

sol!(
    #[allow(missing_docs)]
    //#[sol(rpc)]
    #[derive(Debug, Serialize, Deserialize)]
    L0V2EndpointAbi,
    "abi/ArbitrumL0V2Endpoint.json"
);

sol!(
    #[allow(missing_docs)]
    #[derive(Debug)]
    struct Packet {
        uint64 nonce;     // the nonce of the message in the pathway
        uint32 srcEid;    // the source endpoint ID
        address sender;   // the sender address
        uint32 dstEid;    // the destination endpoint ID
        bytes32 receiver; // the receiving address
        bytes32 guid;     // a global unique identifier
        bytes message;    // the message payload
    }
);
