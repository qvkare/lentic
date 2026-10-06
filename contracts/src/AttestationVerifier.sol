// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IAutomataDcapAttestation} from "./interfaces/IAutomataDcapAttestation.sol";
import {IPolicySignerRegistry} from "./interfaces/IPolicySignerRegistry.sol";
import {TdxQuote} from "./TdxQuote.sol";

/// @title AttestationVerifier
/// @notice Registers a policy-signer key after verifying, on chain, a TDX quote from the
///         dstack CVM that holds it. Permissionless: anyone can submit a quote; only a quote from
///         the expected code (measurement) can register a key. No owner, no upgrade path: a new
///         policy image means a new verifier (Lentic holds at most 1 of 3 escrow shares).
/// @dev REPORTDATA binds the key: [0:20] signer address, [20:32] zero, [32:64] policy config hash.
contract AttestationVerifier is IPolicySignerRegistry {
    IAutomataDcapAttestation public immutable dcap;
    bytes32 public immutable expectedMeasurement;
    uint64 public immutable registrationTtl;

    struct Registration {
        uint64 validUntil;
        bytes32 configHash;
        bytes32 quoteHash;
    }

    mapping(address signer => Registration) public registrations;

    event PolicySignerRegistered(
        address indexed signer, bytes32 configHash, bytes32 quoteHash, uint64 validUntil
    );

    error QuoteVerificationFailed();
    error MeasurementMismatch(bytes32 actual);
    error MalformedReportData();

    constructor(IAutomataDcapAttestation dcap_, bytes32 expectedMeasurement_, uint64 registrationTtl_) {
        dcap = dcap_;
        expectedMeasurement = expectedMeasurement_;
        registrationTtl = registrationTtl_;
    }

    /// @notice Verifies `quote` with Automata DCAP and registers the key it attests to.
    /// @dev Forwards msg.value as the verifier fee.
    function register(bytes calldata quote) external payable returns (address signer) {
        (bool ok,) = dcap.verifyAndAttestOnChain{value: msg.value}(quote);
        if (!ok) revert QuoteVerificationFailed();

        TdxQuote.Report memory report = TdxQuote.parse(quote);
        bytes32 m = TdxQuote.measurement(report);
        if (m != expectedMeasurement) revert MeasurementMismatch(m);

        if (uint96(uint256(report.reportDataHigh)) != 0) revert MalformedReportData();
        signer = address(bytes20(report.reportDataHigh));
        if (signer == address(0)) revert MalformedReportData();

        uint64 validUntil = uint64(block.timestamp) + registrationTtl;
        bytes32 quoteHash = keccak256(quote);
        registrations[signer] = Registration(validUntil, report.reportDataLow, quoteHash);
        emit PolicySignerRegistered(signer, report.reportDataLow, quoteHash, validUntil);
    }

    function isPolicySigner(address account) external view returns (bool) {
        return registrations[account].validUntil >= block.timestamp;
    }
}
