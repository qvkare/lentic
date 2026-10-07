//! Loan arithmetic the policy signer enforces.
//!
//! Units: debt in micro-USDC (1e-6 USD), collateral in zatoshi (1e-8 ZEC), price in USD per
//! ZEC scaled by 1e8 (Pyth `expo = -8`). The value of `z` zatoshi in micro-USDC is therefore
//! `z * P / 1e10`. All intermediate products fit in `u128` for any realistic loan.

use serde::{Deserialize, Serialize};

use crate::{Health, LoanTerms};

const BPS: u128 = 10_000;
const ZAT_PRICE_TO_MICRO_USD: u128 = 10_000_000_000; // 1e8 (zat) * 1e8 (price) / 1e6 (µUSD)
const SECONDS_PER_YEAR: u128 = 365 * 24 * 60 * 60;

/// ZEC/USD price scaled by 1e8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Price {
    pub usd_e8: u64,
}

impl Price {
    pub fn from_usd(usd: f64) -> Self {
        Price { usd_e8: (usd * 1e8).round() as u64 }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SplitError {
    #[error("price must be positive")]
    ZeroPrice,
    #[error("haircut must be below 100%")]
    InvalidHaircut,
    #[error("fee {fee} exceeds collateral {collateral}")]
    FeeExceedsCollateral { fee: u64, collateral: u64 },
}

/// Value of `zat` zatoshi in micro-USDC at `price`.
pub fn collateral_value(zat: u64, price: Price) -> u128 {
    zat as u128 * price.usd_e8 as u128 / ZAT_PRICE_TO_MICRO_USD
}

/// Loan-to-value in basis points; `None` when the collateral is worthless at this price.
pub fn ltv_bps(debt: u128, collateral_zat: u64, price: Price) -> Option<u32> {
    let value = collateral_value(collateral_zat, price);
    if value == 0 {
        return None;
    }
    Some((debt * BPS / value).min(u32::MAX as u128) as u32)
}

/// Principal plus simple interest accrued over `elapsed_secs`, rounded up.
pub fn accrued_debt(principal: u128, apr_bps: u32, elapsed_secs: u64) -> u128 {
    let num = principal * apr_bps as u128 * elapsed_secs as u128;
    let den = BPS * SECONDS_PER_YEAR;
    principal + num.div_ceil(den)
}

/// A repayment into the lender's vault: when, and how much (micro-USDC).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repayment {
    pub at: u64,
    pub amount: u128,
}

/// What a borrower owes at a moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Owed {
    pub principal: u128,
    /// Interest accrued and not yet paid (rounded up).
    pub interest: u128,
}

impl Owed {
    pub fn total(&self) -> u128 {
        self.principal + self.interest
    }
}

/// Debt under simple interest on the principal still outstanding. Each repayment is booked
/// the way `LenderVault.repay` books it: principal first, the rest pays interest. Interest
/// stops accruing once the principal is repaid.
pub fn owed_at(principal: u128, apr_bps: u32, opened_at: u64, repayments: &[Repayment], now: u64) -> Owed {
    let mut outstanding = principal;
    let mut interest_num = 0u128; // in micro-USDC × BPS × seconds-per-year
    let mut interest_paid = 0u128;
    let mut t = opened_at;
    let mut sorted = repayments.to_vec();
    sorted.sort_by_key(|r| r.at);
    for r in sorted.iter().filter(|r| r.at <= now) {
        let at = r.at.max(t);
        interest_num += outstanding * apr_bps as u128 * (at - t) as u128;
        let to_principal = r.amount.min(outstanding);
        outstanding -= to_principal;
        interest_paid += r.amount - to_principal;
        t = at;
    }
    interest_num += outstanding * apr_bps as u128 * now.saturating_sub(t) as u128;
    let interest = interest_num.div_ceil(BPS * SECONDS_PER_YEAR);
    Owed { principal: outstanding, interest: interest.saturating_sub(interest_paid) }
}

