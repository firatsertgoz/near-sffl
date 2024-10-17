// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "@openzeppelin/contracts/access/AccessControl.sol";
import "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

import { ILayerZeroEndpointV2 } from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ILayerZeroEndpointV2.sol";
import { PacketV1Codec } from "@layerzerolabs/lz-evm-protocol-v2/contracts/messagelib/libs/PacketV1Codec.sol";
import { ISendLib } from "@layerzerolabs/lz-evm-protocol-v2/contracts/interfaces/ISendLib.sol";

import { ILayerZeroEndpoint } from "./interfaces/ILayerZeroEndpoint.sol";
import { ILayerZeroDVN } from "./interfaces/ILayerZeroDVN.sol";
import { IReceiveUlnE2, Verification, UlnConfig} from "./interfaces/IReceiveUlnE2.sol";
import "./interfaces/INuffClient.sol";
import { INuffDVNConfig } from "./interfaces/INuffDVNConfig.sol";

contract NuffDVN is ILayerZeroDVN, AccessControl {
    using PacketV1Codec for bytes;
    using ECDSA for bytes32;
    using MessageHashUtils for bytes32;

    struct Job {
        address origin;
        uint32 srcEid;
        uint32 dstEid;
        bytes packetHeader;
        bytes32 payloadHash;
        uint64 confirmations;
        address sender;
        address receiver;
        bytes options;
    }

    bytes32 public constant ADMIN_ROLE = keccak256("ADMIN_ROLE");
    bytes32 public constant MESSAGE_LIB_ROLE = keccak256("MESSAGE_LIB_ROLE");

    ILayerZeroEndpointV2 public layerZeroEndpointV2;
    ILayerZeroEndpoint public layerZeroEndpointV1;
    uint32 public immutable localEid;

    uint256 public lastJobId;

    uint256 public nuffAppId;
    INuffClient.PublicKey public nuffPublicKey;
    INuffClient public nuff;
    INuffDVNConfig public dvnConfig;

    uint256 public fee;

    mapping(uint256 => Job) public jobs;

    // eid => bool
    mapping(uint32 => bool) public supportedDstChain;
    // srcEid => ( jobId => isVerified )
    mapping(uint32 => mapping(uint256 => bool)) public verifiedJobs;

    event JobAssigned(uint256 jobId);
    event Verified(uint32 srcEid, uint256 jobId);
    event Withdraw(address lib, address to, uint256 amount);

    constructor(
        uint256 _nuffAppId,
        INuffClient.PublicKey memory _nuffPublicKey,
        address _nuff,
        address _layerZeroEndpointV2,
        address _layerZeroEndpointV1,
        address _dvnConfig
    ) {
        nuffAppId = _nuffAppId;
        nuffPublicKey = _nuffPublicKey;
        nuff = INuffClient(_nuff);
        layerZeroEndpointV2 = ILayerZeroEndpointV2(_layerZeroEndpointV2);
        layerZeroEndpointV1 = ILayerZeroEndpoint(_layerZeroEndpointV1);
        dvnConfig = INuffDVNConfig(_dvnConfig);
        localEid = layerZeroEndpointV2.eid();
        _grantRole(DEFAULT_ADMIN_ROLE, msg.sender);
        _grantRole(ADMIN_ROLE, msg.sender);
    }

    function assignJob(
        AssignJobParam calldata _param,
        bytes calldata _options
    )
        external
        payable
        override
        onlyRole(MESSAGE_LIB_ROLE)
        returns (uint256 _fee)
    {
        require(supportedDstChain[_param.dstEid], "Unsupported chain");

        uint256 jobId = ++lastJobId;
        Job storage newJob = jobs[jobId];

        newJob.origin = msg.sender;
        newJob.srcEid = layerZeroEndpointV2.eid();
        newJob.dstEid = _param.dstEid;
        newJob.packetHeader = _param.packetHeader;
        newJob.payloadHash = _param.payloadHash;
        newJob.confirmations = _param.confirmations;
        newJob.sender = _param.sender;
        newJob.receiver = address(
            uint160(uint256(_param.packetHeader.receiver()))
        );
        newJob.options = _options;

        emit JobAssigned(jobId);

        _fee = fee;
    }

    function verify(
        uint32 _srcEid,
        uint32 _dstEid,
        uint256 _jobId,
        bytes memory _packetHeader,
        bytes32 _payloadHash,
        uint64 _confirmations,
        address _receiver,
        bytes calldata _reqId,
        INuffClient.BSLSign calldata _signature,
        bytes calldata gatewaySignature
    ) external {
        require(_isLocal(_dstEid), "Invalid dstEid");
        require(
            !verifiedJobs[_srcEid][_jobId],
            "src jobId is already verified"
        );

        bytes32 hash = keccak256(
            abi.encodePacked(
                nuffAppId,
                _reqId,
                _srcEid,
                _dstEid,
                _jobId,
                _packetHeader,
                _payloadHash,
                _confirmations,
                _receiver
            )
        );

        _verifyNuffSig(
            _reqId,
            hash,
            _signature,
            dvnConfig.shieldNodes(_receiver),
            gatewaySignature
        );

        _lzVerify(
            _srcEid,
            _packetHeader,
            _payloadHash,
            _confirmations,
            _receiver
        );

        emit Verified(_srcEid, _jobId);

        verifiedJobs[_srcEid][_jobId] = true;

    }

    function setNuffAppId(uint256 _nuffAppId) external onlyRole(ADMIN_ROLE) {
        nuffAppId = _nuffAppId;
    }

    function setNuffContract(address addr) external onlyRole(ADMIN_ROLE) {
        nuff = INuffClient(addr);
    }

    function setNuffPubKey(
        INuffClient.PublicKey memory _nuffPublicKey
    ) external onlyRole(ADMIN_ROLE) {
        nuffPublicKey = _nuffPublicKey;
    }

    function setLzEndpointV2(
        address _layerZeroEndpointV2
    ) external onlyRole(ADMIN_ROLE) {
        layerZeroEndpointV2 = ILayerZeroEndpointV2(_layerZeroEndpointV2);
    }

    function updateSupportedDstChain(
        uint32 eid,
        bool status
    ) external onlyRole(ADMIN_ROLE) {
        supportedDstChain[eid] = status;
    }

    function setFee(uint256 _fee) external onlyRole(ADMIN_ROLE) {
        fee = _fee;
    }

    function withdrawFee(
        address _lib,
        address _to,
        uint256 _amount
    ) external onlyRole(ADMIN_ROLE) {
        require(hasRole(MESSAGE_LIB_ROLE, _lib), "Invalid lib");
        ISendLib(_lib).withdrawFee(_to, _amount);
        emit Withdraw(_lib, _to, _amount);
    }

    function getFee(
        uint32,        // _dstEid
        uint64,        // _confirmations
        address,       // _sender
        bytes calldata // _options
    ) external view override returns (uint256 _fee) {
        _fee = fee;
    }

    function _verifyNuffSig(
        bytes calldata reqId,
        bytes32 hash,
        INuffClient.BLSSign calldata sign,
        address nuffValidGateway,
        bytes calldata gatewaySignature
    ) internal {
        bool verified = nuff.nuffVerify(
            reqId,
            uint256(hash),
            sign,
            nuffPublicKey
        );
        require(verified, "Invalid signature!");

        if (nuffValidGateway != address(0)) {
            hash = hash.toEthSignedMessageHash();
            address gatewaySignatureSigner = hash.recover(gatewaySignature);

            require(
                gatewaySignatureSigner == nuffValidGateway,
                "Gateway is not valid"
            );
        }
    }

    function _lzVerify(
        uint32 _srcEid,
        bytes memory _packetHeader,
        bytes32 _payloadHash,
        uint64 _confirmations,
        address _receiver
    ) internal {
        address receiverLib;
        if (_isV2(_srcEid)) {
            (receiverLib, ) = layerZeroEndpointV2.getReceiveLibrary(
                _receiver,
                _srcEid
            );
        } else {
            receiverLib = layerZeroEndpointV1.getReceiveLibraryAddress(
                _receiver
            );
        }

        IReceiveUlnE2(receiverLib).verify(
            _packetHeader,
            _payloadHash,
            _confirmations
        );
    }

    function _isLocal(uint32 _dstEid) internal view returns (bool) {
        if (localEid == _dstEid || localEid == _dstEid + 30000) {
            return true;
        }
        return false;
    }

    function _isV2(uint32 _eid) internal pure returns (bool) {
        if (_eid > 30000) {
            return true;
        }
        return false;
    }
}
