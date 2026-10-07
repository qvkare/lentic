//! The loan's DKG run through an untrusted relay (in Lentic, the policy signer's API).
//!
//! Round-1 packages are public. Each participant's round-1 post also carries a session key
//! (secp256k1) and is signed (EIP-191) by the participant's Base key: the borrower's
//! loan-specific address, the lender's vault owner, the policy signer's attested key. So the
//! relay cannot swap a package or a session key without the signature failing.
//!
//! Round-2 packages are secret shares and travel only as ECIES envelopes (secp256k1 ECDH,
//! BLAKE2b key derivation, ChaCha20-Poly1305) sealed to the recipient's session key. This is
//! what keeps a relay that is also a participant honest: with the borrower→lender package in
//! the clear it would hold two evaluations of the borrower's degree-1 polynomial and could
//! rebuild it, and with the lender's as well it would have the whole group key. Envelopes need
//! no sender signature: FROST's round-2 check verifies every share against the sender's signed
//! round-1 commitment. The borrower's viewing-key seed cannot be checked that way, so the
//! borrower commits to it in its signed round-1 post.

use std::collections::BTreeMap;

use alloy_primitives::{Address, B256, Signature};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use lentic_core::{Role, escrow_fvk_hash};
use rand_core_06::{CryptoRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::dkg::{KeyPackage, PublicKeyPackage};
use crate::{Error, frost, fvk, group_ak, identifier, role_of};
use frost::keys::dkg::{round1, round2};

pub const PROTOCOL: &str = "lentic-dkg-v1";

fn relay_err(e: impl std::fmt::Display) -> Error {
    Error::Relay(e.to_string())
}

fn blake2b32(personal: &[u8; 16], parts: &[&[u8]]) -> [u8; 32] {
    let mut state = blake2b_simd::Params::new().hash_length(32).personal(personal).to_state();
    for p in parts {
        state.update(p);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(state.finalize().as_bytes());
    out
}

/// EIP-191 `personal_sign` with a raw secp256k1 Base key: the 65-byte `r ‖ s ‖ v`
/// signature `ecrecover` and alloy accept.
pub fn eip191_sign(key: &k256::ecdsa::SigningKey, message: &str) -> Result<Vec<u8>, Error> {
    let hash = alloy_primitives::eip191_hash_message(message);
    let (sig, recid) = key.sign_prehash_recoverable(hash.as_slice()).map_err(relay_err)?;
    let mut out = sig.to_bytes().to_vec();
    out.push(27 + recid.to_byte());
    Ok(out)
}

/// The Base address of a secp256k1 key.
pub fn base_address(key: &k256::ecdsa::SigningKey) -> Address {
    let point = key.verifying_key().to_encoded_point(false);
    Address::from_raw_public_key(&point.as_bytes()[1..])
}

/// The Base address that signed `message` (EIP-191).
pub fn recover_signer(message: &str, signature: &[u8]) -> Result<Address, Error> {
    Signature::from_raw(signature).map_err(relay_err)?.recover_address_from_msg(message).map_err(relay_err)
}

/// Commitment to the borrower's viewing-key seed, bound to the session.
pub fn seed_commitment(session: &str, seed: &[u8; 32]) -> [u8; 32] {
    blake2b32(b"Lentic_seed_comm", &[seed, session.as_bytes()])
}

/// A participant's signed round-1 post.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Round1Post {
    pub role: Role,
    /// SEC1-compressed secp256k1 session key.
    #[serde(with = "hex::serde")]
    pub comm_key: Vec<u8>,
    pub package: round1::Package,
    /// The borrower's commitment to the viewing-key seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_commitment: Option<B256>,
    /// EIP-191 signature by the role's Base key over [`Round1Post::message`].
    #[serde(with = "hex::serde")]
    pub signature: Vec<u8>,
}

impl Round1Post {
    /// The text a participant signs with its Base key.
    pub fn message(
        session: &str,
        role: Role,
        comm_key: &[u8],
        package: &round1::Package,
        seed_commitment: Option<B256>,
    ) -> Result<String, Error> {
        let package = package.serialize().map_err(relay_err)?;
        Ok(format!(
            "{PROTOCOL}|round1|{session}|{role}|{}|{}|{}",
            hex::encode(comm_key),
            hex::encode(blake2b32(b"Lentic_r1_digest", &[&package])),
            seed_commitment.map(|c| c.to_string()).unwrap_or_else(|| "-".into())
        ))
    }

    /// The Base address that signed this post.
    pub fn signer(&self, session: &str) -> Result<Address, Error> {
        let msg = Self::message(session, self.role, &self.comm_key, &self.package, self.seed_commitment)?;
        recover_signer(&msg, &self.signature)
    }
}

/// A round-2 package sealed to its recipient.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub from: Role,
    pub to: Role,
    #[serde(with = "hex::serde")]
    pub ephemeral_key: Vec<u8>,
    #[serde(with = "hex::serde")]
    pub ciphertext: Vec<u8>,
}

