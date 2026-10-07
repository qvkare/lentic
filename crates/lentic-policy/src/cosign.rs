//! Co-signing escrow spends: the signer's half of every refund, close and liquidation, and
//! the checks it runs before contributing a share.
//!
//! The signer never takes the coordinator's numbers. For every request it reads the escrow
//! from its own scan, the debt from the vault's repayment events, the price from its own
//! feed and the deadlines from the terms, works out the only payments it will accept (the
//! plan), and checks the PCZT against them:
//!
//! - every pending spend is an unspent escrow note of the stated value, and every escrow note
//!   is spent: nothing is left behind and nothing else is spent;
//! - the outputs are exactly the planned payments, so the fee is exactly ZIP 317's;
//! - the transaction targets the next block's consensus branch and expires within 50 blocks;
//! - no other spend of the escrow is in flight: it never signs a close and a liquidation of
//!   the same notes.
//!
//! Signing is two FROST rounds per spend. Round 1 runs only after the checks pass, with fresh
//! nonces for that PCZT; round 2 builds the signing package itself from the stored sighash,
//! so the coordinator cannot swap the message, and each nonce is used once. The randomizer is
//! the PCZT's own `alpha` (frost#1094: fine, provided the PCZT stays among the parties). With
//! its shares the signer returns an attestation (price, round, debt, payments, sighash)
//! signed by its attested Base key; when the spend is mined its hash goes to LoanBook.

use std::collections::BTreeMap;

use alloy::primitives::{Address, B256, keccak256};
use alloy::signers::SignerSync;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use lentic_base::LoanView;
use lentic_core::math::{self, Owed};
use lentic_core::{Health, Role};
use lentic_frost::identifier;
use lentic_frost::sign::{self, SignatureShare, SigningCommitments, SigningNonces};
use lentic_zcash::escrow;
use lentic_zcash::fee::zip317_fee;
use lentic_zcash::keys;
use lentic_zcash::scan::SpentIn;
use rand_core_06::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{Mutex, RwLock};
use zcash_protocol::consensus::{BlockHeight, BranchId};

use crate::api::{ApiError, ApiResult, Shared, bad_request};
use crate::book::{self, HeldLoan};

/// A co-signed transaction must expire within this many blocks of the signer's tip.
const MAX_EXPIRY_DELTA: u32 = 50;
/// How long a liquidation plan's price stays good for co-signing.
const PLAN_TTL_SECS: u64 = 300;
/// How long round-1 nonces wait for round 2.
const SESSION_TTL_SECS: u64 = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Lender + signer, at flood stage or in default, with the split the signer computes.
    Liquidation,
    /// Borrower + signer, once the debt is repaid on Base.
    Close,
    /// Borrower + signer, when the loan was never funded and the funding deadline passed.
    Refund,
}

impl Kind {
    /// Who coordinates the spend with the signer.
    pub fn coordinator(self) -> Role {
        match self {
            Kind::Liquidation => Role::Lender,
            Kind::Close | Kind::Refund => Role::Borrower,
        }
    }

    /// What LoanBook records once the spend is mined (a refunded loan was never opened).
    pub fn final_health(self) -> Option<Health> {
        match self {
            Kind::Liquidation => Some(Health::Liquidated),
            Kind::Close => Some(Health::Closed),
            Kind::Refund => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlannedPayment {
    /// The unified address from the terms.
    pub address: String,
    pub zat: u64,
}

/// What the signer will co-sign for a loan right now, and why (or why not).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub loan_id: B256,
    pub kind: Kind,
    pub allowed: bool,
    pub reason: String,
    pub coordinator: Role,
    /// The signer's view of the Zcash tip.
    pub tip: u32,
    /// Oracle round (unix time of the price reading) the plan is priced at.
    pub round: u64,
    pub price_usd: Option<f64>,
    pub owed: Owed,
    pub collateral_zat: u64,
    pub notes: usize,
    pub fee_zat: u64,
    pub payments: Vec<PlannedPayment>,
    pub computed_at: u64,
}

/// The attested statement returned with the signer's shares.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attestation {
    /// The statement as signed (compact JSON).
    pub statement: String,
    /// EIP-191 signature over `statement` by the signer's attested Base key.
    #[serde(with = "hex::serde")]
    pub signature: Vec<u8>,
    pub signer: Address,
    /// keccak256(statement): what LoanBook stores as the attestation hash.
    pub hash: B256,
}

/// A spend the signer has contributed to, kept until it is mined (and after, as a record).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Intent {
    pub loan_id: B256,
    pub kind: Kind,
    pub sighash: String,
    pub expiry_height: u32,
    /// Nullifiers of the escrow notes it spends (hex).
    pub nullifiers: Vec<String>,
    pub attestation: Attestation,
    pub signed_at: u64,
    #[serde(default)]
    pub mined: Option<SpentIn>,
    /// The LoanBook transaction that recorded the outcome.
    #[serde(default)]
    pub reported: Option<B256>,
}