/// Health of a live loan at `now`: past maturity with debt left it is in default and may be
/// liquidated whatever the price; otherwise the loan-to-value decides.
pub fn assess_at(terms: &LoanTerms, debt: u128, collateral_zat: u64, price: Price, now: u64) -> Health {
    if debt > 0 && now >= terms.maturity {
        return Health::Liquidating;
    }
    assess(terms, debt, collateral_zat, price)
}

/// Health the policy signer reports for a live loan, from the loan-to-value alone.
pub fn assess(terms: &LoanTerms, debt: u128, collateral_zat: u64, price: Price) -> Health {
    match ltv_bps(debt, collateral_zat, price) {
        None => Health::Liquidating,
        Some(ltv) if ltv >= terms.liquidation_ltv_bps => Health::Liquidating,
        Some(ltv) if ltv >= terms.margin_ltv_bps => Health::MarginCall,
        Some(_) => Health::Active,
    }
}

/// How the escrowed collateral is divided in a liquidation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiquidationSplit {
    /// Paid to the lender's shielded address.
    pub lender_zat: u64,
    /// Returned to the borrower's shielded return address.
    pub borrower_zat: u64,
    /// ZIP 317 fee, paid out of the collateral.
    pub fee_zat: u64,
    /// Debt (incl. penalty) left uncovered when the collateral is insufficient; the lender's loss.
    pub shortfall: u128,
}

