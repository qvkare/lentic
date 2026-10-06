// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {ERC4626} from "@openzeppelin/contracts/token/ERC20/extensions/ERC4626.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Math} from "@openzeppelin/contracts/utils/math/Math.sol";
import {SafeCast} from "@openzeppelin/contracts/utils/math/SafeCast.sol";

import {LoanBook} from "./LoanBook.sol";

/// @title LenderVault
/// @notice One lender's own USDC vault (Lentic runs no pooled vault). The owner is the
///         lender — the same entity that holds the lender FROST share — and is the only one who
///         funds loans. Deposits are allowlisted (the lender's own professional LPs, if any).
///         Lentic's service fee is fixed at deployment and taken from interest inside the
///         contract; Lentic never holds funds.
/// @dev totalAssets = idle USDC + outstanding principal. Interest raises share value; a write-off
///      after liquidation removes the uncovered principal. Withdrawals are limited to idle USDC.
contract LenderVault is ERC4626, Ownable {
    using SafeERC20 for IERC20;

    struct Position {
        address borrower;
        uint128 principal;
        uint128 principalRepaid;
        uint128 interestPaid;
        bool writtenOff;
    }

    LoanBook public immutable loanBook;
    address public immutable feeRecipient;
    uint16 public immutable feeBps;

    mapping(address => bool) public allowedDepositor;
    mapping(bytes32 loanId => Position) public positions;
    uint256 public totalOutstanding;

    event DepositorSet(address indexed depositor, bool allowed);
    event LoanFunded(bytes32 indexed loanId, address indexed borrower, uint256 principal);
    event LoanRepaid(
        bytes32 indexed loanId,
        address indexed payer,
        uint256 principalPart,
        uint256 interestPart,
        uint256 fee
    );
    event LoanWrittenOff(bytes32 indexed loanId, uint256 principalLost);

    error DepositorNotAllowed(address caller);
    error InsufficientIdle(uint256 idle, uint256 needed);
    error UnknownPosition(bytes32 loanId);
    error PositionWrittenOff(bytes32 loanId);
    error FeeTooHigh();

    constructor(
        IERC20 usdc,
        string memory name_,
        string memory symbol_,
        address lender,
        LoanBook loanBook_,
        address feeRecipient_,
        uint16 feeBps_
    ) ERC20(name_, symbol_) ERC4626(usdc) Ownable(lender) {
        if (feeBps_ > 2_000) revert FeeTooHigh();
        loanBook = loanBook_;
        feeRecipient = feeRecipient_;
        feeBps = feeBps_;
    }

    // ---------------------------------------------------------------- deposits / accounting

    function setDepositor(address depositor, bool allowed) external onlyOwner {
        allowedDepositor[depositor] = allowed;
        emit DepositorSet(depositor, allowed);
    }

    function idle() public view returns (uint256) {
        return IERC20(asset()).balanceOf(address(this));
    }

    function totalAssets() public view override returns (uint256) {
        return idle() + totalOutstanding;
    }

    function maxWithdraw(address holder) public view override returns (uint256) {
        return Math.min(super.maxWithdraw(holder), idle());
    }

    function maxRedeem(address holder) public view override returns (uint256) {
        return Math.min(super.maxRedeem(holder), convertToShares(idle()));
    }

    function _deposit(address caller, address receiver, uint256 assets, uint256 shares) internal override {
        if (caller != owner() && !allowedDepositor[caller]) revert DepositorNotAllowed(caller);
        super._deposit(caller, receiver, assets, shares);
    }

    function _decimalsOffset() internal pure override returns (uint8) {
        return 6;
    }

    // ---------------------------------------------------------------- loans

    /// @notice Sends `principal` USDC to the borrower's loan-specific address and opens the loan
    ///         in LoanBook. Call only after the collateral is confirmed in the escrow.
    function fund(bytes32 loanId, bytes32 termsHash, address borrower, uint128 principal) external onlyOwner {
        uint256 available = idle();
        if (available < principal) revert InsufficientIdle(available, principal);
        positions[loanId] = Position(borrower, principal, 0, 0, false);
        totalOutstanding += principal;
        loanBook.open(loanId, termsHash);
        IERC20(asset()).safeTransfer(borrower, principal);
        emit LoanFunded(loanId, borrower, principal);
    }

    /// @notice Anyone (normally the borrower) repays; principal is credited first, the rest is
    ///         interest, of which `feeBps` goes to Lentic.
    function repay(bytes32 loanId, uint256 amount) external {
        Position storage p = positions[loanId];
        if (p.principal == 0) revert UnknownPosition(loanId);
        if (p.writtenOff) revert PositionWrittenOff(loanId);
        IERC20(asset()).safeTransferFrom(msg.sender, address(this), amount);

        uint256 remaining = p.principal - p.principalRepaid;
        uint256 principalPart = Math.min(amount, remaining);
        uint256 interestPart = amount - principalPart;
        uint256 fee = interestPart * feeBps / 10_000;

        p.principalRepaid += SafeCast.toUint128(principalPart);
        p.interestPaid += SafeCast.toUint128(interestPart);
        totalOutstanding -= principalPart;
        if (fee > 0) IERC20(asset()).safeTransfer(feeRecipient, fee);
        emit LoanRepaid(loanId, msg.sender, principalPart, interestPart, fee);
    }

    /// @notice After a liquidation the lender receives ZEC, not USDC; the unrepaid principal
    ///         leaves the books here. Proceeds the lender later converts can simply be sent in.
    function writeOff(bytes32 loanId) external onlyOwner {
        Position storage p = positions[loanId];
        if (p.principal == 0) revert UnknownPosition(loanId);
        if (p.writtenOff) revert PositionWrittenOff(loanId);
        uint256 lost = p.principal - p.principalRepaid;
        p.writtenOff = true;
        totalOutstanding -= lost;
        emit LoanWrittenOff(loanId, lost);
    }

    /// @notice Records a cooperative (borrower + lender, no TEE) close in LoanBook.
    function markClosed(bytes32 loanId) external onlyOwner {
        loanBook.closeByVault(loanId);
    }
}
