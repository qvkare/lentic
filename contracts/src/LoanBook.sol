// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import {IPolicySignerRegistry} from "./interfaces/IPolicySignerRegistry.sol";
import {LenderVault} from "./LenderVault.sol";
import {LoanTermsLib} from "./LoanTermsLib.sol";

/// @title LoanBook
/// @notice Public record of Lentic loans. Stores only a commitment to the terms, the health
///         flag the attested policy signer reports, and the hash of its liquidation attestation
///         (no size, no Zcash address). The USDC transfers themselves are visible
///         on Base regardless; this contract does not add to that.
/// @dev Lenders create their vaults here, and only those vaults can open loans, so the book
///      holds no loan a real vault did not fund. A loan's id is keccak256(abi.encode(vault,
///      termsHash)): no other vault can take it first.
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
    /// @notice The USDC every vault lends.
    IERC20 public immutable usdc;
    /// @notice Lentic's share of interest, the same for every vault (Lentic never holds funds).
    address public immutable feeRecipient;
    uint16 public immutable feeBps;

    mapping(bytes32 loanId => Loan) public loans;
    mapping(address vault => bool) public isVault;
    uint256 public loanCount;

    event VaultCreated(address indexed vault, address indexed lender);
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
    error NotVault(address caller);
    error BadLoanId(bytes32 expected);
    error InvalidHealth();
    error StaleOracleRound(uint64 current, uint64 submitted);
    error FeeTooHigh();

    constructor(IPolicySignerRegistry policySigners_, IERC20 usdc_, address feeRecipient_, uint16 feeBps_) {
        if (feeBps_ > 2_000) revert FeeTooHigh();
        policySigners = policySigners_;
        usdc = usdc_;
        feeRecipient = feeRecipient_;
        feeBps = feeBps_;
    }

    /// @notice The id of the loan `vault` opens for `termsHash`.
    function loanIdOf(address vault, bytes32 termsHash) public pure returns (bytes32) {
        return keccak256(abi.encode(vault, termsHash));
    }

    /// @notice A lender creates its own vault; the caller owns it.
    function createVault(string calldata name, string calldata symbol) external returns (LenderVault vault) {
        vault = new LenderVault(usdc, name, symbol, msg.sender, this, feeRecipient, feeBps);
        isVault[address(vault)] = true;
        emit VaultCreated(address(vault), msg.sender);
    }

    /// @notice Called by a vault created here when it funds a loan; the caller becomes the loan's
    ///         vault.
    function open(bytes32 loanId, bytes32 termsHash) external {
        if (!isVault[msg.sender]) revert NotVault(msg.sender);
        bytes32 expected = loanIdOf(msg.sender, termsHash);
        if (loanId != expected) revert BadLoanId(expected);
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
