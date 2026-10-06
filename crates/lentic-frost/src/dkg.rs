//! Three-round DKG for one loan, plus an in-process runner for tests and local demos.
//!
//! Orchard requires `ak` to have an even y-coordinate. After round 3 every participant
//! checks the (common) group key and, if it is odd, negates its own share and the public
//! package; negation is linear, so the shares stay consistent (reddsa `keys::EvenY`).

use std::collections::BTreeMap;

use lentic_core::Role;
use rand_core_06::{CryptoRng, RngCore};

use crate::{Error, MAX_SIGNERS, MIN_SIGNERS, frost, identifier};
use frost::keys::EvenY;
use frost::keys::dkg::{round1, round2};

pub use frost::keys::{KeyPackage, PublicKeyPackage};

/// Round 1: commit to a random polynomial. Broadcast the package, keep the secret.
pub fn part1<R: RngCore + CryptoRng>(
    role: Role,
    rng: &mut R,
) -> Result<(round1::SecretPackage, round1::Package), Error> {
    Ok(frost::keys::dkg::part1(identifier(role), MAX_SIGNERS, MIN_SIGNERS, rng)?)
}

/// Round 2: given the other two round-1 packages, produce one private package per peer.
pub fn part2(
    secret: round1::SecretPackage,
    received: &BTreeMap<frost::Identifier, round1::Package>,
) -> Result<(round2::SecretPackage, BTreeMap<frost::Identifier, round2::Package>), Error> {
    Ok(frost::keys::dkg::part2(secret, received)?)
}

/// Round 3: derive this participant's long-lived share and the group's public package,
/// normalized so that the group key is a valid Orchard `ak`.
pub fn part3(
    secret: &round2::SecretPackage,
    round1_received: &BTreeMap<frost::Identifier, round1::Package>,
    round2_received: &BTreeMap<frost::Identifier, round2::Package>,
) -> Result<(KeyPackage, PublicKeyPackage), Error> {
    let (key, public) = frost::keys::dkg::part3(secret, round1_received, round2_received)?;
    let is_even = public.has_even_y();
    Ok((key.into_even_y(Some(is_even)), public.into_even_y(Some(is_even))))
}

/// Output of [`run_local`]: every role's share and the common public package.
pub struct LocalDkg {
    pub keys: BTreeMap<Role, KeyPackage>,
    pub public: PublicKeyPackage,
}

/// Runs the full DKG for all three roles in one process. For tests and the local demo only:
/// in production each role runs its rounds on its own machine and exchanges packages over
/// an authenticated, encrypted channel (frostd).
pub fn run_local<R: RngCore + CryptoRng>(rng: &mut R) -> Result<LocalDkg, Error> {
    let mut r1_secrets = BTreeMap::new();
    let mut r1_packages = BTreeMap::new();
    for role in Role::ALL {
        let (secret, package) = part1(role, rng)?;
        r1_secrets.insert(role, secret);
        r1_packages.insert(identifier(role), package);
    }

    let others = |me: Role, all: &BTreeMap<frost::Identifier, round1::Package>| {
        all.iter().filter(|(id, _)| **id != identifier(me)).map(|(id, p)| (*id, p.clone())).collect::<BTreeMap<_, _>>()
    };

    let mut r2_secrets = BTreeMap::new();
    // r2_inbox[recipient][sender]
    let mut r2_inbox: BTreeMap<frost::Identifier, BTreeMap<frost::Identifier, round2::Package>> = BTreeMap::new();
    for role in Role::ALL {
        let secret = r1_secrets.remove(&role).expect("every role ran round 1");
        let (r2_secret, outgoing) = part2(secret, &others(role, &r1_packages))?;
        r2_secrets.insert(role, r2_secret);
        for (recipient, package) in outgoing {
            r2_inbox.entry(recipient).or_default().insert(identifier(role), package);
        }
    }

    let mut keys = BTreeMap::new();
    let mut public = None;
    for role in Role::ALL {
        let inbox = r2_inbox.remove(&identifier(role)).unwrap_or_default();
        let (key, pkg) = part3(&r2_secrets[&role], &others(role, &r1_packages), &inbox)?;
        if let Some(prev) = &public {
            debug_assert_eq!(prev, &pkg, "all participants derive the same public package");
        }
        public = Some(pkg);
        keys.insert(role, key);
    }

    Ok(LocalDkg { keys, public: public.expect("three participants") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group_ak;
    use rand_core_06::OsRng;

    #[test]
    fn local_dkg_produces_even_y_orchard_ak() {
        // Run several times: an odd group key shows up with probability 1/2 per run.
        for _ in 0..8 {
            let dkg = run_local(&mut OsRng).unwrap();
            assert_eq!(dkg.keys.len(), 3);
            let ak = group_ak(&dkg.public).unwrap();
            assert_eq!(ak[31] & 0x80, 0);
            assert!(
                orchard::keys::FullViewingKey::from_bytes(&{
                    let mut b = [0u8; 96];
                    b[..32].copy_from_slice(&ak);
                    b[32..64].copy_from_slice(&[1; 32]);
                    b[64..].copy_from_slice(&[2; 32]);
                    b
                })
                .is_some()
            );
            for key in dkg.keys.values() {
                assert_eq!(key.verifying_key(), dkg.public.verifying_key());
                assert_eq!(*key.min_signers(), MIN_SIGNERS);
            }
        }
    }
}
