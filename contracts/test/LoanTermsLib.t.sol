// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Test, console2} from "forge-std/Test.sol";
import {LoanTermsLib} from "../src/LoanTermsLib.sol";

/// @notice Pins the termsHash encoding shared with crates/lentic-core (terms::tests::fixture).
contract LoanTermsLibTest is Test {
    function fixture() internal pure returns (LoanTermsLib.LoanTerms memory t, bytes32 salt) {
        t = LoanTermsLib.LoanTerms({
            version: 1,
            chainId: 8453,
            lenderVault: 0x1111111111111111111111111111111111111111,
            borrower: 0x2222222222222222222222222222222222222222,
            principal: 10_000_000_000,
            aprBps: 1_200,
            originationFeeBps: 100,
            collateralZat: 2_000_000_000,
            marginLtvBps: 6_000,
            liquidationLtvBps: 7_500,
            haircutBps: 500,
            penaltyBps: 300,
            fundingDeadline: 1_760_000_000,
            maturity: 1_791_536_000,
            escrowFvkHash: 0x3333333333333333333333333333333333333333333333333333333333333333,
            borrowerReturnAddress: "u1borrowerreturn",
            lenderZcashAddress: "u1lenderpayout"
        });
        salt = 0x4444444444444444444444444444444444444444444444444444444444444444;
    }

    function test_fixtureHash() public view {
        (LoanTermsLib.LoanTerms memory t, bytes32 salt) = fixture();
        bytes32 h = LoanTermsLib.hash(t, salt);
        console2.logBytes32(h);
        string memory expected = vm.trim(vm.readFile("test/vectors/terms_hash.txt"));
        assertEq(vm.toString(h), expected, "termsHash drifted from the shared vector");
    }

    function test_saltChangesHash() public pure {
        (LoanTermsLib.LoanTerms memory t, bytes32 salt) = fixture();
        assertTrue(LoanTermsLib.hash(t, salt) != LoanTermsLib.hash(t, bytes32(0)));
    }
}