/// What an envelope carries.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sealed {
    package: round2::Package,
    /// The viewing-key seed, from the borrower only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fvk_seed: Option<B256>,
}

fn aad(session: &str, from: Role, to: Role) -> String {
    format!("{PROTOCOL}|round2|{session}|{from}->{to}")
}

fn envelope_key(shared: &[u8], ephemeral: &[u8], recipient: &[u8]) -> [u8; 32] {
    blake2b32(b"Lentic_ECIES_key", &[shared, ephemeral, recipient])
}

fn seal<R: RngCore + CryptoRng>(
    session: &str,
    from: Role,
    to: Role,
    recipient: &[u8],
    sealed: &Sealed,
    rng: &mut R,
) -> Result<Envelope, Error> {
    let recipient_key = k256::PublicKey::from_sec1_bytes(recipient).map_err(relay_err)?;
    let ephemeral = k256::SecretKey::random(rng);
    let ephemeral_pub = ephemeral.public_key().to_encoded_point(true).as_bytes().to_vec();
    let shared = k256::ecdh::diffie_hellman(ephemeral.to_nonzero_scalar(), recipient_key.as_affine());
    let key = envelope_key(&shared.raw_secret_bytes()[..], &ephemeral_pub, recipient);
    let plaintext = serde_json::to_vec(sealed).map_err(relay_err)?;
    // A fresh key per envelope, so a fixed nonce is safe.
    let ciphertext = ChaCha20Poly1305::new(&Key::from(key))
        .encrypt(&Nonce::from([0u8; 12]), Payload { msg: &plaintext, aad: aad(session, from, to).as_bytes() })
        .map_err(relay_err)?;
    Ok(Envelope { from, to, ephemeral_key: ephemeral_pub, ciphertext })
}

fn open(session: &str, envelope: &Envelope, secret: &k256::SecretKey) -> Result<Sealed, Error> {
    let ephemeral = k256::PublicKey::from_sec1_bytes(&envelope.ephemeral_key).map_err(relay_err)?;
    let shared = k256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), ephemeral.as_affine());
    let mine = secret.public_key().to_encoded_point(true).as_bytes().to_vec();
    let key = envelope_key(&shared.raw_secret_bytes()[..], &envelope.ephemeral_key, &mine);
    let plaintext = ChaCha20Poly1305::new(&Key::from(key))
        .decrypt(
            &Nonce::from([0u8; 12]),
            Payload { msg: &envelope.ciphertext, aad: aad(session, envelope.from, envelope.to).as_bytes() },
        )
        .map_err(|_| relay_err(format!("envelope {}->{} does not open", envelope.from, envelope.to)))?;
    serde_json::from_slice(&plaintext).map_err(relay_err)
}

/// A participant's report of the group key it derived.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Done {
    pub role: Role,
    /// The group key (Orchard `ak`), hex.
    pub ak: String,
    /// keccak256 of the escrow FVK, as committed in the loan terms.
    pub escrow_fvk_hash: B256,
}

/// Everything the relay holds for a session (all of it public or sealed).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub session: String,
    /// Base address expected to sign each role's round-1 post.
    pub identities: BTreeMap<Role, Address>,
    pub round1: Vec<Round1Post>,
    pub envelopes: Vec<Envelope>,
    pub done: Vec<Done>,
    /// Chain height when the relay's own participant finished: the escrow's birthday.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birthday: Option<u32>,
}

impl SessionView {
    pub fn round1_of(&self, role: Role) -> Option<&Round1Post> {
        self.round1.iter().find(|p| p.role == role)
    }

    /// Envelopes addressed to `role`, one per sender.
    pub fn inbox(&self, role: Role) -> Vec<&Envelope> {
        self.envelopes.iter().filter(|e| e.to == role).collect()
    }

    pub fn has_round1(&self) -> bool {
        Role::ALL.iter().all(|r| self.round1_of(*r).is_some())
    }

