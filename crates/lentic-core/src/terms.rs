//! Private loan terms and their on-chain commitment.
//!
//! Only `termsHash = keccak256(abi.encode(LoanTerms, salt))` is written to `LoanBook`.
//! The borrower, the lender and the policy signer hold the full terms and the salt, so any
//! of them can prove what was agreed. The encoding must match `LoanTermsLib.hash` in
//! `contracts/src/LoanTermsLib.sol`; `tests::matches_solidity_vector` pins both sides.

use alloy_primitives::{Address, B256, keccak256};
use alloy_sol_types::{SolValue, sol};
use serde::{Deserialize, Serialize};

/// The 32-byte salt hashed with the terms.
pub type Salt = B256;

/// Version of the terms layout. Bump when a field is added or reordered.
pub const TERMS_VERSION: u8 = 1;

sol! {
    /// ABI mirror of [`LoanTerms`]; field order is the hash preimage order.
    struct LoanTermsAbi {
        uint8 version;
        uint64 chainId;
        address lenderVault;
        address borrower;
        uint128 principal;
        uint32 aprBps;
        uint32 originationFeeBps;
        uint64 collateralZat;
        uint32 marginLtvBps;
        uint32 liquidationLtvBps;
        uint32 haircutBps;
        uint32 penaltyBps;
        uint64 fundingDeadline;
        uint64 maturity;
        bytes32 escrowFvkHash;
        string borrowerReturnAddress;
        string lenderZcashAddress;
    }
}

/// The terms of one loan.
///
/// Amounts: USDC in 6-decimal base units, ZEC in zatoshi (1e-8 ZEC). Times are unix seconds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoanTerms {
    pub version: u8,
    /// Base chain id the loan's USDC leg lives on (8453 mainnet, 84532 Sepolia, 31337 local).
    pub chain_id: u64,
    /// The lender's own `LenderVault`.
    pub lender_vault: Address,
    /// Loan-specific Base address that receives the USDC.
    pub borrower: Address,
    pub principal: u128,
    pub apr_bps: u32,
    pub origination_fee_bps: u32,
    /// Collateral the borrower commits to deposit into the escrow address.
    pub collateral_zat: u64,
    /// LTV at which the policy signer raises a margin call.
    pub margin_ltv_bps: u32,
    /// LTV at which the policy signer agrees to liquidate.
    pub liquidation_ltv_bps: u32,
    /// Price haircut applied when sizing the lender's share in a liquidation.
    pub haircut_bps: u32,
    /// Liquidation penalty on the outstanding debt.
    pub penalty_bps: u32,
    /// If the USDC has not arrived by this time, borrower + policy return the collateral.
    pub funding_deadline: u64,
    pub maturity: u64,
    /// keccak256 of the loan's 96-byte Orchard FVK (binds the terms to one FROST group).
    pub escrow_fvk_hash: B256,
    /// Zcash unified address the collateral (or liquidation surplus) returns to.
    pub borrower_return_address: String,
    /// Zcash unified address the lender's liquidation share is paid to.
    pub lender_zcash_address: String,
}

impl LoanTerms {
    pub fn to_abi(&self) -> LoanTermsAbi {
        LoanTermsAbi {
            version: self.version,
            chainId: self.chain_id,
            lenderVault: self.lender_vault,
            borrower: self.borrower,
            principal: self.principal,
            aprBps: self.apr_bps,
            originationFeeBps: self.origination_fee_bps,
            collateralZat: self.collateral_zat,
            marginLtvBps: self.margin_ltv_bps,
            liquidationLtvBps: self.liquidation_ltv_bps,
            haircutBps: self.haircut_bps,
            penaltyBps: self.penalty_bps,
            fundingDeadline: self.funding_deadline,
            maturity: self.maturity,
            escrowFvkHash: self.escrow_fvk_hash,
            borrowerReturnAddress: self.borrower_return_address.clone(),
            lenderZcashAddress: self.lender_zcash_address.clone(),
        }
    }

