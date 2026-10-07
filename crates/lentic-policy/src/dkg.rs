//! The signer's side of every loan's key generation, and the relay the borrower and the
//! lender run theirs through (`lentic_frost::relay` explains why the relay learns nothing).
//!
//! A session names the borrower's and the lender's Base addresses; the signer joins as the
//! third participant with its attested Base key, keeps its share in the store and records
//! the chain height at which the escrow came to exist (the escrow's birthday).

use std::collections::BTreeMap;

use alloy::primitives::{Address, B256};
use alloy::signers::SignerSync;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use lentic_core::Role;
use lentic_frost::dkg::{KeyPackage, PublicKeyPackage};
use lentic_frost::relay::{Done, Envelope, Participant, Round1Post, SessionView};
use lentic_frost::{fvk, group_ak};
use rand_core_06::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::api::{ApiError, ApiResult, Shared, bad_request};

/// The signer's share of one escrow key.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyShare {
    pub session: String,
    pub identities: BTreeMap<Role, Address>,
    pub key_package: KeyPackage,
    pub public_key_package: PublicKeyPackage,
    #[serde(with = "hex::serde")]
    pub fvk_seed: [u8; 32],
    pub escrow_fvk_hash: B256,
    /// Chain height when the key was made; no deposit can be older.
    pub birthday: u32,
}

impl PolicyShare {
    pub fn fvk(&self) -> anyhow::Result<orchard::keys::FullViewingKey> {
        Ok(fvk::derive_fvk(group_ak(&self.public_key_package)?, &self.fvk_seed)?)
    }
}

struct Session {
    view: SessionView,
    me: Participant,
}

#[derive(Default)]
pub struct Dkg {
    sessions: Mutex<BTreeMap<String, Session>>,
    shares: RwLock<BTreeMap<String, PolicyShare>>,
}

impl Dkg {
    pub fn with_shares(shares: Vec<PolicyShare>) -> Self {
        Dkg {
            sessions: Mutex::default(),
            shares: RwLock::new(shares.into_iter().map(|s| (s.session.clone(), s)).collect()),
        }
    }

    pub async fn share(&self, session: &str) -> Option<PolicyShare> {
        self.shares.read().await.get(session).cloned()
    }

    pub async fn share_count(&self) -> usize {
        self.shares.read().await.len()
    }
}

fn conflict(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::CONFLICT, msg.into())
}

fn sign_with(s: &Shared) -> impl FnOnce(&str) -> Result<Vec<u8>, lentic_frost::Error> + '_ {
    move |message: &str| {
        s.signer
            .sign_message_sync(message.as_bytes())
            .map(|sig| sig.as_bytes().to_vec())
            .map_err(|e| lentic_frost::Error::Relay(e.to_string()))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSession {
    /// The loan-specific Base address that will receive the USDC.
    pub borrower: Address,
    /// The lender's vault owner.
    pub lender: Address,
}

/// Opens a session; the signer posts its own round-1 package straight away.
pub async fn create(State(s): State<Shared>, Json(req): Json<NewSession>) -> ApiResult<SessionView> {
    let mut id = [0u8; 32];
    OsRng.fill_bytes(&mut id);
    let session = format!("0x{}", hex::encode(id));
    let mut me = Participant::new(Role::Policy, &session, &mut OsRng);
    let post = me.round1(&mut OsRng, sign_with(&s)).map_err(|e| anyhow::anyhow!(e))?;
    let view = SessionView {
        session: session.clone(),
        identities: BTreeMap::from([
            (Role::Borrower, req.borrower),
            (Role::Lender, req.lender),
            (Role::Policy, s.signer_address),
        ]),
        round1: vec![post],
        ..Default::default()
    };
    s.dkg.sessions.lock().await.insert(session, Session { view: view.clone(), me });
    Ok(Json(view))
}

pub async fn view(State(s): State<Shared>, Path(id): Path<String>) -> ApiResult<SessionView> {
    let sessions = s.dkg.sessions.lock().await;
    let session = sessions.get(&id).ok_or_else(|| not_found(&id))?;
    Ok(Json(session.view.clone()))
}

fn not_found(id: &str) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, format!("no DKG session {id}"))
}

