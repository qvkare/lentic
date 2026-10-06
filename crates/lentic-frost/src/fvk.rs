//! Full viewing key for a FROST-controlled escrow.
//!
//! ZIP 312 only thresholds `ask`; `nk` and `rivk` are ordinary secrets shared by all
//! participants. The borrower (whose privacy is at stake) samples a 32-byte seed and sends it
//! to the lender and the policy signer over the encrypted DKG channel; everyone derives
//! `nk` and `rivk` from it. Nobody can compute the FVK from public data.
//!
//! Such an FVK is not ZIP 2005 quantum-recoverable (there is no `sk` behind `ak`).

use ff::{FromUniformBytes, PrimeField};
use orchard::keys::FullViewingKey;
use pasta_curves::pallas;
use rand_core_06::{CryptoRng, RngCore};

use crate::Error;

const NK_PERSONAL: &[u8; 16] = b"Lentic_FVK_nk___";
const RIVK_PERSONAL: &[u8; 16] = b"Lentic_FVK_rivk_";

/// A fresh viewing-key seed (sampled by the borrower).
pub fn fvk_seed<R: RngCore + CryptoRng>(rng: &mut R) -> [u8; 32] {
    let mut seed = [0u8; 32];
    rng.fill_bytes(&mut seed);
    seed
}

fn wide_hash(personal: &[u8; 16], seed: &[u8; 32], counter: u8) -> [u8; 64] {
    let hash = blake2b_simd::Params::new()
        .hash_length(64)
        .personal(personal)
        .to_state()
        .update(seed)
        .update(&[counter])
        .finalize();
    *hash.as_array()
}

/// Derives the escrow FVK from the group `ak` and the borrower's seed.
pub fn derive_fvk(ak: [u8; 32], seed: &[u8; 32]) -> Result<FullViewingKey, Error> {
    if !ak_is_valid(&ak) {
        return Err(Error::InvalidAk);
    }
    // `from_bytes` rejects the (negligible-probability) cases where the derived ivk is
    // invalid; retry with a counter so the derivation is total and deterministic.
    for counter in 0u8..=16 {
        let nk = pallas::Base::from_uniform_bytes(&wide_hash(NK_PERSONAL, seed, counter));
        let rivk = pallas::Scalar::from_uniform_bytes(&wide_hash(RIVK_PERSONAL, seed, counter));
        let mut bytes = [0u8; 96];
        bytes[..32].copy_from_slice(&ak);
        bytes[32..64].copy_from_slice(&nk.to_repr());
        bytes[64..].copy_from_slice(&rivk.to_repr());
        if let Some(fvk) = FullViewingKey::from_bytes(&bytes) {
            return Ok(fvk);
        }
    }
    Err(Error::FvkDerivation)
}

fn ak_is_valid(ak: &[u8; 32]) -> bool {
    *ak != [0; 32]
        && ak[31] & 0x80 == 0
        && orchard::primitives::redpallas::VerificationKey::<orchard::primitives::redpallas::SpendAuth>::try_from(*ak)
            .is_ok()
}

/// keccak-free binding of an FVK for display; the on-chain binding (`escrowFvkHash`) is
/// computed by callers with keccak256 over [`FullViewingKey::to_bytes`].
pub fn fvk_fingerprint(fvk: &FullViewingKey) -> String {
    let hash = blake2b_simd::Params::new().hash_length(8).personal(b"Lentic_FVK_fp___").hash(&fvk.to_bytes());
    hex::encode(hash.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dkg, group_ak};
    use rand_core_06::OsRng;

    #[test]
    fn derivation_is_deterministic_and_seed_bound() {
        let dkg = dkg::run_local(&mut OsRng).unwrap();
        let ak = group_ak(&dkg.public).unwrap();
        let seed = fvk_seed(&mut OsRng);
        let a = derive_fvk(ak, &seed).unwrap();
        let b = derive_fvk(ak, &seed).unwrap();
        assert_eq!(a.to_bytes(), b.to_bytes());
        assert_eq!(&a.to_bytes()[..32], &ak);
        let c = derive_fvk(ak, &fvk_seed(&mut OsRng)).unwrap();
        assert_ne!(a.to_bytes(), c.to_bytes());
        assert_eq!(fvk_fingerprint(&a).len(), 16);
    }

    #[test]
    fn rejects_invalid_ak() {
        assert!(matches!(derive_fvk([0; 32], &[7; 32]), Err(Error::InvalidAk)));
        let mut odd = [1u8; 32];
        odd[31] |= 0x80;
        assert!(matches!(derive_fvk(odd, &[7; 32]), Err(Error::InvalidAk)));
    }
}