struct SpendRound {
    index: usize,
    alpha: [u8; 32],
    nonces: SigningNonces,
    commitments: SigningCommitments,
}

struct Session {
    plan: Plan,
    sighash: [u8; 32],
    expiry: u32,
    nullifiers: Vec<[u8; 32]>,
    session_key: String,
    spends: Vec<SpendRound>,
    created: u64,
}

#[derive(Default)]
pub struct Cosign {
    sessions: Mutex<BTreeMap<String, Session>>,
    intents: RwLock<BTreeMap<B256, Intent>>,
    /// Recent liquidation plans, so a price tick between plan and co-sign does not void a PCZT.
    recent: Mutex<Vec<Plan>>,
}

impl Cosign {
    pub fn with_intents(intents: Vec<Intent>) -> Self {
        Cosign { intents: RwLock::new(intents.into_iter().map(|i| (i.loan_id, i)).collect()), ..Default::default() }
    }

    pub async fn intent(&self, loan_id: &B256) -> Option<Intent> {
        self.intents.read().await.get(loan_id).cloned()
    }

    /// Once the escrow notes of a co-signed spend are seen spent, records it and reports the
    /// outcome (Liquidated or Closed, with the attestation hash) to LoanBook.
    pub async fn settle(
        &self,
        s: &crate::api::AppState,
        loan_id: B256,
        loan: Option<&LoanView>,
        round: u64,
    ) -> anyhow::Result<()> {
        let Some(mut intent) = self.intent(&loan_id).await else { return Ok(()) };
        if intent.reported.is_some() || (intent.mined.is_some() && intent.kind.final_health().is_none()) {
            return Ok(());
        }
        if intent.mined.is_none() {
            let watchers = s.book.watchers.lock().await;
            let Some(w) = watchers.get(&loan_id) else { return Ok(()) };
            let spent: Option<Vec<SpentIn>> = intent
                .nullifiers
                .iter()
                .map(|nf| {
                    let bytes: [u8; 32] = hex::decode(nf).ok()?.try_into().ok()?;
                    let nf = Option::from(orchard::note::Nullifier::from_bytes(&bytes))?;
                    w.note_by_nullifier(&nf)?.spent.clone()
                })
                .collect();
            match spent {
                // Every note went in one transaction: the co-signed spend (only its signers
                // could have spent all of them together without the signer).
                Some(all) if !all.is_empty() && all.windows(2).all(|p| p[0].txid == p[1].txid) => {
                    tracing::info!(loan = %loan_id, txid = %all[0].txid, "co-signed spend mined");
                    intent.mined = Some(all[0].clone());
                }
                _ => return Ok(()),
            }
        }
        if let (Some(health), Some(view)) = (intent.kind.final_health(), loan)
            && s.report_health
            && !view.health.is_terminal()
            && round >= view.oracle_round
        {
            let tx = lentic_base::update_health(
                &s.provider,
                s.deployment.loan_book,
                loan_id,
                health,
                round,
                intent.attestation.hash,
            )
            .await?;
            tracing::info!(loan = %loan_id, ?health, %tx, "outcome recorded in LoanBook");
            intent.reported = Some(tx);
        }
        s.store.put("intents", &hex::encode(loan_id), &intent)?;
        self.intents.write().await.insert(loan_id, intent);
        Ok(())
    }
}

fn conflict(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::CONFLICT, msg.into())
}

async fn held(s: &Shared, loan_id: &B256) -> Result<HeldLoan, ApiError> {
    s.book
        .held(loan_id)
        .await
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("this policy signer holds no loan {loan_id}")))
}

