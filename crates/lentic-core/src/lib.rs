//! Types shared by every Lentic component.
//!
//! - [`terms`]: the private loan terms and the `termsHash` committed in `LoanBook` on Base.
//! - [`health`]: loan health as stored on Base (mirrors `LoanBook.Health`).
//! - [`math`]: LTV, accrued debt and the liquidation split the policy signer enforces.
//! - [`roles`]: the three FROST participants.

pub mod health;
pub mod math;
pub mod roles;
pub mod terms;

pub use health::Health;
pub use math::{LiquidationSplit, Owed, Price, Repayment, SplitError};
pub use roles::Role;
pub use terms::{LoanTerms, escrow_fvk_hash, loan_id, terms_hash};
