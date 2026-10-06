//! Two-round re-randomized FROST signing of one PCZT spend.
//!
//! Message = the PCZT's shielded sighash; randomizer = the spend's `alpha` from the PCZT.
//! Round 1 ([`commit`]) can run before the PCZT exists; round 2 ([`sign_share`]) needs the
//! sighash and `alpha`; the coordinator (the lender's daemon in a liquidation, the borrower
//! in a close) [`aggregate`]s two shares into a 64-byte RedPallas `SpendAuth` signature.

use std::collections::BTreeMap;

use ff::PrimeField;
use orchard::primitives::redpallas::{self, SpendAuth};
use pasta_curves::pallas;
use rand_core_06::{CryptoRng, RngCore};

use crate::{Error, frost};

pub use frost::SigningPackage;
pub use frost::round1::{SigningCommitments, SigningNonces};
pub use frost::round2::SignatureShare;

type Suite = frost::PallasBlake2b512;

fn randomizer(alpha: &[u8; 32]) -> Result<frost_rerandomized::Randomizer<Suite>, Error> {
    frost_rerandomized::Randomizer::<Suite>::deserialize(alpha).map_err(|e| Error::Randomizer(e.to_string()))
}

/// Round 1: fresh nonces (kept secret, used once) and their public commitments.
pub fn commit<R: RngCore + CryptoRng>(
    key: &frost::keys::KeyPackage,
    rng: &mut R,
) -> (SigningNonces, SigningCommitments) {
    frost::round1::commit(key.signing_share(), rng)
}

/// The package every signer signs: the two signers' commitments and the sighash.
pub fn signing_package(
    commitments: BTreeMap<frost::Identifier, SigningCommitments>,
    sighash: &[u8; 32],
) -> SigningPackage {
    SigningPackage::new(commitments, sighash)
}

/// Round 2: this participant's signature share for the spend randomized by `alpha`.
///
/// The caller must have validated the PCZT (outputs, values, that `alpha` belongs to the
/// spend being signed) before calling this.
pub fn sign_share(
    package: &SigningPackage,
    nonces: &SigningNonces,
    key: &frost::keys::KeyPackage,
    alpha: &[u8; 32],
) -> Result<SignatureShare, Error> {
    // See the crate docs: a PCZT fixes alpha before signing, so the commitment-derived
    // randomizer API cannot be used (frost#1094).
    #[allow(deprecated)]
    let share = frost_rerandomized::sign::<Suite>(package, nonces, key, randomizer(alpha)?)?;
    Ok(share)
}

/// Aggregates shares into a RedPallas signature valid under `rk = ak + [alpha]G`.
pub fn aggregate(
    package: &SigningPackage,
    shares: &BTreeMap<frost::Identifier, SignatureShare>,
    public: &frost::keys::PublicKeyPackage,
    alpha: &[u8; 32],
) -> Result<[u8; 64], Error> {
    let params = frost_rerandomized::RandomizedParams::from_randomizer(public.verifying_key(), randomizer(alpha)?);
    let signature = frost_rerandomized::aggregate::<Suite>(package, shares, public, &params)?;
    let bytes = signature.serialize()?;
    bytes.as_slice().try_into().map_err(|_| Error::BadSignature)
}

/// Verifies a spend authorization signature against `ak` randomized by `alpha`, exactly as
/// consensus does against the action's `rk`.
pub fn verify_spend_auth(
    ak: &[u8; 32],
    alpha: &[u8; 32],
    sighash: &[u8; 32],
    signature: &[u8; 64],
) -> Result<(), Error> {
    let ak = redpallas::VerificationKey::<SpendAuth>::try_from(*ak).map_err(|_| Error::InvalidAk)?;
    let alpha = Option::<pallas::Scalar>::from(pallas::Scalar::from_repr(*alpha))
        .ok_or_else(|| Error::Randomizer("alpha is not a canonical Pallas scalar".into()))?;
    let rk = ak.randomize(&alpha);
    rk.verify(sighash, &redpallas::Signature::<SpendAuth>::from(*signature)).map_err(|_| Error::BadSignature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dkg, group_ak, identifier};
    use lentic_core::Role;
    use rand_core_06::OsRng;

    fn random_alpha() -> [u8; 32] {
        use ff::FromUniformBytes;
        let mut wide = [0u8; 64];
        OsRng.fill_bytes(&mut wide);
        pallas::Scalar::from_uniform_bytes(&wide).to_repr()
    }

    fn sign_with(
        dkg: &dkg::LocalDkg,
        signers: &[Role],
        sighash: &[u8; 32],
        alpha: &[u8; 32],
    ) -> Result<[u8; 64], Error> {
        let mut nonces = BTreeMap::new();
        let mut commitments = BTreeMap::new();
        for role in signers {
            let (n, c) = commit(&dkg.keys[role], &mut OsRng);
            nonces.insert(*role, n);
            commitments.insert(identifier(*role), c);
        }
        let package = signing_package(commitments, sighash);
        let mut shares = BTreeMap::new();
        for role in signers {
            shares.insert(identifier(*role), sign_share(&package, &nonces[role], &dkg.keys[role], alpha)?);
        }
        aggregate(&package, &shares, &dkg.public, alpha)
    }

    #[test]
    fn any_two_roles_produce_a_valid_randomized_signature() {
        let dkg = dkg::run_local(&mut OsRng).unwrap();
        let ak = group_ak(&dkg.public).unwrap();
        let sighash = [42u8; 32];
        for pair in [
            [Role::Lender, Role::Policy],   // liquidation
            [Role::Borrower, Role::Policy], // close
            [Role::Borrower, Role::Lender], // cooperative close without the TEE
        ] {
            let alpha = random_alpha();
            let sig = sign_with(&dkg, &pair, &sighash, &alpha).unwrap();
            verify_spend_auth(&ak, &alpha, &sighash, &sig).unwrap();
            // Bound to this alpha and this message.
            assert!(verify_spend_auth(&ak, &random_alpha(), &sighash, &sig).is_err());
            assert!(verify_spend_auth(&ak, &alpha, &[0u8; 32], &sig).is_err());
        }
    }

    #[test]
    fn one_role_alone_cannot_sign() {
        let dkg = dkg::run_local(&mut OsRng).unwrap();
        for role in Role::ALL {
            assert!(sign_with(&dkg, &[role], &[1u8; 32], &random_alpha()).is_err());
        }
    }
}