    /// Checks every round-1 post's signature against the role's expected Base address.
    pub fn verify_round1(&self) -> Result<(), Error> {
        for role in Role::ALL {
            let post = self.round1_of(role).ok_or_else(|| relay_err(format!("no round-1 post from {role}")))?;
            let expected = self.identities.get(&role).ok_or_else(|| relay_err(format!("no identity for {role}")))?;
            let signer = post.signer(&self.session)?;
            if signer != *expected {
                return Err(relay_err(format!("{role}'s round-1 post is signed by {signer}, expected {expected}")));
            }
            if (role == Role::Borrower) != post.seed_commitment.is_some() {
                return Err(relay_err("only the borrower commits to the viewing-key seed"));
            }
        }
        Ok(())
    }
}

/// The result of a completed DKG for one participant.
pub struct DkgOutput {
    pub key_package: KeyPackage,
    pub public_key_package: PublicKeyPackage,
    pub fvk_seed: [u8; 32],
    pub done: Done,
}

/// One participant's side of the relayed DKG.
pub struct Participant {
    role: Role,
    session: String,
    comm: k256::SecretKey,
    fvk_seed: Option<[u8; 32]>,
    round1_secret: Option<round1::SecretPackage>,
    round2_secret: Option<round2::SecretPackage>,
}

impl Participant {
    pub fn new<R: RngCore + CryptoRng>(role: Role, session: &str, rng: &mut R) -> Self {
        let fvk_seed = (role == Role::Borrower).then(|| fvk::fvk_seed(rng));
        Participant {
            role,
            session: session.to_owned(),
            comm: k256::SecretKey::random(rng),
            fvk_seed,
            round1_secret: None,
            round2_secret: None,
        }
    }

    pub fn role(&self) -> Role {
        self.role
    }

    /// Round 1: the FROST package and session key, signed by `sign` (EIP-191 with the
    /// participant's Base key, returning the 65-byte signature).
    pub fn round1<R: RngCore + CryptoRng>(
        &mut self,
        rng: &mut R,
        sign: impl FnOnce(&str) -> Result<Vec<u8>, Error>,
    ) -> Result<Round1Post, Error> {
        let (secret, package) =
            frost::keys::dkg::part1(identifier(self.role), crate::MAX_SIGNERS, crate::MIN_SIGNERS, rng)?;
        self.round1_secret = Some(secret);
        let comm_key = self.comm.public_key().to_encoded_point(true).as_bytes().to_vec();
        let seed_commitment = self.fvk_seed.map(|s| B256::from(seed_commitment(&self.session, &s)));
        let message = Round1Post::message(&self.session, self.role, &comm_key, &package, seed_commitment)?;
        let signature = sign(&message)?;
        Ok(Round1Post { role: self.role, comm_key, package, seed_commitment, signature })
    }

    /// Round 2, once every round-1 post is in: verifies them and seals one envelope per peer.
    pub fn round2<R: RngCore + CryptoRng>(&mut self, view: &SessionView, rng: &mut R) -> Result<Vec<Envelope>, Error> {
        view.verify_round1()?;
        let secret = self.round1_secret.take().ok_or_else(|| relay_err("round 1 has not run"))?;
        let received = self.peers_round1(view);
        let (secret, outgoing) = frost::keys::dkg::part2(secret, &received)?;
        self.round2_secret = Some(secret);
        let mut envelopes = Vec::new();
        for (id, package) in outgoing {
            let to = role_of(&id).ok_or_else(|| relay_err("unknown FROST identifier"))?;
            let recipient = &view.round1_of(to).expect("verified above").comm_key;
            let sealed = Sealed { package, fvk_seed: self.fvk_seed.map(B256::from) };
            envelopes.push(seal(&self.session, self.role, to, recipient, &sealed, rng)?);
        }
        Ok(envelopes)
    }

