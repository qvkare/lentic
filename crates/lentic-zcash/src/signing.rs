//! Glue between the escrow PCZT and FROST: who signs what, with which randomizer.

use std::collections::BTreeMap;

use lentic_core::Role;
use lentic_frost::{dkg::KeyPackage, dkg::PublicKeyPackage, identifier, sign};
use rand_core_06::OsRng;

use crate::{Error, Pczt, escrow};

fn frost_err(e: lentic_frost::Error) -> Error {
    Error::Pczt { role: "frost", detail: e.to_string() }
}

/// Runs both FROST rounds for every pending escrow spend with two roles in one process and
/// returns `(action index, signature)` pairs ready for
/// [`escrow::apply_spend_auth_signatures`]. For tests and the local demo; in production each
/// role signs on its own machine after running its own PCZT checks.
pub fn sign_locally(
    pczt: &Pczt,
    keys: &BTreeMap<Role, KeyPackage>,
    public: &PublicKeyPackage,
    signers: [Role; 2],
) -> Result<Vec<(usize, [u8; 64])>, Error> {
    if signers[0] == signers[1] {
        return Err(Error::Pczt { role: "frost", detail: "two distinct signers required".into() });
    }
    let sighash = escrow::shielded_sighash(pczt)?;
    let mut signatures = Vec::new();
    for spend in escrow::spends_to_sign(pczt)? {
        let mut nonces = BTreeMap::new();
        let mut commitments = BTreeMap::new();
        for role in signers {
            let key = keys
                .get(&role)
                .ok_or_else(|| Error::Pczt { role: "frost", detail: format!("no key package for {role}") })?;
            let (n, c) = sign::commit(key, &mut OsRng);
            nonces.insert(role, n);
            commitments.insert(identifier(role), c);
        }
        let package = sign::signing_package(commitments, &sighash);
        let mut shares = BTreeMap::new();
        for role in signers {
            let share = sign::sign_share(&package, &nonces[&role], &keys[&role], &spend.alpha).map_err(frost_err)?;
            shares.insert(identifier(role), share);
        }
        let sig = sign::aggregate(&package, &shares, public, &spend.alpha).map_err(frost_err)?;
        signatures.push((spend.index, sig));
    }
    Ok(signatures)
}
