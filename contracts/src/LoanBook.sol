// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IPolicySignerRegistry} from "./interfaces/IPolicySignerRegistry.sol";
import {LoanTermsLib} from "./LoanTermsLib.sol";

/// @title LoanBook
/// @notice Public record of Lentic loans. Stores only a commitment to the terms, the health
///         flag the attested policy signer reports, and the hash of its liquidation attestation
///         (no size, no Zcash address). The USDC transfers themselves are visible
///         on Base regardless; this contract does not add to that.
contract LoanBook {
    /// @dev Must stay in sync with lentic_core::Health.
    enum Health {
        None,
        Active,
        MarginCall,
        Liquidating,
        Closed,
        Liquidated
    }

    struct Loan {
        bytes32 termsHash;
        address vault;
        Health health;
        uint64 openedAt;
        uint64 updatedAt;
        uint64 oracleRound;
        bytes32 attestationHash;
    }

    IPolicySignerRegistry public immutable policySigners;
    mapping(bytes32 loanId => Loan) public loans;
    uint256 public loanCount;

    event LoanOpened(bytes32 indexed loanId, address indexed vault, bytes32 termsHash);
    event HealthUpdated(
        bytes32 indexed loanId,
        Health health,
        uint64 oracleRound,
        bytes32 attestationHash,
        address indexed signer
    );

    error LoanExists(bytes32 loanId);
    error UnknownLoan(bytes32 loanId);
    error LoanFinalized(bytes32 loanId);
    error NotPolicySigner(address caller);
    error NotLoanVault(address caller);
    error InvalidHealth();
    error StaleOracleRound(uint64 current, uint64 submitted);

    constructor(IPolicySignerRegistry policySigners_) {
        policySigners = policySigners_;
    }

    /// @notice Called by a LenderVault when it funds a loan; the caller becomes the loan's vault.
    function open(bytes32 loanId, bytes32 termsHash) external {
        if (loans[loanId].health != Health.None) revert LoanExists(loanId);
        loans[loanId] = Loan({
            termsHash: termsHash,
            vault: msg.sender,
            health: Health.Active,
            openedAt: uint64(block.timestamp),
            updatedAt: uint64(block.timestamp),
            oracleRound: 0,
            attestationHash: bytes32(0)
        });
        unchecked {
            ++loanCount;
        }
        emit LoanOpened(loanId, msg.sender, termsHash);
    }

    /// @notice Health report from an attested policy signer.
    function updateHealth(bytes32 loanId, Health health, uint64 oracleRound, bytes32 attestationHash)
        external
    {
        if (!policySigners.isPolicySigner(msg.sender)) revert NotPolicySigner(msg.sender);
        Loan storage loan = _live(loanId);
        if (health == Health.None) revert InvalidHealth();
        if (oracleRound < loan.oracleRound) revert StaleOracleRound(loan.oracleRound, oracleRound);
        loan.health = health;
        loan.oracleRound = oracleRound;
        loan.attestationHash = attestationHash;
        loan.updatedAt = uint64(block.timestamp);
        emit HealthUpdated(loanId, health, oracleRound, attestationHash, msg.sender);
    }

    /// @notice Borrower + lender may close cooperatively without the TEE (2-of-3); the lender's
    ///         vault then records the close itself.
    function closeByVault(bytes32 loanId) external {
        Loan storage loan = _live(loanId);
        if (msg.sender != loan.vault) revert NotLoanVault(msg.sender);
        loan.health = Health.Closed;
        loan.updatedAt = uint64(block.timestamp);
        emit HealthUpdated(loanId, Health.Closed, loan.oracleRound, loan.attestationHash, msg.sender);
    }

    /// @notice Lets any party holding the preimage prove the agreed terms (disputes).
    function verifyTerms(bytes32 loanId, LoanTermsLib.LoanTerms calldata terms, bytes32 salt)
        external
        view
        returns (bool)
    {
        Loan storage loan = loans[loanId];
        return loan.health != Health.None && LoanTermsLib.hash(terms, salt) == loan.termsHash;
    }

    function _live(bytes32 loanId) private view returns (Loan storage loan) {
        loan = loans[loanId];
        if (loan.health == Health.None) revert UnknownLoan(loanId);
        if (loan.health == Health.Closed || loan.health == Health.Liquidated) revert LoanFinalized(loanId);
    }
}