    /// Round 3, once both peers' envelopes to this participant are in.
    pub fn round3(&mut self, view: &SessionView) -> Result<DkgOutput, Error> {
        view.verify_round1()?;
        let secret = self.round2_secret.as_ref().ok_or_else(|| relay_err("round 2 has not run"))?;
        let mut round2_received = BTreeMap::new();
        let mut seed = self.fvk_seed;
        for envelope in view.inbox(self.role) {
            let sealed = open(&self.session, envelope, &self.comm)?;
            if envelope.from == Role::Borrower {
                let s = sealed.fvk_seed.ok_or_else(|| relay_err("the borrower's envelope carries no seed"))?;
                let committed = view.round1_of(Role::Borrower).and_then(|p| p.seed_commitment);
                if committed != Some(B256::from(seed_commitment(&self.session, &s.0))) {
                    return Err(relay_err("the viewing-key seed does not match the borrower's commitment"));
                }
                seed = Some(s.0);
            }
            round2_received.insert(identifier(envelope.from), sealed.package);
        }
        if round2_received.len() != 2 {
            return Err(relay_err(format!("{} of 2 envelopes to {} are in", round2_received.len(), self.role)));
        }
        let (key, public) = frost::keys::dkg::part3(secret, &self.peers_round1(view), &round2_received)?;
        let is_even = public.has_even_y();
        use frost::keys::EvenY;
        let (key_package, public_key_package) = (key.into_even_y(Some(is_even)), public.into_even_y(Some(is_even)));
        let fvk_seed = seed.ok_or_else(|| relay_err("no viewing-key seed"))?;
        let ak = group_ak(&public_key_package)?;
        let fvk = fvk::derive_fvk(ak, &fvk_seed)?;
        let done = Done { role: self.role, ak: hex::encode(ak), escrow_fvk_hash: escrow_fvk_hash(&fvk.to_bytes()) };
        Ok(DkgOutput { key_package, public_key_package, fvk_seed, done })
    }

    fn peers_round1(&self, view: &SessionView) -> BTreeMap<frost::Identifier, round1::Package> {
        view.round1.iter().filter(|p| p.role != self.role).map(|p| (identifier(p.role), p.package.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;
    use rand_core_06::OsRng;

    fn eip191_signer(key: &SigningKey) -> impl Fn(&str) -> Result<Vec<u8>, Error> + '_ {
        move |msg: &str| eip191_sign(key, msg)
    }

    fn address(key: &SigningKey) -> Address {
        base_address(key)
    }

    fn run(tamper: impl Fn(&mut SessionView)) -> Result<BTreeMap<Role, DkgOutput>, Error> {
        let session = "0x01";
        let keys: BTreeMap<Role, SigningKey> = Role::ALL.iter().map(|r| (*r, SigningKey::random(&mut OsRng))).collect();
        let mut view = SessionView {
            session: session.into(),
            identities: keys.iter().map(|(r, k)| (*r, address(k))).collect(),
            ..Default::default()
        };
        let mut parts: BTreeMap<Role, Participant> =
            Role::ALL.iter().map(|r| (*r, Participant::new(*r, session, &mut OsRng))).collect();
        for (role, p) in parts.iter_mut() {
            view.round1.push(p.round1(&mut OsRng, eip191_signer(&keys[role]))?);
        }
        for p in parts.values_mut() {
            let envelopes = p.round2(&view, &mut OsRng)?;
            view.envelopes.extend(envelopes);
        }
        tamper(&mut view);
        parts.into_iter().map(|(r, mut p)| Ok((r, p.round3(&view)?))).collect()
    }

    #[test]
    fn relayed_dkg_agrees_on_the_escrow_key() {
        let out = run(|_| {}).unwrap();
        let dones: Vec<&Done> = out.values().map(|o| &o.done).collect();
        assert!(dones.windows(2).all(|w| w[0].ak == w[1].ak && w[0].escrow_fvk_hash == w[1].escrow_fvk_hash));
        let seeds: Vec<_> = out.values().map(|o| o.fvk_seed).collect();
        assert!(seeds.windows(2).all(|w| w[0] == w[1]));
    }

    #[test]
    fn the_relay_cannot_read_or_replace_what_it_carries() {
        // A swapped session key or package breaks the round-1 signature.
        assert!(run(|v| v.round1[0].comm_key = v.round1[1].comm_key.clone()).is_err());
        // Identities are pinned: a post signed by anyone else is refused.
        assert!(
            run(|v| {
                v.identities.insert(Role::Lender, Address::ZERO);
            })
            .is_err()
        );
        // A corrupted envelope does not open.
        assert!(run(|v| v.envelopes[0].ciphertext[0] ^= 1).is_err());
        // An envelope re-addressed to another recipient does not open either (wrong key, wrong AAD).
        assert!(
            run(|v| {
                let e = &mut v.envelopes[0];
                e.to = Role::ALL.into_iter().find(|r| *r != e.to && *r != e.from).unwrap();
            })
            .is_err()
        );
    }
}
