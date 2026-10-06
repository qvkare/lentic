//! Zcash side of a Lentic loan.
//!
//! - [`keys`]: the escrow's FVK, Ironwood address and UFVK encodings.
//! - [`escrow`]: the PCZT pipeline for spending the escrow (close or liquidation): build,
//!   read what must be signed, check outputs, apply FROST signatures, refresh the anchor and
//!   witnesses at broadcast time, prove, extract.
//! - [`fee`]: ZIP 317 fee for a shielded-only Ironwood transaction.
//! - [`lightwalletd`]: gRPC client for chain info, tree states and broadcast.
//! - [`network`]: mainnet/testnet/regtest parameters and branch helpers.
//! - [`signing`]: runs 2-of-3 FROST over every pending escrow spend (local flows).
//! - [`sim`] (feature `sim`): fabricates an Ironwood deposit offline for tests and demos.

pub mod escrow;
pub mod fee;
pub mod keys;
pub mod lightwalletd;
pub mod network;
pub mod signing;
#[cfg(feature = "sim")]
pub mod sim;

pub use pczt::Pczt;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("PCZT builder: {0}")]
    Builder(String),
    #[error("PCZT {role}: {detail}")]
    Pczt { role: &'static str, detail: String },
    #[error("invalid address: {0}")]
    Address(String),
    #[error("output check failed: {0}")]
    OutputMismatch(String),
    #[error("lightwalletd: {0}")]
    Lightwalletd(String),
}

pub(crate) fn pczt_err(role: &'static str, e: impl std::fmt::Debug) -> Error {
    Error::Pczt { role, detail: format!("{e:?}") }
}
