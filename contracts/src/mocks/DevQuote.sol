// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {TdxQuote} from "../TdxQuote.sol";

/// @notice Builds a structurally valid (unsigned) TDX v4 quote for local deployments and tests,
///         with fixed dev measurement registers and the given REPORTDATA. Mirrors
///         `lentic_policy::attest::dev_quote`.
library DevQuote {
    function measurementRegisters() internal pure returns (bytes memory mrtd, bytes memory rtmrs) {
        mrtd = new bytes(48);
        rtmrs = new bytes(192);
        for (uint256 i = 0; i < 48; i++) {
            mrtd[i] = 0x11;
        }
        for (uint256 i = 0; i < 192; i++) {
            // forge-lint: disable-next-line(unsafe-typecast)
            rtmrs[i] = bytes1(uint8(0x20 + i / 48)); // RTMR0 = 0x20.., RTMR1 = 0x21.., ... (i < 192)
        }
    }

    function measurement() internal pure returns (bytes32) {
        (bytes memory mrtd, bytes memory rtmrs) = measurementRegisters();
        return keccak256(abi.encodePacked(mrtd, rtmrs));
    }

    function build(address signer, bytes32 configHash) internal pure returns (bytes memory quote) {
        quote = new bytes(TdxQuote.HEADER_LEN + TdxQuote.BODY_LEN);
        quote[0] = 0x04; // version 4, little-endian
        (bytes memory mrtd, bytes memory rtmrs) = measurementRegisters();
        for (uint256 i = 0; i < 48; i++) {
            quote[TdxQuote.MRTD_OFFSET + i] = mrtd[i];
        }
        for (uint256 i = 0; i < 192; i++) {
            quote[TdxQuote.RTMR0_OFFSET + i] = rtmrs[i];
        }
        bytes20 s = bytes20(signer);
        for (uint256 i = 0; i < 20; i++) {
            quote[TdxQuote.REPORT_DATA_OFFSET + i] = s[i];
        }
        for (uint256 i = 0; i < 32; i++) {
            quote[TdxQuote.REPORT_DATA_OFFSET + 32 + i] = configHash[i];
        }
    }
}
