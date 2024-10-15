// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./NuffClientBase.sol";

contract NuffClient is NuffClientBase {
    constructor(uint256 _nuffAppId, PublicKey memory _nuffPublicKey) {
        validatePubKey(_nuffPublicKey.x);

        nuffAppId = _nuffAppId;
        nuffPublicKey = _nuffPublicKey;
    }
}
