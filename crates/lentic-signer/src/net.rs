//! The networked side of a party: key generation through the policy signer's relay, terms
//! approval and registration, and co-signing an escrow spend with the policy signer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use alloy_primitives::{Address, B256};
use anyhow::{Context, Result, bail, ensure};
use k256::ecdsa::SigningKey;
use lentic_core::terms::approval_message;
use lentic_core::{LoanTerms, Role, terms_hash};
use lentic_frost::identifier;
use lentic_frost::relay::{self, Participant, SessionView};
use lentic_frost::sign::{self, SignatureShare, SigningCommitments};
use lentic_zcash::escrow::{self, Payment};
use lentic_zcash::network::ZcashNetwork;
use lentic_zcash::scan::Watcher;
use lentic_zcash::{keys, lightwalletd};
use rand_core_06::OsRng;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zcash_protocol::consensus::BlockHeight;

use crate::ShareBundle;

/// The policy signer's HTTP API.
pub struct Policy {
    base: String,
    http: reqwest::Client,
}

impl Policy {
    pub fn new(url: &str) -> Self {
        Policy { base: url.trim_end_matches('/').to_owned(), http: reqwest::Client::new() }
    }

    pub fn url(&self) -> &str {
        &self.base
    }

    async fn read<T: DeserializeOwned>(&self, path: &str, res: reqwest::Response) -> Result<T> {
        let status = res.status();
        let body: Value = res.json().await.with_context(|| format!("{path}: not JSON"))?;
        if !status.is_success() {
            bail!("{path}: {} ({status})", body["error"].as_str().unwrap_or("error"));
        }
        serde_json::from_value(body).with_context(|| format!("{path}: unexpected answer"))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let res = self.http.get(format!("{}{path}", self.base)).send().await.with_context(|| self.base.clone())?;
        self.read(path, res).await
    }

    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T> {
        let res = self
            .http
            .post(format!("{}{path}", self.base))
            .json(body)
            .send()
            .await
            .with_context(|| self.base.clone())?;
        self.read(path, res).await
    }
}

/// A Base key in a file (hex). Test keys only: it is stored in the clear.
pub fn load_base_key(path: &Path) -> Result<SigningKey> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let bytes = hex::decode(raw.trim().trim_start_matches("0x")).context("a Base key file holds 32-byte hex")?;
    SigningKey::from_slice(&bytes).context("not a secp256k1 key")
}

pub fn new_base_key(out: &Path) -> Result<()> {
    if out.exists() {
        bail!("{} exists; not overwriting a key", out.display());
    }
    let key = SigningKey::random(&mut OsRng);
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, format!("0x{}\n", hex::encode(key.to_bytes())))?;
    println!("{}", json!({ "address": relay::base_address(&key), "key": out }));
    Ok(())
}

