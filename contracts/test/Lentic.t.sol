// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Test} from "forge-std/Test.sol";
import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";

import {AttestationVerifier} from "../src/AttestationVerifier.sol";
import {LenderVault} from "../src/LenderVault.sol";
import {LoanBook} from "../src/LoanBook.sol";
import {LoanTermsLib} from "../src/LoanTermsLib.sol";
import {TdxQuote} from "../src/TdxQuote.sol";
import {DevQuote} from "../src/mocks/DevQuote.sol";
import {MockDcapAttestation} from "../src/mocks/MockDcapAttestation.sol";
import {MockUSDC} from "../src/mocks/MockUSDC.sol";

contract LenticTest is Test {
    uint256 constant USDC = 1e6;
    bytes32 constant LOAN = keccak256("loan-1");
    bytes32 constant TERMS = keccak256("terms-1");
    bytes32 constant CONFIG = keccak256("policy-config-v1");

    MockUSDC usdc;
    MockDcapAttestation dcap;
    AttestationVerifier verifier;
    LoanBook book;
    LenderVault vault;

    address lender = makeAddr("lender");
    address lp = makeAddr("lp");
    address borrower = makeAddr("borrower");
    address lentic = makeAddr("lentic");
    address policy = makeAddr("policy");
    address stranger = makeAddr("stranger");

    function setUp() public {
        usdc = new MockUSDC();
        dcap = new MockDcapAttestation();
        verifier = new AttestationVerifier(dcap, DevQuote.measurement(), 7 days);
        book = new LoanBook(verifier);
        vault = new LenderVault(usdc, "Lender A Lentic Vault", "lvUSDC-A", lender, book, lentic, 1_000);

        usdc.mint(lender, 100_000 * USDC);
        usdc.mint(lp, 50_000 * USDC);
        usdc.mint(borrower, 5_000 * USDC);
        vm.prank(lender);
        usdc.approve(address(vault), type(uint256).max);
        vm.prank(lp);
        usdc.approve(address(vault), type(uint256).max);
        vm.prank(borrower);
        usdc.approve(address(vault), type(uint256).max);

        vm.prank(lender);
        vault.deposit(100_000 * USDC, lender);
        verifier.register(DevQuote.build(policy, CONFIG));
    }

    // ------------------------------------------------------------ attestation

    function test_register_bindsSignerAndConfig() public view {
        assertTrue(verifier.isPolicySigner(policy));
        assertFalse(verifier.isPolicySigner(stranger));
        (uint64 validUntil, bytes32 configHash,) = verifier.registrations(policy);
        assertEq(configHash, CONFIG);
        assertEq(validUntil, block.timestamp + 7 days);
    }

    function test_register_expires() public {
        vm.warp(block.timestamp + 7 days + 1);
        assertFalse(verifier.isPolicySigner(policy));
    }

    function test_register_rejectsFailedDcap() public {
        dcap.setAccept(false);
        vm.expectRevert(AttestationVerifier.QuoteVerificationFailed.selector);
        verifier.register(DevQuote.build(stranger, CONFIG));
    }

    function test_register_rejectsOtherCode() public {
        bytes memory quote = DevQuote.build(stranger, CONFIG);
        quote[TdxQuote.RTMR0_OFFSET + 3 * 48] = 0xff; // different RTMR3 = different compose hash
        vm.expectRevert();
        verifier.register(quote);
        assertFalse(verifier.isPolicySigner(stranger));
    }

    function test_register_rejectsMalformedQuotes() public {
        vm.expectRevert(abi.encodeWithSelector(TdxQuote.QuoteTooShort.selector, 10));
        verifier.register(new bytes(10));

        bytes memory v5 = DevQuote.build(stranger, CONFIG);
        v5[0] = 0x05;
        vm.expectRevert(abi.encodeWithSelector(TdxQuote.UnsupportedQuoteVersion.selector, 5));
        verifier.register(v5);

        bytes memory dirty = DevQuote.build(stranger, CONFIG);
        dirty[TdxQuote.REPORT_DATA_OFFSET + 25] = 0x01; // padding after the address must be zero
        vm.expectRevert(AttestationVerifier.MalformedReportData.selector);
        verifier.register(dirty);
    }

    // ------------------------------------------------------------ vault

    function test_deposit_isAllowlisted() public {
        vm.prank(lp);
        vm.expectRevert(abi.encodeWithSelector(LenderVault.DepositorNotAllowed.selector, lp));
        vault.deposit(1_000 * USDC, lp);

        vm.prank(lender);
        vault.setDepositor(lp, true);
        vm.prank(lp);
        vault.deposit(1_000 * USDC, lp);
        assertEq(vault.totalAssets(), 101_000 * USDC);
    }

    function test_fund_paysBorrowerAndOpensLoan() public {
        vm.prank(lender);
        vault.fund(LOAN, TERMS, borrower, uint128(10_000 * USDC));

        assertEq(usdc.balanceOf(borrower), 15_000 * USDC);
        assertEq(vault.totalOutstanding(), 10_000 * USDC);
        assertEq(vault.totalAssets(), 100_000 * USDC); // idle + outstanding unchanged
        (bytes32 termsHash, address loanVault, LoanBook.Health health,,,,) = book.loans(LOAN);
        assertEq(termsHash, TERMS);
        assertEq(loanVault, address(vault));
        assertEq(uint8(health), uint8(LoanBook.Health.Active));
        assertEq(book.loanCount(), 1);
    }

    function test_fund_onlyOwnerAndOnlyIdle() public {
        vm.prank(stranger);
        vm.expectRevert();
        vault.fund(LOAN, TERMS, borrower, uint128(1 * USDC));

        vm.prank(lender);
        vm.expectRevert(
            abi.encodeWithSelector(LenderVault.InsufficientIdle.selector, 100_000 * USDC, 100_001 * USDC)
        );
        vault.fund(LOAN, TERMS, borrower, uint128(100_001 * USDC));
    }

    function test_repay_splitsPrincipalInterestAndFee() public {
        vm.prank(lender);
        vault.fund(LOAN, TERMS, borrower, uint128(10_000 * USDC));

        vm.prank(borrower);
        vault.repay(LOAN, 4_000 * USDC); // all principal
        assertEq(vault.totalOutstanding(), 6_000 * USDC);

        vm.prank(borrower);
        vault.repay(LOAN, 7_200 * USDC); // 6,000 principal + 1,200 interest
        assertEq(vault.totalOutstanding(), 0);
        assertEq(usdc.balanceOf(lentic), 120 * USDC); // 10% of interest
        assertEq(vault.totalAssets(), 101_080 * USDC); // interest net of fee accrues to the vault
        (,, uint128 principalRepaid, uint128 interestPaid,) = vault.positions(LOAN);
        assertEq(principalRepaid, 10_000 * USDC);
        assertEq(interestPaid, 1_200 * USDC);
    }

    function test_writeOff_removesUnrepaidPrincipal() public {
        vm.prank(lender);
        vault.fund(LOAN, TERMS, borrower, uint128(10_000 * USDC));
        vm.prank(borrower);
        vault.repay(LOAN, 1_000 * USDC);

        vm.prank(lender);
        vault.writeOff(LOAN);
        assertEq(vault.totalOutstanding(), 0);
        assertEq(vault.totalAssets(), 91_000 * USDC);

        vm.prank(borrower);
        vm.expectRevert(abi.encodeWithSelector(LenderVault.PositionWrittenOff.selector, LOAN));
        vault.repay(LOAN, 1);
    }

    function test_withdraw_limitedToIdle() public {
        vm.prank(lender);
        vault.fund(LOAN, TERMS, borrower, uint128(60_000 * USDC));
        assertEq(vault.maxWithdraw(lender), 40_000 * USDC);
        vm.prank(lender);
        vm.expectRevert();
        vault.withdraw(40_001 * USDC, lender, lender);
        vm.prank(lender);
        vault.withdraw(40_000 * USDC, lender, lender);
    }

    // ------------------------------------------------------------ loan book

    function test_updateHealth_onlyAttestedPolicySigner() public {
        vm.prank(lender);
        vault.fund(LOAN, TERMS, borrower, uint128(10_000 * USDC));

        vm.prank(stranger);
        vm.expectRevert(abi.encodeWithSelector(LoanBook.NotPolicySigner.selector, stranger));
        book.updateHealth(LOAN, LoanBook.Health.MarginCall, 1, bytes32(0));

        vm.prank(policy);
        book.updateHealth(LOAN, LoanBook.Health.MarginCall, 7, bytes32(0));

        vm.prank(policy);
        vm.expectRevert(abi.encodeWithSelector(LoanBook.StaleOracleRound.selector, 7, 6));
        book.updateHealth(LOAN, LoanBook.Health.Active, 6, bytes32(0));

        bytes32 att = keccak256("liquidation attestation");
        vm.prank(policy);
        book.updateHealth(LOAN, LoanBook.Health.Liquidated, 9, att);
        (,,,,, uint64 round, bytes32 storedAtt) = book.loans(LOAN);
        assertEq(round, 9);
        assertEq(storedAtt, att);

        vm.prank(policy);
        vm.expectRevert(abi.encodeWithSelector(LoanBook.LoanFinalized.selector, LOAN));
        book.updateHealth(LOAN, LoanBook.Health.Active, 10, bytes32(0));
    }

    function test_open_rejectsDuplicates() public {
        vm.prank(lender);
        vault.fund(LOAN, TERMS, borrower, uint128(1 * USDC));
        vm.expectRevert(abi.encodeWithSelector(LoanBook.LoanExists.selector, LOAN));
        book.open(LOAN, TERMS);
    }

    function test_cooperativeClose_onlyByLoanVault() public {
        vm.prank(lender);
        vault.fund(LOAN, TERMS, borrower, uint128(1 * USDC));
        vm.expectRevert(abi.encodeWithSelector(LoanBook.NotLoanVault.selector, address(this)));
        book.closeByVault(LOAN);

        vm.prank(lender);
        vault.markClosed(LOAN);
        (,, LoanBook.Health health,,,,) = book.loans(LOAN);
        assertEq(uint8(health), uint8(LoanBook.Health.Closed));
    }

    function test_verifyTerms() public {
        LoanTermsLib.LoanTerms memory t;
        t.version = 1;
        t.principal = uint128(10_000 * USDC);
        t.borrower = borrower;
        bytes32 salt = keccak256("salt");
        bytes32 h = LoanTermsLib.hash(t, salt);
        vm.prank(lender);
        vault.fund(LOAN, h, borrower, uint128(10_000 * USDC));

        assertTrue(book.verifyTerms(LOAN, t, salt));
        t.principal += 1;
        assertFalse(book.verifyTerms(LOAN, t, salt));
    }
}