/// The split the policy signer computes at liquidation time with the live price:
/// `lender = min(C − fee, ceil((D + penalty) / (P · (1 − h))))`, surplus to the borrower.
pub fn liquidation_split(
    collateral_zat: u64,
    fee_zat: u64,
    debt: u128,
    price: Price,
    haircut_bps: u32,
    penalty_bps: u32,
) -> Result<LiquidationSplit, SplitError> {
    if price.usd_e8 == 0 {
        return Err(SplitError::ZeroPrice);
    }
    if haircut_bps as u128 >= BPS {
        return Err(SplitError::InvalidHaircut);
    }
    let available = collateral_zat
        .checked_sub(fee_zat)
        .ok_or(SplitError::FeeExceedsCollateral { fee: fee_zat, collateral: collateral_zat })?;

    let debt_with_penalty = debt * (BPS + penalty_bps as u128) / BPS;
    let num = debt_with_penalty * BPS * ZAT_PRICE_TO_MICRO_USD;
    let den = price.usd_e8 as u128 * (BPS - haircut_bps as u128);
    let needed = num.div_ceil(den);

    if needed <= available as u128 {
        let lender_zat = needed as u64;
        Ok(LiquidationSplit { lender_zat, borrower_zat: available - lender_zat, fee_zat, shortfall: 0 })
    } else {
        let haircut_price = Price { usd_e8: (price.usd_e8 as u128 * (BPS - haircut_bps as u128) / BPS) as u64 };
        let covered = collateral_value(available, haircut_price);
        Ok(LiquidationSplit {
            lender_zat: available,
            borrower_zat: 0,
            fee_zat,
            shortfall: debt_with_penalty.saturating_sub(covered),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terms::tests::fixture;

    const USDC: u128 = 1_000_000;
    const ZEC: u64 = 100_000_000;

    #[test]
    fn collateral_value_units() {
        // 1 ZEC at $1,400 = 1,400 USDC.
        assert_eq!(collateral_value(ZEC, Price::from_usd(1_400.0)), 1_400 * USDC);
    }

    #[test]
    fn ltv() {
        // 10k USDC against 20 ZEC at $1,000 = 50%.
        assert_eq!(ltv_bps(10_000 * USDC, 20 * ZEC, Price::from_usd(1_000.0)), Some(5_000));
        assert_eq!(ltv_bps(1, 0, Price::from_usd(1_000.0)), None);
    }

    #[test]
    fn interest_accrues_linearly_and_rounds_up() {
        let year = 365 * 24 * 60 * 60;
        assert_eq!(accrued_debt(10_000 * USDC, 1_200, year), 11_200 * USDC);
        assert_eq!(accrued_debt(10_000 * USDC, 1_200, 0), 10_000 * USDC);
        assert_eq!(accrued_debt(1, 1, 1), 2);
    }

    #[test]
    fn interest_accrues_only_on_outstanding_principal() {
        let year = 365 * 24 * 60 * 60;
        let p = 10_000 * USDC;
        // Untouched for a year: same as simple interest on the whole principal.
        assert_eq!(owed_at(p, 1_200, 0, &[], year).total(), accrued_debt(p, 1_200, year));
        // Principal repaid on day 1: one day of interest stays owed, nothing accrues after.
        let day = 24 * 60 * 60;
        let owed = owed_at(p, 1_200, 0, &[Repayment { at: day, amount: p }], year);
        assert_eq!(owed.principal, 0);
        assert_eq!(owed.interest, (p * 1_200 * day as u128).div_ceil(10_000 * year as u128));
        // Half the principal at mid-year: interest on 10k for half a year, then on 5k.
        let owed = owed_at(p, 1_200, 0, &[Repayment { at: year / 2, amount: p / 2 }], year);
        assert_eq!(owed.total(), p / 2 + 600 * USDC + 300 * USDC);
        // Paying principal and interest in one go closes the loan.
        let interest = owed_at(p, 1_200, 0, &[], 30 * day).interest;
        let closed = owed_at(p, 1_200, 0, &[Repayment { at: 30 * day, amount: p + interest }], 40 * day);
        assert_eq!(closed.total(), 0);
        // Repayments after `now` do not count yet.
        assert_eq!(owed_at(p, 1_200, 0, &[Repayment { at: year, amount: p }], day).principal, p);
    }

    #[test]
    fn assess_thresholds() {
        let (terms, _) = fixture(); // margin 60%, liquidation 75%
        let c = 20 * ZEC;
        let d = 10_000 * USDC;
        assert_eq!(assess(&terms, d, c, Price::from_usd(1_000.0)), Health::Active); // 50%
        assert_eq!(assess(&terms, d, c, Price::from_usd(800.0)), Health::MarginCall); // 62.5%
        assert_eq!(assess(&terms, d, c, Price::from_usd(600.0)), Health::Liquidating); // 83%
        // Past maturity with debt left: default, liquidatable at any price.
        let p = Price::from_usd(1_000.0);
        assert_eq!(assess_at(&terms, d, c, p, terms.maturity - 1), Health::Active);
        assert_eq!(assess_at(&terms, d, c, p, terms.maturity), Health::Liquidating);
        assert_eq!(assess_at(&terms, 0, c, p, terms.maturity), Health::Active);
    }

    #[test]
    fn split_with_surplus_pays_borrower() {
        // Debt 10k, penalty 3% -> 10,300; price $800, haircut 5% -> $760; needs 13.5526... ZEC.
        let s = liquidation_split(20 * ZEC, 20_000, 10_000 * USDC, Price::from_usd(800.0), 500, 300).unwrap();
        assert_eq!(s.lender_zat, 1_355_263_158);
        assert_eq!(s.lender_zat + s.borrower_zat + s.fee_zat, 20 * ZEC);
        assert_eq!(s.shortfall, 0);
        // The lender's share covers the debt at the haircut price.
        assert!(collateral_value(s.lender_zat, Price::from_usd(760.0)) >= 10_300 * USDC);
    }

    #[test]
    fn split_gap_down_leaves_shortfall_to_lender() {
        let s = liquidation_split(10 * ZEC, 20_000, 10_000 * USDC, Price::from_usd(800.0), 500, 300).unwrap();
        assert_eq!(s.lender_zat, 10 * ZEC - 20_000);
        assert_eq!(s.borrower_zat, 0);
        assert!(s.shortfall > 0);
    }

    #[test]
    fn split_rejects_bad_inputs() {
        assert_eq!(liquidation_split(1, 0, 1, Price { usd_e8: 0 }, 0, 0), Err(SplitError::ZeroPrice));
        assert_eq!(liquidation_split(1, 0, 1, Price::from_usd(1.0), 10_000, 0), Err(SplitError::InvalidHaircut));
        assert!(matches!(
            liquidation_split(1, 2, 1, Price::from_usd(1.0), 0, 0),
            Err(SplitError::FeeExceedsCollateral { .. })
        ));
    }
}
