use serde::{Deserialize, Serialize};

/// The three holders of a loan's 2-of-3 FROST group.
///
/// The discriminant is the FROST participant identifier, so every party derives the same
/// identifier for a role without coordination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u16)]
pub enum Role {
    Borrower = 1,
    Lender = 2,
    /// The TEE policy signer, the only share Lentic operates.
    Policy = 3,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Borrower, Role::Lender, Role::Policy];

    pub fn id(self) -> u16 {
        self as u16
    }

    pub fn from_id(id: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.id() == id)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Borrower => "borrower",
            Role::Lender => "lender",
            Role::Policy => "policy",
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Role {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|r| r.as_str() == s)
            .ok_or_else(|| format!("unknown role `{s}` (expected borrower, lender or policy)"))
    }
}
