// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @notice Answers whether an address is a currently attested Lentic policy signer.
interface IPolicySignerRegistry {
    function isPolicySigner(address account) external view returns (bool);
}
