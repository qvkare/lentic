// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IAutomataDcapAttestation} from "../interfaces/IAutomataDcapAttestation.sol";

/// @notice Accepts every quote (or none). Local and test deployments only; on Base the real
///         Automata verifier is used.
contract MockDcapAttestation is IAutomataDcapAttestation {
    bool public accept = true;

    function setAccept(bool accept_) external {
        accept = accept_;
    }

    function verifyAndAttestOnChain(bytes calldata) external payable returns (bool, bytes memory) {
        return (accept, "");
    }
}
