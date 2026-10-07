//! PCZT pipeline for spending a loan escrow (close or liquidation).
//!
//! ```text
//! coordinator: build_escrow_spend ─▶ PCZT (outputs fixed, alpha fixed, sighash fixed)
//! each signer: payments + check_payments ─▶ spends_to_sign (alpha) ─▶ FROST round 1/2
//! coordinator: aggregate ─▶ apply_spend_auth_signatures
//! at broadcast: refresh_witnesses (fresh anchor, v6 sighash unaffected) ─▶ prove ─▶ extract
//! ```
//!
//! Signers never need the note commitment tree; only whoever proves does.

use std::sync::OnceLock;

use orchard::keys::{FullViewingKey, Scope};
use orchard::primitives::redpallas::{self, SpendAuth};
use orchard::{Anchor, Note, tree::MerklePath};
use pczt::roles::{
    creator::Creator, io_finalizer::IoFinalizer, prover::Prover, redactor::Redactor, signer::Signer,
    tx_extractor::TransactionExtractor, updater::Updater, verifier::Verifier,
};
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use zcash_primitives::transaction::builder::{BuildConfig, Builder, BundlePadding, PcztResult};
use zcash_primitives::transaction::fees::zip317;
use zcash_protocol::consensus::{BlockHeight, Parameters};
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;

use crate::{Error, Pczt, fee::zip317_fee, pczt_err};

/// A note held by the escrow, with a witness against `anchor`.
#[derive(Clone, Debug)]
pub struct EscrowInput {
    pub note: Note,
    pub merkle_path: MerklePath,
}

/// One payment out of the escrow.
#[derive(Clone, Debug)]
pub struct Payment {
    pub recipient: orchard::Address,
    pub value: u64,
    pub memo: MemoBytes,
}

impl Payment {
    pub fn new(recipient: orchard::Address, value: u64) -> Self {
        Payment { recipient, value, memo: MemoBytes::empty() }
    }
}

/// An unsigned escrow spend.
pub struct EscrowSpend {
    pub pczt: Pczt,
    /// Ironwood action indices of the escrow spends (one per input, in input order).
    pub spend_indices: Vec<usize>,
    pub fee: u64,
}

/// A spend that still needs a FROST signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpendToSign {
    pub index: usize,
    /// The FROST randomizer for this spend.
    pub alpha: [u8; 32],
    pub value: Option<u64>,
}

/// A real (non-zero) payment as it will appear on chain, with its note commitment checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputView {
    pub index: usize,
    pub recipient: orchard::Address,
    pub value: u64,
}

pub(crate) fn rng() -> UnwrapErr<SysRng> {
    UnwrapErr(SysRng)
}

/// Builds a v6 transaction spending escrow notes to `payments`. Inputs must exactly cover
/// the payments plus the ZIP 317 fee: escrow spends never produce change.
pub fn build_escrow_spend<P: Parameters>(
    params: P,
    target_height: BlockHeight,
    fvk: &FullViewingKey,
    anchor: Anchor,
    inputs: Vec<EscrowInput>,
    payments: &[Payment],
) -> Result<EscrowSpend, Error> {
    build_spend(params, target_height, fvk, anchor, inputs, payments, None)
}