async fn wait_for(
    policy: &Policy,
    session: &str,
    timeout: Duration,
    ready: impl Fn(&SessionView) -> bool,
) -> Result<SessionView> {
    let start = Instant::now();
    loop {
        let view: SessionView = policy.get(&format!("/v1/dkg/{session}")).await?;
        if ready(&view) {
            return Ok(view);
        }
        ensure!(start.elapsed() < timeout, "timed out waiting for the other parties in DKG session {session}");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

pub async fn dkg_create(policy: &Policy, borrower: Address, lender: Address) -> Result<()> {
    let view: SessionView = policy.post("/v1/dkg", &json!({ "borrower": borrower, "lender": lender })).await?;
    println!("{}", json!({ "session": view.session, "identities": view.identities }));
    Ok(())
}

/// Runs this party's side of the DKG and writes its share bundle.
pub async fn dkg_join(
    policy: &Policy,
    session: &str,
    role: Role,
    key: &SigningKey,
    network: ZcashNetwork,
    out: &Path,
    timeout: Duration,
) -> Result<()> {
    ensure!(role != Role::Policy, "the policy signer joins on its own");
    if out.exists() {
        bail!("{} exists; not overwriting a share", out.display());
    }
    let me = relay::base_address(key);
    let view: SessionView = policy.get(&format!("/v1/dkg/{session}")).await?;
    let expected = view.identities.get(&role).copied().context("the session names no such role")?;
    ensure!(expected == me, "the session expects {role} to be {expected}; this key is {me}");

    let mut participant = Participant::new(role, session, &mut OsRng);
    let post = participant.round1(&mut OsRng, |m| relay::eip191_sign(key, m))?;
    let _: SessionView = policy.post(&format!("/v1/dkg/{session}/round1"), &post).await?;
    eprintln!("round 1 posted; waiting for the other parties");
    let view = wait_for(policy, session, timeout, SessionView::has_round1).await?;
    let envelopes = participant.round2(&view, &mut OsRng)?;
    let _: SessionView = policy.post(&format!("/v1/dkg/{session}/round2"), &json!({ "envelopes": envelopes })).await?;
    eprintln!("round 2 sealed and posted; waiting for the envelopes to {role}");
    let view = wait_for(policy, session, timeout, |v| v.inbox(role).len() == 2 && v.birthday.is_some()).await?;
    let result = participant.round3(&view)?;
    let _: SessionView = policy.post(&format!("/v1/dkg/{session}/done"), &result.done).await?;

    let bundle = ShareBundle {
        role,
        key_package: result.key_package,
        public_key_package: result.public_key_package,
        fvk_seed: result.fvk_seed,
        session: Some(session.to_owned()),
        birthday: view.birthday,
        policy: Some(policy.url().to_owned()),
        policy_signer: view.identities.get(&Role::Policy).copied(),
        network: Some(network),
    };
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, serde_json::to_string_pretty(&bundle)?)?;
    eprintln!("share written to {}", out.display());
    crate::show(&bundle, network)
}

pub fn terms_approve(terms: &Path, salt: B256, key: &SigningKey) -> Result<()> {
    let terms: LoanTerms = serde_json::from_str(&std::fs::read_to_string(terms)?)?;
    terms.validate().map_err(anyhow::Error::msg)?;
    let hash = terms_hash(&terms, salt);
    let loan_id = lentic_core::loan_id(terms.lender_vault, hash);
    let signature = relay::eip191_sign(key, &approval_message(loan_id, hash))?;
    println!(
        "{}",
        json!({ "loanId": loan_id, "termsHash": hash, "signer": relay::base_address(key), "signature": hex::encode(signature) })
    );
    Ok(())
}

pub async fn terms_register(
    policy: &Policy,
    terms: &Path,
    salt: B256,
    session: &str,
    borrower_signature: &str,
    lender_signature: &str,
) -> Result<()> {
    let terms: LoanTerms = serde_json::from_str(&std::fs::read_to_string(terms)?)?;
    let loan_id = lentic_core::loan_id(terms.lender_vault, terms_hash(&terms, salt));
    let registered: Value = policy
        .post(
            "/v1/loans",
            &json!({
                "loanId": loan_id,
                "salt": salt,
                "terms": terms,
                "dkgSession": session,
                "borrowerSignature": borrower_signature.trim_start_matches("0x"),
                "lenderSignature": lender_signature.trim_start_matches("0x"),
            }),
        )
        .await?;
    println!("{}", serde_json::to_string_pretty(&registered)?);
    Ok(())
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Kind {
    Liquidation,
    Close,
    Refund,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Liquidation => "liquidation",
            Kind::Close => "close",
            Kind::Refund => "refund",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexedCommitments {
    index: usize,
    commitments: SigningCommitments,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexedShare {
    index: usize,
    share: SignatureShare,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Started {
    session: String,
    sighash: String,
    commitments: Vec<IndexedCommitments>,
}

#[derive(Deserialize)]
struct Shares {
    shares: Vec<IndexedShare>,
    attestation: Value,
}

/// Builds the escrow spend the policy signer plans, co-signs it with the signer (two FROST
/// rounds), refreshes the anchor, proves and broadcasts.
pub async fn cosign(
    policy: &Policy,
    loan_id: B256,
    kind: Kind,
    share: &Path,
    server: &str,
    out_dir: &Path,
    no_broadcast: bool,
) -> Result<()> {
    let bundle = ShareBundle::load(share)?;
    let network = bundle.network.unwrap_or(ZcashNetwork::Testnet);
    let params = network.params();
    let birthday = bundle.birthday.context("the share bundle has no escrow birthday (made with dkg-local?)")?;

    let plan: Value = policy.get(&format!("/v1/loans/{loan_id}/plan/{}", kind.as_str())).await?;
    if plan["allowed"] != json!(true) {
        bail!("the policy signer will not co-sign a {}: {}", kind.as_str(), plan["reason"].as_str().unwrap_or("?"));
    }
    let coordinator: Role = serde_json::from_value(plan["coordinator"].clone())?;
    ensure!(
        coordinator == bundle.role,
        "a {} is coordinated by the {coordinator}, this share is the {}",
        kind.as_str(),
        bundle.role
    );
    eprintln!("plan: {}", plan["reason"].as_str().unwrap_or(""));

    let fvk = bundle.fvk()?;
    let mut watcher = Watcher::new(fvk.clone(), birthday);
    let mut client = lightwalletd::connect(server).await?;
    let tip = watcher.sync(&mut client).await?;
    let (anchor, inputs) = watcher.spendable(tip)?;
    let mut payments = Vec::new();
    for p in plan["payments"].as_array().context("plan without payments")? {
        let address = p["address"].as_str().context("payment address")?;
        let zat = p["zat"].as_u64().context("payment value")?;
        payments.push(Payment::new(keys::decode_orchard_receiver(&params, address)?, zat));
    }
    let spend = escrow::build_escrow_spend(params, BlockHeight::from_u32(tip + 1), &fvk, anchor, inputs, &payments)?;
    let sighash = escrow::shielded_sighash(&spend.pczt)?;
    let pending: BTreeMap<usize, [u8; 32]> =
        escrow::spends_to_sign(&spend.pczt)?.into_iter().map(|s| (s.index, s.alpha)).collect();
    std::fs::create_dir_all(out_dir)?;
    let file = |ext: &str| -> PathBuf { out_dir.join(format!("{}.{ext}", kind.as_str())) };
    std::fs::write(file("unsigned.pczt"), escrow::serialize(spend.pczt.clone())?)?;

    // Round 1: the signer checks the PCZT, then commits.
    let started: Started = policy
        .post(
            &format!("/v1/loans/{loan_id}/cosign"),
            &json!({ "kind": kind.as_str(), "pczt": hex::encode(escrow::serialize(spend.pczt.clone())?) }),
        )
        .await?;
    ensure!(started.sighash == hex::encode(sighash), "the signer saw a different sighash");
    let theirs: BTreeMap<usize, SigningCommitments> =
        started.commitments.into_iter().map(|c| (c.index, c.commitments)).collect();
    ensure!(theirs.keys().eq(pending.keys()), "the signer committed to other spends");
    let mut mine = BTreeMap::new();
    for index in pending.keys() {
        mine.insert(*index, sign::commit(&bundle.key_package, &mut OsRng));
    }

    // Round 2: both shares, then aggregation.
    let commitments: Vec<IndexedCommitments> =
        mine.iter().map(|(index, (_, c))| IndexedCommitments { index: *index, commitments: *c }).collect();
    let answer: Shares =
        policy.post(&format!("/v1/cosign/{}/shares", started.session), &json!({ "commitments": commitments })).await?;
    let statement = answer.attestation["statement"].as_str().context("attestation statement")?;
    let signature = hex::decode(answer.attestation["signature"].as_str().context("attestation signature")?)?;
    let attester = relay::recover_signer(statement, &signature)?;
    if let Some(expected) = bundle.policy_signer {
        ensure!(attester == expected, "the attestation is signed by {attester}, not the policy signer {expected}");
    }
    let stated: Value = serde_json::from_str(statement)?;
    ensure!(stated["sighash"] == json!(hex::encode(sighash)), "the attestation is for another transaction");
    std::fs::write(file("attestation.json"), serde_json::to_string_pretty(&answer.attestation)?)?;

    let their_shares: BTreeMap<usize, SignatureShare> = answer.shares.into_iter().map(|s| (s.index, s.share)).collect();
    let mut signatures = Vec::new();
    for (index, alpha) in &pending {
        let (nonces, my_commitments) = &mine[index];
        let package = sign::signing_package(
            BTreeMap::from([(identifier(Role::Policy), theirs[index]), (identifier(bundle.role), *my_commitments)]),
            &sighash,
        );
        let my_share = sign::sign_share(&package, nonces, &bundle.key_package, alpha)?;
        let shares = BTreeMap::from([
            (identifier(Role::Policy), *their_shares.get(index).context("the signer sent no share for a spend")?),
            (identifier(bundle.role), my_share),
        ]);
        signatures.push((*index, sign::aggregate(&package, &shares, &bundle.public_key_package, alpha)?));
    }
    let signed = escrow::apply_spend_auth_signatures(spend.pczt, &signatures)?;
    std::fs::write(file("signed.pczt"), escrow::serialize(signed.clone())?)?;

    // Fresh anchor and witnesses (signatures do not commit to them), proof, broadcast.
    let tip = watcher.sync(&mut client).await?;
    let mut witnesses = Vec::new();
    for (index, nf) in escrow::spend_nullifiers(&signed)? {
        if let Some(note) = watcher.note_by_nullifier(&nf) {
            witnesses.push((index, watcher.witness(note.position, tip)?));
        }
    }
    let refreshed = escrow::refresh_witnesses(signed, watcher.anchor(tip)?, witnesses)?;
    let tx = escrow::extract(escrow::prove(refreshed)?)?;
    std::fs::write(file("tx.hex"), hex::encode(&tx.raw))?;
    if !no_broadcast {
        lightwalletd::broadcast(&mut client, tx.raw.clone()).await?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "kind": kind.as_str(),
            "loanId": loan_id,
            "txid": tx.txid,
            "broadcast": !no_broadcast,
            "explorer": (network == ZcashNetwork::Testnet).then(|| format!("https://testnet.zecblock.com/tx/{}", tx.txid)),
            "payments": plan["payments"],
            "feeZat": plan["feeZat"],
            "attestationHash": answer.attestation["hash"],
            "attestedBy": attester,
            "files": out_dir,
        }))?
    );
    Ok(())
}