async fn owed(s: &Shared, held: &HeldLoan, loan: &LoanView, now: u64) -> anyhow::Result<Owed> {
    let repayments = lentic_base::repayments(&s.provider, loan.vault, held.loan_id, s.deployment.start_block).await?;
    Ok(math::owed_at(held.terms.principal, held.terms.apr_bps, loan.opened_at, &repayments, now))
}

/// The plan, plus the escrow notes it was made from (nullifier, value).
pub async fn make_plan(s: &Shared, loan_id: B256, kind: Kind) -> Result<(Plan, Vec<([u8; 32], u64)>), ApiError> {
    let held = held(s, &loan_id).await?;
    let (tip, notes) = s.book.escrow_notes(s, &held).await?;
    let loan = lentic_base::read_loan(&s.provider, s.deployment.loan_book, loan_id).await?;
    let now = book::now();
    let terms = &held.terms;
    let collateral: u64 = notes.iter().map(|n| n.1).sum();
    let mut plan = Plan {
        loan_id,
        kind,
        allowed: false,
        reason: String::new(),
        coordinator: kind.coordinator(),
        tip,
        round: 0,
        price_usd: None,
        owed: Owed::default(),
        collateral_zat: collateral,
        notes: notes.len(),
        fee_zat: 0,
        payments: vec![],
        computed_at: now,
    };
    let to_borrower =
        |fee: u64| vec![PlannedPayment { address: terms.borrower_return_address.clone(), zat: collateral - fee }];

    if notes.is_empty() {
        plan.reason = "the escrow holds no confirmed note".into();
        return Ok((plan, notes));
    }
    match (kind, &loan) {
        (Kind::Refund, Some(_)) => plan.reason = "the loan was funded; a refund is only for a loan never funded".into(),
        (Kind::Refund, None) if now <= terms.funding_deadline => {
            plan.reason = format!("the funding deadline ({}) has not passed", terms.funding_deadline)
        }
        (Kind::Refund, None) => {
            plan.fee_zat = zip317_fee(notes.len(), 1);
            plan.payments = to_borrower(plan.fee_zat);
            plan.allowed = true;
            plan.reason = "never funded, and the funding deadline has passed".into();
        }
        (_, None) => plan.reason = "the loan is not open on LoanBook".into(),
        (_, Some(view)) if view.health.is_terminal() => plan.reason = format!("the loan is already {:?}", view.health),
        (Kind::Close, Some(view)) => {
            plan.owed = owed(s, &held, view, now).await?;
            if plan.owed.total() > 0 {
                plan.reason = format!("{} micro-USDC is still owed", plan.owed.total());
            } else {
                plan.fee_zat = zip317_fee(notes.len(), 1);
                plan.payments = to_borrower(plan.fee_zat);
                plan.allowed = true;
                plan.reason = "repaid in full on Base".into();
            }
        }
        (Kind::Liquidation, Some(view)) => {
            plan.owed = owed(s, &held, view, now).await?;
            let Some(price) = s.book.latest_price().await else {
                plan.reason = "no price reading yet".into();
                return Ok((plan, notes));
            };
            plan.round = price.at;
            plan.price_usd = Some(price.usd);
            let debt = plan.owed.total();
            let health = math::assess_at(terms, debt, collateral, price.price, now);
            if health != Health::Liquidating {
                let ltv = math::ltv_bps(debt, collateral, price.price).map(|b| b as f64 / 100.0);
                plan.reason =
                    format!("not liquidatable: loan-to-value {ltv:?} % at {} USD, maturity not reached", price.usd);
            } else {
                plan.fee_zat = zip317_fee(notes.len(), 2);
                let split = math::liquidation_split(
                    collateral,
                    plan.fee_zat,
                    debt,
                    price.price,
                    terms.haircut_bps,
                    terms.penalty_bps,
                )
                .map_err(|e| bad_request(e.to_string()))?;
                plan.payments
                    .push(PlannedPayment { address: terms.lender_zcash_address.clone(), zat: split.lender_zat });
                if split.borrower_zat > 0 {
                    plan.payments.push(PlannedPayment {
                        address: terms.borrower_return_address.clone(),
                        zat: split.borrower_zat,
                    });
                }
                plan.allowed = true;
                plan.reason = if now >= terms.maturity && debt > 0 {
                    "in default: maturity passed with debt outstanding".into()
                } else {
                    "at flood stage".into()
                };
            }
        }
    }
    Ok((plan, notes))
}

