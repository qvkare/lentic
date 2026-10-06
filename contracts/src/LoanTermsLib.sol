// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// @notice Private loan terms. Only `hash(terms, salt)` is stored on chain (LoanBook.termsHash);
///         the borrower, the lender and the policy signer hold the preimage.
/// @dev Field order is the hash preimage order and must match `LoanTermsAbi` in
///      crates/lentic-core/src/terms.rs. Both sides are pinned to test/vectors/terms_hash.txt.
library LoanTermsLib {
    struct LoanTerms {
        uint8 version;
        uint64 chainId;
        address lenderVault;
        address borrower; // loan-specific Base address that receives the USDC
        uint128 principal; // USDC, 6 decimals
        uint32 aprBps;
        uint32 originationFeeBps;
        uint64 collateralZat; // zatoshi
        uint32 marginLtvBps;
        uint32 liquidationLtvBps;
        uint32 haircutBps;
        uint32 penaltyBps;
        uint64 fundingDeadline;
        uint64 maturity;
        bytes32 escrowFvkHash; // keccak256 of the escrow's 96-byte Orchard FVK
        string borrowerReturnAddress; // Zcash unified address
        string lenderZcashAddress; // Zcash unified address
    }

    function hash(LoanTerms memory terms, bytes32 salt) internal pure returns (bytes32) {
        return keccak256(abi.encode(terms, salt));
    }
}
