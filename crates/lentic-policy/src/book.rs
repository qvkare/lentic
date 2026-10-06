//! The loans this policy signer holds terms for, and its readings of them.
//!
//! Every round the signer reads ZEC/USD, then for each loan it holds terms for: accrued debt
//! (simple interest since LoanBook.openedAt, less what the vault has received), collateral,
//! LTV and health. Readings are kept in memory for the gauge and the hydrograph. With
//! `--report-health`, a change of health is written to LoanBook, using the price reading's
//! timestamp as the oracle round so rounds stay monotonic across restarts.
//!
//! Collateral is the amount committed in the terms until the escrow watcher reports what the
//! escrow actually holds (`collateralSource` says which).

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::primitives::B256;
use anyhow::{Context, Result};
use lentic_core::{Health, LoanTerms, math, terms_hash};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::api::AppState;
use crate::price::{PriceKind, PriceReading, reading};

const HISTORY: usize = 720;

/// `dev/loans/<loanId>.json`: what the signer keeps per loan (inside the TEE in production).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldLoan {
    pub loan_id: B256,
    pub salt: B256,
    pub terms: LoanTerms,
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

#[derive(Default)]
pub struct Book {
    pub loans: Vec<HeldLoan>,
    readings: RwLock<BTreeMap<B256, VecDeque<Reading>>>,
    latest_price: RwLock<Option<PriceReading>>,
    test_price: RwLock<Option<f64>>,
    last_error: RwLock<Option<String>>,
    /// One round at a time: the timer and the test-price route must not both report a change.
    round_lock: tokio::sync::Mutex<()>,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Book {
    pub fn new(loans: Vec<HeldLoan>) -> Self {
        Book { loans, ..Default::default() }
    }

    pub fn held(&self, loan_id: &B256) -> Option<&HeldLoan> {
        self.loans.iter().find(|l| &l.loan_id == loan_id)
    }

    pub async fn readings(&self, loan_id: &B256) -> Vec<Reading> {
        self.readings.read().await.get(loan_id).map(|d| d.iter().cloned().collect()).unwrap_or_default()
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

        for held in &self.loans {
            let Some(loan) = lentic_base::read_loan(&s.provider, s.deployment.loan_book, held.loan_id).await? else {
                continue; // not opened yet
            };
            if loan.terms_hash != terms_hash(&held.terms, held.salt) {
                warn!(loan = %held.loan_id, "held terms do not match the on-chain termsHash; skipping");
                continue;
            }
            if loan.health.is_terminal() {
                continue;
            }
            let position = lentic_base::vault_position(&s.provider, loan.vault, held.loan_id).await?;
            let elapsed = at.saturating_sub(loan.opened_at);
            let repaid = position.principal_repaid + position.interest_paid;
            let debt = math::accrued_debt(held.terms.principal, held.terms.apr_bps, elapsed).saturating_sub(repaid);
            let collateral_zat = held.terms.collateral_zat;
            let health = math::assess(&held.terms, debt, collateral_zat, price.price);
            let r = Reading {
                round: price.at,
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
            if s.report_health && health != loan.health && price.at > loan.oracle_round {
                let tx = lentic_base::update_health(
                    &s.provider,
                    s.deployment.loan_book,
                    held.loan_id,
                    health,
                    price.at,
                    B256::ZERO,
                )
                .await?;
                info!(loan = %held.loan_id, ?health, %tx, "health reported to LoanBook");
            }
        }
        Ok(())
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

        std::fs::write(dir.join("b.json"), good.replace("\"haircutBps\":500", "\"haircutBps\":10000")).unwrap();
        assert!(load_dir(&dir).is_err());
        assert!(load_dir(&dir.join("missing")).unwrap().is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}
