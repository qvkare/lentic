//! ZIP 317 conventional fee for the escrow's shielded-only transactions.
//!
//! For an Orchard-protocol bundle (Orchard or Ironwood) the logical action count is
//! `max(spends, outputs)`; the fee is `5000 · max(2, logical actions)` zatoshi.

pub const MARGINAL_FEE: u64 = 5_000;
pub const GRACE_ACTIONS: u64 = 2;

pub fn zip317_fee(spends: usize, outputs: usize) -> u64 {
    MARGINAL_FEE * GRACE_ACTIONS.max(spends.max(outputs) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fees() {
        assert_eq!(zip317_fee(1, 1), 10_000);
        assert_eq!(zip317_fee(1, 2), 10_000);
        assert_eq!(zip317_fee(3, 2), 15_000);
    }
}