struct Checked {
    sighash: [u8; 32],
    expiry: u32,
    spends: Vec<(usize, [u8; 32])>,
    nullifiers: Vec<[u8; 32]>,
}

/// The PCZT must spend exactly the escrow's notes and pay exactly the plan.
fn check(
    pczt: &lentic_zcash::Pczt,
    plan: &Plan,
    notes: &[([u8; 32], u64)],
    params: &lentic_zcash::network::Params,
) -> Result<Checked, String> {
    let e = |x: lentic_zcash::Error| x.to_string();
    let pending = escrow::spends_to_sign(pczt).map_err(e)?;
    let nullifiers: BTreeMap<usize, [u8; 32]> =
        escrow::spend_nullifiers(pczt).map_err(e)?.into_iter().map(|(i, nf)| (i, nf.to_bytes())).collect();
    let unspent: BTreeMap<[u8; 32], u64> = notes.iter().copied().collect();

    let mut spends = Vec::new();
    for p in &pending {
        let nf = nullifiers.get(&p.index).ok_or("a pending spend has no nullifier")?;
        let value = unspent.get(nf).ok_or(format!("spend {} is not an unspent note of this escrow", p.index))?;
        if p.value != Some(*value) {
            return Err(format!("spend {} states {:?} zat, the note holds {value}", p.index, p.value));
        }
        spends.push((p.index, p.alpha));
    }
    if pending.len() != unspent.len() {
        return Err(format!(
            "the spend takes {} of the escrow's {} notes; it must take all",
            pending.len(),
            unspent.len()
        ));
    }

    let expected = plan
        .payments
        .iter()
        .map(|p| keys::decode_orchard_receiver(params, &p.address).map(|a| (a, p.zat)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(e)?;
    escrow::check_payments(pczt, &expected).map_err(e)?;

    let expiry = escrow::expiry_height(pczt);
    if expiry <= plan.tip || expiry > plan.tip + MAX_EXPIRY_DELTA {
        return Err(format!("expiry height {expiry} is outside ({}, {}]", plan.tip, plan.tip + MAX_EXPIRY_DELTA));
    }
    let branch = u32::from(BranchId::for_height(params, BlockHeight::from_u32(plan.tip + 1)));
    if escrow::consensus_branch_id(pczt) != branch {
        return Err(format!(
            "the PCZT targets branch {:08x}, the next block is on {branch:08x}",
            escrow::consensus_branch_id(pczt)
        ));
    }
    let sighash = escrow::shielded_sighash(pczt).map_err(e)?;
    let nullifiers = spends.iter().map(|(i, _)| nullifiers[i]).collect();
    Ok(Checked { sighash, expiry, spends, nullifiers })
}

/// `GET /v1/loans/{id}/plan/{kind}`
pub async fn plan(State(s): State<Shared>, Path((id, kind)): Path<(B256, Kind)>) -> ApiResult<Plan> {
    let (plan, _) = make_plan(&s, id, kind).await?;
    if plan.allowed && kind == Kind::Liquidation {
        let mut recent = s.cosign.recent.lock().await;
        recent.retain(|p| book::now() - p.computed_at < PLAN_TTL_SECS);
        recent.push(plan.clone());
    }
    Ok(Json(plan))
}

#[derive(Deserialize)]
pub struct CosignRequest {
    pub kind: Kind,
    /// The serialized PCZT (hex).
    #[serde(with = "hex::serde")]
    pub pczt: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedCommitments {
    pub index: usize,
    pub commitments: SigningCommitments,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CosignStarted {
    pub session: String,
    #[serde(with = "hex::serde")]
    pub sighash: [u8; 32],
    pub expiry_height: u32,
    pub plan: Plan,
    /// The signer's round-1 commitments, one per escrow spend.
    pub commitments: Vec<IndexedCommitments>,
}

/// `POST /v1/loans/{id}/cosign`: checks the PCZT and answers with round-1 commitments.
pub async fn start(
    State(s): State<Shared>,
    Path(id): Path<B256>,
    Json(req): Json<CosignRequest>,
) -> ApiResult<CosignStarted> {
    let held = held(&s, &id).await?;
    let session_key = held.dkg_session.clone().ok_or_else(|| bad_request("no escrow key held for this loan"))?;
    let share = s.dkg.share(&session_key).await.ok_or_else(|| bad_request("the escrow key is missing"))?;
    let (fresh, notes) = make_plan(&s, id, req.kind).await?;
    if !fresh.allowed {
        return Err(conflict(fresh.reason));
    }
    let pczt = escrow::parse(&req.pczt).map_err(|e| bad_request(e.to_string()))?;
    let params = s.zcash_network.params();

    let mut candidates = vec![fresh.clone()];
    if req.kind == Kind::Liquidation {
        let recent = s.cosign.recent.lock().await;
        candidates.extend(
            recent
                .iter()
                .rev()
                .filter(|p| p.loan_id == id && book::now() - p.computed_at < PLAN_TTL_SECS && p.notes == fresh.notes)
                .map(|p| Plan { tip: fresh.tip, ..p.clone() }),
        );
    }
    let mut first_error = None;
    let mut matched = None;
    for plan in candidates {
        match check(&pczt, &plan, &notes, &params) {
            Ok(c) => {
                matched = Some((plan, c));
                break;
            }
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    let (plan, checked) = matched.ok_or_else(|| conflict(first_error.unwrap_or_default()))?;

    if let Some(intent) = s.cosign.intent(&id).await
        && intent.mined.is_none()
        && intent.expiry_height > plan.tip
        && intent.sighash != hex::encode(checked.sighash)
    {
        return Err(conflict(format!(
            "another spend of this escrow ({:?}) is in flight until height {}",
            intent.kind, intent.expiry_height
        )));
    }

    let mut spends = Vec::new();
    for (index, alpha) in &checked.spends {
        let (nonces, commitments) = sign::commit(&share.key_package, &mut OsRng);
        spends.push(SpendRound { index: *index, alpha: *alpha, nonces, commitments });
    }
    let commitments =
        spends.iter().map(|r| IndexedCommitments { index: r.index, commitments: r.commitments }).collect();
    let mut sid = [0u8; 16];
    OsRng.fill_bytes(&mut sid);
    let session = hex::encode(sid);
    let started = CosignStarted {
        session: session.clone(),
        sighash: checked.sighash,
        expiry_height: checked.expiry,
        plan: plan.clone(),
        commitments,
    };
    s.cosign.sessions.lock().await.insert(
        session,
        Session {
            plan,
            sighash: checked.sighash,
            expiry: checked.expiry,
            nullifiers: checked.nullifiers,
            session_key,
            spends,
            created: book::now(),
        },
    );
    Ok(Json(started))
}

#[derive(Deserialize)]
pub struct SharesRequest {
    /// The coordinator's round-1 commitments, one per escrow spend.
    pub commitments: Vec<IndexedCommitments>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedShare {
    pub index: usize,
    pub share: SignatureShare,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shares {
    pub shares: Vec<IndexedShare>,
    pub attestation: Attestation,
}

/// `POST /v1/cosign/{session}/shares`: round 2.
pub async fn shares(
    State(s): State<Shared>,
    Path(session): Path<String>,
    Json(req): Json<SharesRequest>,
) -> ApiResult<Shares> {
    let session = s.cosign.sessions.lock().await.remove(&session).ok_or_else(|| {
        ApiError(StatusCode::NOT_FOUND, "no such co-signing session (expired or already used)".into())
    })?;
    if book::now() - session.created > SESSION_TTL_SECS {
        return Err(ApiError(StatusCode::GONE, "the co-signing session expired; start again".into()));
    }
    let share = s.dkg.share(&session.session_key).await.ok_or_else(|| bad_request("the escrow key is missing"))?;
    let theirs: BTreeMap<usize, SigningCommitments> =
        req.commitments.into_iter().map(|c| (c.index, c.commitments)).collect();
    if theirs.len() != session.spends.len() || session.spends.iter().any(|r| !theirs.contains_key(&r.index)) {
        return Err(bad_request("send one commitment per escrow spend"));
    }
    let coordinator = identifier(session.plan.coordinator);
    let mut out = Vec::new();
    for round in &session.spends {
        let package = sign::signing_package(
            BTreeMap::from([(identifier(Role::Policy), round.commitments), (coordinator, theirs[&round.index])]),
            &session.sighash,
        );
        let sig = sign::sign_share(&package, &round.nonces, &share.key_package, &round.alpha)
            .map_err(|e| bad_request(e.to_string()))?;
        out.push(IndexedShare { index: round.index, share: sig });
    }

    let attestation = attest(&s, &session)?;
    let intent = Intent {
        loan_id: session.plan.loan_id,
        kind: session.plan.kind,
        sighash: hex::encode(session.sighash),
        expiry_height: session.expiry,
        nullifiers: session.nullifiers.iter().map(hex::encode).collect(),
        attestation: attestation.clone(),
        signed_at: book::now(),
        mined: None,
        reported: None,
    };
    s.store.put("intents", &hex::encode(intent.loan_id), &intent)?;
    s.cosign.intents.write().await.insert(intent.loan_id, intent);
    tracing::info!(loan = %session.plan.loan_id, kind = ?session.plan.kind, sighash = %hex::encode(session.sighash), "co-signed");
    Ok(Json(Shares { shares: out, attestation }))
}

fn attest(s: &Shared, session: &Session) -> anyhow::Result<Attestation> {
    let p = &session.plan;
    let statement = json!({
        "protocol": "lentic-attestation-v1",
        "chainId": s.deployment.chain_id,
        "loanId": p.loan_id,
        "kind": p.kind,
        "sighash": hex::encode(session.sighash),
        "expiryHeight": session.expiry,
        "round": p.round,
        "priceUsd": p.price_usd,
        "owed": p.owed,
        "collateralZat": p.collateral_zat,
        "feeZat": p.fee_zat,
        "payments": p.payments,
        "nullifiers": session.nullifiers.iter().map(hex::encode).collect::<Vec<_>>(),
        "signedAt": book::now(),
    })
    .to_string();
    let signature = s.signer.sign_message_sync(statement.as_bytes())?.as_bytes().to_vec();
    Ok(Attestation { hash: keccak256(statement.as_bytes()), statement, signature, signer: s.signer_address })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lentic_frost::{dkg, fvk as frost_fvk, group_ak};
    use lentic_zcash::escrow::{EscrowInput, Payment};
    use lentic_zcash::network::{Params, regtest};
    use lentic_zcash::sim::SimChain;
    use lentic_zcash::wallet::TestWallet;

    const ZEC: u64 = 100_000_000;
    const TARGET: u32 = 1_000;

    struct Fixture {
        fvk: orchard::keys::FullViewingKey,
        chain: SimChain,
        notes: Vec<(orchard::Note, incrementalmerkletree::Position)>,
        lender: orchard::Address,
        borrower: orchard::Address,
    }

    fn fixture(deposits: &[u64]) -> Fixture {
        let group = dkg::run_local(&mut OsRng).unwrap();
        let fvk = frost_fvk::derive_fvk(group_ak(&group.public).unwrap(), &frost_fvk::fvk_seed(&mut OsRng)).unwrap();
        let mut chain = SimChain::new();
        let notes = deposits.iter().map(|v| chain.deposit(&fvk, *v).unwrap()).collect();
        Fixture {
            fvk,
            chain,
            notes,
            lender: TestWallet::generate().address(),
            borrower: TestWallet::generate().address(),
        }
    }

    fn params() -> Params {
        Params::Regtest(regtest())
    }

    fn ua(a: orchard::Address) -> String {
        keys::encode_unified_address(&params(), a)
    }

    fn plan(f: &Fixture, payments: &[(orchard::Address, u64)]) -> Plan {
        Plan {
            loan_id: B256::ZERO,
            kind: Kind::Liquidation,
            allowed: true,
            reason: String::new(),
            coordinator: Role::Lender,
            tip: TARGET - 1,
            round: 0,
            price_usd: None,
            owed: Owed::default(),
            collateral_zat: f.notes.iter().map(|n| n.0.value().inner()).sum(),
            notes: f.notes.len(),
            fee_zat: 0,
            payments: payments.iter().map(|(a, z)| PlannedPayment { address: ua(*a), zat: *z }).collect(),
            computed_at: 0,
        }
    }

    fn escrow_notes(f: &Fixture) -> Vec<([u8; 32], u64)> {
        f.notes.iter().map(|(n, _)| (n.nullifier(&f.fvk).to_bytes(), n.value().inner())).collect()
    }

    fn build(
        f: &mut Fixture,
        spend: &[usize],
        payments: &[(orchard::Address, u64)],
        target: u32,
    ) -> lentic_zcash::Pczt {
        let anchor = f.chain.checkpoint();
        let inputs = spend
            .iter()
            .map(|&i| EscrowInput { note: f.notes[i].0, merkle_path: f.chain.witness(f.notes[i].1) })
            .collect();
        let payments: Vec<Payment> = payments.iter().map(|(a, z)| Payment::new(*a, *z)).collect();
        escrow::build_escrow_spend(regtest(), BlockHeight::from_u32(target), &f.fvk, anchor, inputs, &payments)
            .unwrap()
            .pczt
    }

    #[test]
    fn accepts_exactly_the_planned_spend() {
        let mut f = fixture(&[15 * ZEC, 5 * ZEC]);
        let fee = zip317_fee(2, 2);
        let pays = [(f.lender, 12 * ZEC), (f.borrower, 8 * ZEC - fee)];
        let pczt = build(&mut f, &[0, 1], &pays, TARGET);
        let checked = check(&pczt, &plan(&f, &pays), &escrow_notes(&f), &params()).unwrap();
        assert_eq!(checked.spends.len(), 2);
        assert_eq!(checked.sighash, escrow::shielded_sighash(&pczt).unwrap());
        assert_eq!(checked.expiry, TARGET + 40);
    }

    #[test]
    fn refuses_other_payments() {
        let mut f = fixture(&[20 * ZEC]);
        let fee = zip317_fee(1, 2);
        let planned = [(f.lender, 12 * ZEC), (f.borrower, 8 * ZEC - fee)];
        // The coordinator pays itself more than the plan.
        let greedy = [(f.lender, 13 * ZEC), (f.borrower, 7 * ZEC - fee)];
        let pczt = build(&mut f, &[0], &greedy, TARGET);
        let err = check(&pczt, &plan(&f, &planned), &escrow_notes(&f), &params()).err().unwrap();
        assert!(err.contains("PCZT pays"), "{err}");
        // Or sends the surplus somewhere else.
        let elsewhere = [(f.lender, 12 * ZEC), (TestWallet::generate().address(), 8 * ZEC - fee)];
        let pczt = build(&mut f, &[0], &elsewhere, TARGET);
        assert!(check(&pczt, &plan(&f, &planned), &escrow_notes(&f), &params()).is_err());
    }

    #[test]
    fn refuses_to_leave_notes_behind_or_spend_foreign_ones() {
        let mut f = fixture(&[15 * ZEC, 5 * ZEC]);
        // Only the first note: the top-up would stay in the escrow.
        let fee = zip317_fee(1, 1);
        let pays = [(f.lender, 15 * ZEC - fee)];
        let pczt = build(&mut f, &[0], &pays, TARGET);
        let err = check(&pczt, &plan(&f, &pays), &escrow_notes(&f), &params()).err().unwrap();
        assert!(err.contains("must take all"), "{err}");
        // A note the signer does not see as an unspent escrow note (e.g. already spent).
        let only_second = vec![escrow_notes(&f)[1]];
        let err = check(&pczt, &plan(&f, &pays), &only_second, &params()).err().unwrap();
        assert!(err.contains("not an unspent note"), "{err}");
    }

    #[test]
    fn refuses_long_lived_transactions() {
        let mut f = fixture(&[20 * ZEC]);
        let fee = zip317_fee(1, 1);
        let pays = [(f.borrower, 20 * ZEC - fee)];
        // Built for a block far ahead: it would stay valid long after the signer's view.
        let pczt = build(&mut f, &[0], &pays, TARGET + 100);
        let err = check(&pczt, &plan(&f, &pays), &escrow_notes(&f), &params()).err().unwrap();
        assert!(err.contains("expiry height"), "{err}");
    }
}
