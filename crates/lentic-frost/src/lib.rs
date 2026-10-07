//! 2-of-3 re-randomized FROST over RedPallas (ZIP 312) for a Lentic loan escrow.
//!
//! Every loan gets its own group: the borrower, the lender and the TEE policy signer run a
//! DKG ([`dkg`]), the group verifying key becomes the Orchard spend validating key `ak`, and
//! the borrower contributes a random seed from which `nk` and `rivk` are derived ([`fvk`]).
//! All three parties therefore hold the same full viewing key; only spend authorization is
//! threshold-protected.
//!
//! Signing ([`sign`]) uses the PCZT's per-action randomizer `alpha` as the FROST randomizer,
//! so the aggregated signature verifies against the action's `rk = ak + [alpha]G`.
//!
//! [`relay`] runs the DKG through an untrusted relay (the policy signer's API): signed round-1
//! posts and round-2 packages sealed to each recipient.
//!
//! Known limitation (frost#1094): frost-rerandomized 3.0 deprecates signing with
//! a caller-supplied randomizer in favour of deriving it from the signing commitments, but a
//! PCZT fixes `alpha` (and `rk`, which the sighash commits to) before signing starts. Each
//! signer must therefore check that `alpha` is the one bound in the PCZT it is asked to sign.

pub mod dkg;
pub mod fvk;
pub mod relay;
pub mod sign;

pub use reddsa_frost::frost::redpallas as frost;

use lentic_core::Role;

/// Threshold: any two of the three roles can authorize a spend.
pub const MIN_SIGNERS: u16 = 2;
/// Borrower, lender, policy signer.
pub const MAX_SIGNERS: u16 = 3;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("FROST: {0}")]
    Frost(#[from] frost::Error),
    #[error("FROST randomizer: {0}")]
    Randomizer(String),
    #[error("group key is not a valid Orchard spend validating key")]
    InvalidAk,
    #[error("could not derive a valid full viewing key from the seed")]
    FvkDerivation,
    #[error("signature does not verify against the randomized key")]
    BadSignature,
    #[error("DKG relay: {0}")]
    Relay(String),
}

/// FROST identifier of a role (its discriminant: borrower 1, lender 2, policy 3).
pub fn identifier(role: Role) -> frost::Identifier {
    frost::Identifier::try_from(role.id()).expect("role ids are non-zero")
}

/// The role a FROST identifier belongs to.
pub fn role_of(id: &frost::Identifier) -> Option<Role> {
    Role::ALL.into_iter().find(|r| identifier(*r) == *id)
}

/// The group verifying key as Orchard `ak` bytes (`I2LEOSP_256`, ỹ = 0).
pub fn group_ak(public: &frost::keys::PublicKeyPackage) -> Result<[u8; 32], Error> {
    let bytes = public.verifying_key().serialize()?;
    let ak: [u8; 32] = bytes.as_slice().try_into().map_err(|_| Error::InvalidAk)?;
    if ak[31] & 0x80 != 0 {
        return Err(Error::InvalidAk);
    }
    Ok(ak)
}