pub async fn round1(
    State(s): State<Shared>,
    Path(id): Path<String>,
    Json(post): Json<Round1Post>,
) -> ApiResult<SessionView> {
    let mut sessions = s.dkg.sessions.lock().await;
    let session = sessions.get_mut(&id).ok_or_else(|| not_found(&id))?;
    if post.role == Role::Policy {
        return Err(bad_request("the policy signer posts its own round 1"));
    }
    if session.view.round1_of(post.role).is_some() {
        return Err(conflict(format!("{} already posted round 1", post.role)));
    }
    let signer = post.signer(&id).map_err(|e| bad_request(e.to_string()))?;
    let expected = session.view.identities[&post.role];
    if signer != expected {
        return Err(bad_request(format!("round 1 for {} is signed by {signer}, expected {expected}", post.role)));
    }
    session.view.round1.push(post);
    if session.view.has_round1() {
        let envelopes = session.me.round2(&session.view, &mut OsRng).map_err(|e| bad_request(e.to_string()))?;
        session.view.envelopes.extend(envelopes);
    }
    Ok(Json(session.view.clone()))
}

#[derive(Deserialize)]
pub struct Round2 {
    pub envelopes: Vec<Envelope>,
}

pub async fn round2(
    State(s): State<Shared>,
    Path(id): Path<String>,
    Json(req): Json<Round2>,
) -> ApiResult<SessionView> {
    let mut sessions = s.dkg.sessions.lock().await;
    let session = sessions.get_mut(&id).ok_or_else(|| not_found(&id))?;
    let from = req.envelopes.first().map(|e| e.from).ok_or_else(|| bad_request("no envelopes"))?;
    let well_formed = from != Role::Policy
        && req.envelopes.len() == 2
        && req.envelopes.iter().all(|e| e.from == from && e.to != from)
        && req.envelopes[0].to != req.envelopes[1].to;
    if !well_formed {
        return Err(bad_request("round 2 is one envelope from the borrower or the lender to each of the other two"));
    }
    if session.view.envelopes.iter().any(|e| e.from == from) {
        return Err(conflict(format!("{from} already posted round 2")));
    }
    session.view.envelopes.extend(req.envelopes);

    if session.view.inbox(Role::Policy).len() == 2 && s.dkg.share(&id).await.is_none() {
        let out = session.me.round3(&session.view).map_err(|e| bad_request(e.to_string()))?;
        let birthday = s.zcash_tip().await?;
        let share = PolicyShare {
            session: id.clone(),
            identities: session.view.identities.clone(),
            key_package: out.key_package,
            public_key_package: out.public_key_package,
            fvk_seed: out.fvk_seed,
            escrow_fvk_hash: out.done.escrow_fvk_hash,
            birthday,
        };
        s.store.put("shares", id.trim_start_matches("0x"), &share)?;
        s.dkg.shares.write().await.insert(id.clone(), share);
        session.view.birthday = Some(birthday);
        session.view.done.push(out.done);
    }
    Ok(Json(session.view.clone()))
}

/// A participant reports the group key it derived; it must match the signer's.
pub async fn done(State(s): State<Shared>, Path(id): Path<String>, Json(done): Json<Done>) -> ApiResult<SessionView> {
    let mut sessions = s.dkg.sessions.lock().await;
    let session = sessions.get_mut(&id).ok_or_else(|| not_found(&id))?;
    if done.role == Role::Policy {
        return Err(bad_request("the policy signer reports its own result"));
    }
    let mine = session.view.done.iter().find(|d| d.role == Role::Policy).cloned();
    match mine {
        Some(m) if m.ak == done.ak && m.escrow_fvk_hash == done.escrow_fvk_hash => {}
        Some(_) => return Err(conflict(format!("{} derived a different escrow key; abandon this session", done.role))),
        None => return Err(conflict("the policy signer has not finished yet")),
    }
    session.view.done.retain(|d| d.role != done.role);
    session.view.done.push(done);
    Ok(Json(session.view.clone()))
}
