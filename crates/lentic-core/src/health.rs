use serde::{Deserialize, Serialize};

/// Loan health flag. Must stay in sync with `LoanBook.Health` in `contracts/src/LoanBook.sol`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Health {
    None = 0,
    Active = 1,
    MarginCall = 2,
    Liquidating = 3,
    Closed = 4,
    Liquidated = 5,
}

impl Health {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Health::None,
            1 => Health::Active,
            2 => Health::MarginCall,
            3 => Health::Liquidating,
            4 => Health::Closed,
            5 => Health::Liquidated,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Health::Closed | Health::Liquidated)
    }
}
