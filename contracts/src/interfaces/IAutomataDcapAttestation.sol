// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @notice Automata's on-chain Intel DCAP quote verifier (AutomataDcapAttestationFee on Base).
/// @dev Only `success` is relied on; the TD report fields are read from the raw quote itself
///      (see TdxQuote), so Lentic does not depend on the verifier's output encoding.
interface IAutomataDcapAttestation {
    function verifyAndAttestOnChain(bytes calldata rawQuote)
        external
        payable
        returns (bool success, bytes memory output);
}
