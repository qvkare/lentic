//! The loans this policy signer holds, and its readings of them.
//!
//! Every round the signer reads ZEC/USD, then for each loan: what the escrow holds (its own
//! scan from the escrow's viewing key, for loans whose key it helped make), what is owed
//! (interest on the outstanding principal, from the vault's repayment events), the LTV and
//! the health. A change of health is written to LoanBook with `--report-health`, using the
//! price reading's timestamp as the oracle round so rounds stay monotonic across restarts.
//! When the escrow notes of a co-signed spend are seen spent, the outcome (Liquidated or
//! Closed) is written with the attestation's hash. One loan's failure does not stop the
//! round for the others.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::primitives::B256;
use anyhow::{Context, Result};
use lentic_core::{Health, LoanTerms, Owed, math, terms_hash};
use lentic_zcash::scan::Watcher;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use crate::api::AppState;
use crate::price::{PriceKind, PriceReading, reading};

const HISTORY: usize = 720;

/// `<loans dir>/<loanId>.json`: what the signer keeps per loan (inside the TEE in production).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldLoan {
    pub loan_id: B256,
    pub salt: B256,
    pub terms: LoanTerms,
    /// The DKG session whose escrow key backs this loan (absent for terms-only demo loans).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dkg_session: Option<String>,
}

pub fn load_dir(dir: &Path) -> Result<Vec<HeldLoan>> {
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut loans = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let loan: HeldLoan = serde_json::from_str(&std::fs::read_to_string(&path)?)
            .with_context(|| format!("parsing {}", path.display()))?;
        loan.terms.validate().map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        loans.push(loan);
    }
    Ok(loans)
}

pub fn save(dir: &Path, loan: &HeldLoan) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(format!("{}.json", loan.loan_id)), serde_json::to_vec_pretty(loan)?)?;
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    /// Oracle round: the price reading's unix timestamp.
    pub round: u64,
    pub price_usd: f64,
    pub price_kind: PriceKind,
    pub debt: u128,
    pub collateral_zat: u64,
    pub ltv_bps: Option<u32>,
    pub health: Health,
}

/// The signer's latest view of a loan, whatever its stage.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoanState {
    /// Opened on LoanBook (funded).
    pub funded: bool,
    pub health: Option<Health>,
    pub owed: Owed,
    pub collateral_zat: u64,
    /// "escrow" when read from the escrow itself, "terms" for terms-only loans.
    pub collateral_source: &'static str,
    pub escrow_notes: usize,
    pub zcash_tip: Option<u32>,
    /// Never funded and past the funding deadline: borrower + signer may refund.
    pub refundable: bool,
    pub at: u64,
}