/// Builds a v6 transaction spending `inputs` to `payments`. Whatever is left after the
/// ZIP 317 fee goes to `change` (a test wallet's own address); with no change address the
/// inputs must exactly cover the payments plus the fee.
pub fn build_spend<P: Parameters>(
    params: P,
    target_height: BlockHeight,
    fvk: &FullViewingKey,
    anchor: Anchor,
    inputs: Vec<EscrowInput>,
    payments: &[Payment],
    change: Option<orchard::Address>,
) -> Result<EscrowSpend, Error> {
    if inputs.is_empty() || payments.is_empty() {
        return Err(Error::Builder("a spend needs at least one input and one payment".into()));
    }
    let total_in: u64 = inputs.iter().map(|i| i.note.value().inner()).sum();
    let total_out: u64 = payments.iter().map(|p| p.value).sum();
    let exact_fee = zip317_fee(inputs.len(), payments.len());
    let mut payments = payments.to_vec();
    let fee = match change {
        _ if total_in == total_out + exact_fee => exact_fee,
        Some(change) => {
            let fee = zip317_fee(inputs.len(), payments.len() + 1);
            let left = total_in.checked_sub(total_out + fee).filter(|v| *v > 0).ok_or_else(|| {
                Error::Builder(format!("insufficient funds: inputs {total_in} < payments {total_out} + fee {fee}"))
            })?;
            payments.push(Payment::new(change, left));
            fee
        }
        None => {
            return Err(Error::Builder(format!(
                "unbalanced escrow spend: inputs {total_in} != payments {total_out} + fee {exact_fee}"
            )));
        }
    };
    let payments = &payments[..];

    let mut builder = Builder::new(
        params,
        target_height,
        BuildConfig::Standard {
            sapling_anchor: None,
            orchard_anchor: None,
            ironwood_anchor: Some(anchor),
            orchard_padding: BundlePadding::DEFAULT,
            ironwood_padding: BundlePadding::DEFAULT,
        },
    );
    let n_inputs = inputs.len();
    for input in inputs {
        builder
            .add_ironwood_spend::<zip317::FeeRule>(fvk.clone(), input.note, input.merkle_path)
            .map_err(|e| Error::Builder(format!("add spend: {e:?}")))?;
    }
    // Outgoing ciphertexts under the escrow OVK, so every party can audit what left.
    let ovk = fvk.to_ovk(Scope::External);
    for p in payments {
        let value = Zatoshis::from_u64(p.value).map_err(|e| Error::Builder(format!("{e:?}")))?;
        builder
            .add_ironwood_output::<zip317::FeeRule>(Some(ovk.clone()), p.recipient, value, p.memo.clone())
            .map_err(|e| Error::Builder(format!("add output: {e:?}")))?;
    }

    let PcztResult { pczt_parts, ironwood_meta, .. } = builder
        .build_for_pczt(rng(), &zip317::FeeRule::standard())
        .map_err(|e| Error::Builder(format!("build: {e:?}")))?;
    let created = Creator::build_from_parts(pczt_parts).ok_or_else(|| pczt_err("creator", "missing parts"))?;
    let pczt = IoFinalizer::new(created).finalize_io(rng()).map_err(|e| pczt_err("io_finalizer", e))?;

    let spend_indices = (0..n_inputs)
        .map(|i| ironwood_meta.spend_action_index(i).ok_or_else(|| pczt_err("creator", "spend index")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(EscrowSpend { pczt, spend_indices, fee })
}

/// The v6 shielded sighash all spend authorization signatures commit to.
pub fn shielded_sighash(pczt: &Pczt) -> Result<[u8; 32], Error> {
    Ok(Signer::new(pczt.clone()).map_err(|e| pczt_err("signer", e))?.shielded_sighash())
}

/// Ironwood spends still awaiting a spend authorization signature, with their randomizers.
pub fn spends_to_sign(pczt: &Pczt) -> Result<Vec<SpendToSign>, Error> {
    let mut found = Vec::new();
    Verifier::new(pczt.clone())
        .with_ironwood::<std::convert::Infallible, _>(|bundle| {
            for (index, action) in bundle.actions().iter().enumerate() {
                let spend = action.spend();
                if spend.spend_auth_sig().is_some() {
                    continue; // dummy spends are signed by the IO finalizer
                }
                if let Some(alpha) = spend.alpha() {
                    use ff::PrimeField;
                    found.push(SpendToSign { index, alpha: alpha.to_repr(), value: spend.value().map(|v| v.inner()) });
                }
            }
            Ok(())
        })
        .map_err(|e| pczt_err("verifier", e))?;
    Ok(found)
}

/// The nullifier of every Ironwood action's spend, by action index (dummy spends included;
/// callers match them against the notes they know).
pub fn spend_nullifiers(pczt: &Pczt) -> Result<Vec<(usize, orchard::note::Nullifier)>, Error> {
    let mut found = Vec::new();
    Verifier::new(pczt.clone())
        .with_ironwood::<std::convert::Infallible, _>(|bundle| {
            found.extend(bundle.actions().iter().enumerate().map(|(i, a)| (i, *a.spend().nullifier())));
            Ok(())
        })
        .map_err(|e| pczt_err("verifier", e))?;
    Ok(found)
}

/// The expiry height of the transaction a PCZT will become.
pub fn expiry_height(pczt: &Pczt) -> u32 {
    *pczt.global().expiry_height()
}

/// The consensus branch the PCZT's signatures commit to.
pub fn consensus_branch_id(pczt: &Pczt) -> u32 {
    *pczt.global().consensus_branch_id()
}

/// Non-zero Ironwood outputs, each checked against its note commitment so the plaintext
/// recipient and value a signer reads are the ones that will be on chain.
pub fn payments(pczt: &Pczt) -> Result<Vec<OutputView>, Error> {
    let mut out = Vec::new();
    let mut failure = None;
    Verifier::new(pczt.clone())
        .with_ironwood::<std::convert::Infallible, _>(|bundle| {
            for (index, action) in bundle.actions().iter().enumerate() {
                let output = action.output();
                let (Some(recipient), Some(value)) = (output.recipient(), output.value()) else {
                    failure = Some(format!("output {index} is redacted; cannot verify it"));
                    return Ok(());
                };
                if let Err(e) = output.verify_note_commitment(action.spend()) {
                    failure = Some(format!("output {index} note commitment: {e:?}"));
                    return Ok(());
                }
                if value.inner() > 0 {
                    out.push(OutputView { index, recipient: *recipient, value: value.inner() });
                }
            }
            Ok(())
        })
        .map_err(|e| pczt_err("verifier", e))?;
    match failure {
        Some(f) => Err(Error::OutputMismatch(f)),
        None => Ok(out),
    }
}

/// Checks that the PCZT pays exactly `expected` (as a multiset) and nothing else.
/// This is the check the policy signer runs before contributing its share.
pub fn check_payments(pczt: &Pczt, expected: &[(orchard::Address, u64)]) -> Result<(), Error> {
    let mut actual: Vec<(Vec<u8>, u64)> =
        payments(pczt)?.into_iter().map(|o| (o.recipient.to_raw_address_bytes().to_vec(), o.value)).collect();
    let mut want: Vec<(Vec<u8>, u64)> = expected.iter().map(|(a, v)| (a.to_raw_address_bytes().to_vec(), *v)).collect();
    actual.sort();
    want.sort();
    if actual != want {
        return Err(Error::OutputMismatch(format!(
            "PCZT pays {} output(s) totalling {}, expected {} totalling {}",
            actual.len(),
            actual.iter().map(|o| o.1).sum::<u64>(),
            want.len(),
            want.iter().map(|o| o.1).sum::<u64>()
        )));
    }
    Ok(())
}

/// Applies aggregated FROST signatures; each is verified against the action's `rk`.
pub fn apply_spend_auth_signatures(pczt: Pczt, signatures: &[(usize, [u8; 64])]) -> Result<Pczt, Error> {
    let mut signer = Signer::new(pczt).map_err(|e| pczt_err("signer", e))?;
    for (index, sig) in signatures {
        signer
            .apply_ironwood_signature(*index, redpallas::Signature::<SpendAuth>::from(*sig))
            .map_err(|e| pczt_err("signer", e))?;
    }
    Ok(signer.finish())
}

/// Strips spend witnesses and the anchor: what a signer needs is independent of both.
pub fn redact_witnesses(pczt: Pczt, spend_indices: &[usize]) -> Pczt {
    Redactor::new(pczt)
        .redact_ironwood_with(|mut r| {
            for &i in spend_indices {
                r.redact_action(i, |mut a| a.clear_spend_witness());
            }
            r.clear_anchor();
        })
        .finish()
}

/// Sets a fresh anchor and matching witnesses right before proving. In a v6 transaction the
/// spend authorization signatures do not commit to either, so this works after signing.
///
/// The Updater refuses to overwrite a different anchor, so the old anchor and witnesses are
/// redacted first.
pub fn refresh_witnesses(pczt: Pczt, anchor: Anchor, witnesses: Vec<(usize, MerklePath)>) -> Result<Pczt, Error> {
    let indices: Vec<usize> = witnesses.iter().map(|(i, _)| *i).collect();
    let pczt = redact_witnesses(pczt, &indices);
    let updated = Updater::new(pczt)
        .set_ironwood_anchor(anchor)
        .map_err(|e| pczt_err("updater", e))?
        .set_ironwood_spend_witnesses(witnesses)
        .map_err(|e| pczt_err("updater", e))?
        .finish();
    Ok(updated)
}

/// The post-NU6.3 Orchard-protocol proving key (built once; takes a few seconds).
pub fn proving_key() -> &'static orchard::circuit::ProvingKey {
    static PK: OnceLock<orchard::circuit::ProvingKey> = OnceLock::new();
    PK.get_or_init(|| orchard::circuit::ProvingKey::build(orchard::circuit::OrchardCircuitVersion::PostNu6_3))
}

/// Creates the Ironwood proof. Needs witnesses and the anchor; no spend key.
pub fn prove(pczt: Pczt) -> Result<Pczt, Error> {
    Ok(Prover::new(pczt).create_ironwood_proof(rng(), proving_key()).map_err(|e| pczt_err("prover", e))?.finish())
}

/// A transaction ready for `SendTransaction`.
#[derive(Clone, Debug)]
pub struct ExtractedTx {
    pub txid: String,
    pub raw: Vec<u8>,
    pub expiry_height: u32,
}

/// Extracts the final transaction (verifies signatures and the proof).
pub fn extract(pczt: Pczt) -> Result<ExtractedTx, Error> {
    let tx = TransactionExtractor::new(pczt).extract(rng()).map_err(|e| pczt_err("tx_extractor", e))?;
    let mut raw = Vec::new();
    tx.write(&mut raw).map_err(|e| pczt_err("tx_extractor", e))?;
    Ok(ExtractedTx { txid: tx.txid().to_string(), raw, expiry_height: u32::from(tx.expiry_height()) })
}

pub fn serialize(pczt: Pczt) -> Result<Vec<u8>, Error> {
    pczt.serialize().map_err(|e| pczt_err("encoding", e))
}

pub fn parse(bytes: &[u8]) -> Result<Pczt, Error> {
    Pczt::parse(bytes).map_err(|e| pczt_err("encoding", e))
}