    /// Basic sanity checks every party runs before agreeing to the terms.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != TERMS_VERSION {
            return Err(format!("unsupported terms version {}", self.version));
        }
        if self.principal == 0 || self.collateral_zat == 0 {
            return Err("principal and collateral must be non-zero".into());
        }
        if !(self.margin_ltv_bps < self.liquidation_ltv_bps && self.liquidation_ltv_bps < 10_000) {
            return Err("require marginLtv < liquidationLtv < 100%".into());
        }
        if self.haircut_bps >= 10_000 {
            return Err("haircut must be below 100%".into());
        }
        if self.funding_deadline >= self.maturity {
            return Err("funding deadline must precede maturity".into());
        }
        if self.borrower_return_address.is_empty() || self.lender_zcash_address.is_empty() {
            return Err("both Zcash addresses are required".into());
        }
        Ok(())
    }
}

/// `escrowFvkHash`: keccak256 of the escrow's 96-byte Orchard full viewing key.
pub fn escrow_fvk_hash(fvk_bytes: &[u8; 96]) -> B256 {
    keccak256(fvk_bytes)
}

/// What the borrower and the lender each sign (EIP-191) to hand a loan's terms to the
/// policy signer.
pub fn approval_message(loan_id: B256, terms_hash: B256) -> String {
    format!("lentic-terms-v1|{loan_id}|{terms_hash}")
}

/// `keccak256(abi.encode(terms, salt))`, identical to `LoanTermsLib.hash` in Solidity.
pub fn terms_hash(terms: &LoanTerms, salt: B256) -> B256 {
    keccak256((terms.to_abi(), salt).abi_encode_params())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alloy_primitives::{address, b256};

    /// The same fixture as `LoanTermsLibTest.fixture()` in `contracts/test/LoanTermsLib.t.sol`.
    pub fn fixture() -> (LoanTerms, B256) {
        let terms = LoanTerms {
            version: TERMS_VERSION,
            chain_id: 8453,
            lender_vault: address!("0x1111111111111111111111111111111111111111"),
            borrower: address!("0x2222222222222222222222222222222222222222"),
            principal: 10_000_000_000, // 10,000 USDC
            apr_bps: 1_200,
            origination_fee_bps: 100,
            collateral_zat: 2_000_000_000, // 20 ZEC
            margin_ltv_bps: 6_000,
            liquidation_ltv_bps: 7_500,
            haircut_bps: 500,
            penalty_bps: 300,
            funding_deadline: 1_760_000_000,
            maturity: 1_791_536_000,
            escrow_fvk_hash: b256!("0x3333333333333333333333333333333333333333333333333333333333333333"),
            borrower_return_address: "u1borrowerreturn".into(),
            lender_zcash_address: "u1lenderpayout".into(),
        };
        let salt = b256!("0x4444444444444444444444444444444444444444444444444444444444444444");
        (terms, salt)
    }

    #[test]
    fn fixture_is_valid() {
        fixture().0.validate().unwrap();
    }

    #[test]
    fn hash_depends_on_salt_and_terms() {
        let (terms, salt) = fixture();
        let h = terms_hash(&terms, salt);
        assert_ne!(h, terms_hash(&terms, B256::ZERO));
        let mut other = terms.clone();
        other.principal += 1;
        assert_ne!(h, terms_hash(&other, salt));
    }

    #[test]
    fn matches_solidity_vector() {
        let (terms, salt) = fixture();
        // Produced by `forge test --match-test test_fixtureHash -vv` (LoanTermsLib.t.sol).
        let expected = include_str!("../../../contracts/test/vectors/terms_hash.txt").trim();
        assert_eq!(terms_hash(&terms, salt).to_string(), expected);
    }

    #[test]
    fn serde_round_trip_uses_camel_case() {
        let (terms, _) = fixture();
        let json = serde_json::to_string(&terms).unwrap();
        assert!(json.contains("\"liquidationLtvBps\":7500"));
        let back: LoanTerms = serde_json::from_str(&json).unwrap();
        assert_eq!(back, terms);
    }
}