#[derive(Default)]
pub struct Book {
    loans: RwLock<Vec<HeldLoan>>,
    readings: RwLock<BTreeMap<B256, VecDeque<Reading>>>,
    states: RwLock<BTreeMap<B256, LoanState>>,
    latest_price: RwLock<Option<PriceReading>>,
    test_price: RwLock<Option<f64>>,
    last_error: RwLock<Option<String>>,
    /// One round at a time: the timer and the test-price route must not both report a change.
    round_lock: Mutex<()>,
    /// One escrow watcher per loan, synced incrementally.
    pub watchers: Mutex<BTreeMap<B256, Watcher>>,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Book {
    pub fn new(loans: Vec<HeldLoan>) -> Self {
        Book { loans: RwLock::new(loans), ..Default::default() }
    }

    pub async fn loans(&self) -> Vec<HeldLoan> {
        self.loans.read().await.clone()
    }

    pub async fn held(&self, loan_id: &B256) -> Option<HeldLoan> {
        self.loans.read().await.iter().find(|l| &l.loan_id == loan_id).cloned()
    }

    /// Adds a loan (registration); replaces nothing.
    pub async fn hold(&self, loan: HeldLoan) -> bool {
        let mut loans = self.loans.write().await;
        if loans.iter().any(|l| l.loan_id == loan.loan_id) {
            return false;
        }
        loans.push(loan);
        true
    }

    pub async fn readings(&self, loan_id: &B256) -> Vec<Reading> {
        self.readings.read().await.get(loan_id).map(|d| d.iter().cloned().collect()).unwrap_or_default()
    }

    pub async fn state(&self, loan_id: &B256) -> Option<LoanState> {
        self.states.read().await.get(loan_id).cloned()
    }

    pub async fn latest_price(&self) -> Option<PriceReading> {
        self.latest_price.read().await.clone()
    }

    pub async fn last_error(&self) -> Option<String> {
        self.last_error.read().await.clone()
    }

    pub async fn set_test_price(&self, usd: Option<f64>) {
        *self.test_price.write().await = usd;
    }

    /// Syncs the loan's escrow watcher and returns the tip and its unspent notes
    /// (nullifier, value).
    pub async fn escrow_notes(&self, s: &AppState, loan: &HeldLoan) -> Result<(u32, Vec<([u8; 32], u64)>)> {
        let session = loan.dkg_session.as_deref().context("this loan has no escrow held by the signer")?;
        let share = s.dkg.share(session).await.context("the escrow key for this loan is missing")?;
        let mut watchers = self.watchers.lock().await;
        if let std::collections::btree_map::Entry::Vacant(slot) = watchers.entry(loan.loan_id) {
            slot.insert(Watcher::new(share.fvk()?, share.birthday));
        }
        let watcher = watchers.get_mut(&loan.loan_id).expect("inserted above");
        let mut client = s.zcash_client().await?;
        let tip = match watcher.sync(&mut client).await {
            Ok(tip) => tip,
            Err(e) => {
                // A reorg or a bad block: start over from the birthday next time.
                watchers.remove(&loan.loan_id);
                s.drop_zcash_client().await;
                return Err(e.into());
            }
        };
        let notes = watcher.unspent_at(tip).map(|n| (n.nullifier.to_bytes(), n.value())).collect();
        Ok((tip, notes))
    }

    /// One round: price, then every held loan.
    pub async fn round(&self, s: &AppState) {
        let _one_at_a_time = self.round_lock.lock().await;
        let result = self.round_inner(s).await;
        *self.last_error.write().await = result.as_ref().err().map(|e| format!("{e:#}"));
        if let Err(e) = result {
            warn!("round failed: {e:#}");
        }
    }

    async fn round_inner(&self, s: &AppState) -> Result<()> {
        let at = now();
        let price = match *self.test_price.read().await {
            Some(usd) => reading(usd, PriceKind::TestOverride, vec![], at),
            None => s.feed.read(at).await?,
        };
        *self.latest_price.write().await = Some(price.clone());

        let mut errors = Vec::new();
        for held in self.loans().await {
            if let Err(e) = self.assess_loan(s, &held, &price).await {
                warn!(loan = %held.loan_id, "{e:#}");
                errors.push(format!("{}: {e:#}", held.loan_id));
            }
        }
        anyhow::ensure!(errors.is_empty(), "{}", errors.join("; "));
        Ok(())
    }

    async fn assess_loan(&self, s: &AppState, held: &HeldLoan, price: &PriceReading) -> Result<()> {
        let at = price.at;
        let mut state = LoanState { at, collateral_source: "terms", ..Default::default() };
        let collateral_zat = if held.dkg_session.is_some() {
            let (tip, notes) = self.escrow_notes(s, held).await?;
            state.collateral_source = "escrow";
            state.escrow_notes = notes.len();
            state.zcash_tip = Some(tip);
            notes.iter().map(|n| n.1).sum()
        } else {
            held.terms.collateral_zat
        };
        state.collateral_zat = collateral_zat;

        let loan = lentic_base::read_loan(&s.provider, s.deployment.loan_book, held.loan_id).await?;
        match &loan {
            None => {
                state.refundable = collateral_zat > 0 && now() > held.terms.funding_deadline;
            }
            Some(view) if view.terms_hash != terms_hash(&held.terms, held.salt) => {
                warn!(loan = %held.loan_id, "held terms do not match the on-chain termsHash; skipping");
            }
            Some(view) => {
                state.funded = true;
                state.health = Some(view.health);
                if !view.health.is_terminal() {
                    let repayments =
                        lentic_base::repayments(&s.provider, view.vault, held.loan_id, s.deployment.start_block)
                            .await?;
                    state.owed =
                        math::owed_at(held.terms.principal, held.terms.apr_bps, view.opened_at, &repayments, now());
                    let debt = state.owed.total();
                    let health = math::assess_at(&held.terms, debt, collateral_zat, price.price, now());
                    state.health = Some(health);
                    let r = Reading {
                        round: at,
                        price_usd: price.usd,
                        price_kind: price.kind,
                        debt,
                        collateral_zat,
                        ltv_bps: math::ltv_bps(debt, collateral_zat, price.price),
                        health,
                    };
                    {
                        let mut all = self.readings.write().await;
                        let series = all.entry(held.loan_id).or_default();
                        if series.len() == HISTORY {
                            series.pop_front();
                        }
                        series.push_back(r);
                    }
                    let settling = s.cosign.intent(&held.loan_id).await.is_some_and(|i| i.reported.is_none());
                    if s.report_health && !settling && health != view.health && at > view.oracle_round {
                        let tx = lentic_base::update_health(
                            &s.provider,
                            s.deployment.loan_book,
                            held.loan_id,
                            health,
                            at,
                            B256::ZERO,
                        )
                        .await?;
                        info!(loan = %held.loan_id, ?health, %tx, "health reported to LoanBook");
                    }
                }
            }
        }
        self.states.write().await.insert(held.loan_id, state);
        s.cosign.settle(s, held.loan_id, loan.as_ref(), at).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_held_loans_and_rejects_bad_terms() {
        let dir = std::env::temp_dir().join(format!("lentic-book-{}", now()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = r#"{"loanId":"0x1111111111111111111111111111111111111111111111111111111111111111",
          "salt":"0x4444444444444444444444444444444444444444444444444444444444444444",
          "terms":{"version":1,"chainId":31337,"lenderVault":"0x8464135c8F25Da09e49BC8782676a84730C318bC",
          "borrower":"0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC","principal":10000000000,"aprBps":1200,
          "originationFeeBps":100,"collateralZat":2000000000,"marginLtvBps":6000,"liquidationLtvBps":7500,
          "haircutBps":500,"penaltyBps":300,"fundingDeadline":1,"maturity":2,
          "escrowFvkHash":"0x3333333333333333333333333333333333333333333333333333333333333333",
          "borrowerReturnAddress":"u1b","lenderZcashAddress":"u1l"}}"#;
        std::fs::write(dir.join("a.json"), good).unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let loans = load_dir(&dir).unwrap();
        assert_eq!(loans.len(), 1);
        assert_eq!(loans[0].terms.liquidation_ltv_bps, 7500);
        assert!(loans[0].dkg_session.is_none());

        // Round trip through save, with an escrow session.
        let mut held = loans[0].clone();
        held.dkg_session = Some("0xabc".into());
        let other = dir.join("saved");
        save(&other, &held).unwrap();
        assert_eq!(load_dir(&other).unwrap()[0].dkg_session.as_deref(), Some("0xabc"));

        std::fs::write(dir.join("b.json"), good.replace("\"haircutBps\":500", "\"haircutBps\":10000")).unwrap();
        assert!(load_dir(&dir).is_err());
        assert!(load_dir(&dir.join("missing")).unwrap().is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}
