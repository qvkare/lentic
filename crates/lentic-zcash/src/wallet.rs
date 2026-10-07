//! A single-key Ironwood wallet for testnet demos: it funds escrows and receives payouts.
//!
//! The spending key is handled in the clear, so this is for test networks only. Notes and
//! witnesses come from a [`crate::scan::Watcher`] over the wallet's full viewing key.

use orchard::keys::{FullViewingKey, Scope, SpendAuthorizingKey, SpendingKey};
use pczt::roles::signer::Signer;
use rand_core_06::{OsRng, RngCore};

use crate::{Error, Pczt, escrow, pczt_err};

pub struct TestWallet {
    sk: SpendingKey,
}

impl TestWallet {
    /// A fresh random key.
    pub fn generate() -> Self {
        loop {
            let mut bytes = [0u8; 32];
            OsRng.fill_bytes(&mut bytes);
            if let Ok(wallet) = Self::from_bytes(bytes) {
                return wallet;
            }
        }
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, Error> {
        Option::from(SpendingKey::from_bytes(bytes))
            .map(|sk| TestWallet { sk })
            .ok_or_else(|| Error::Address("not a valid Orchard spending key".into()))
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        *self.sk.to_bytes()
    }

    pub fn fvk(&self) -> FullViewingKey {
        FullViewingKey::from(&self.sk)
    }

    /// Where the wallet receives (diversifier index 0, external scope).
    pub fn address(&self) -> orchard::Address {
        self.fvk().address_at(0u32, Scope::External)
    }

    /// Where its own change goes.
    pub fn change_address(&self) -> orchard::Address {
        self.fvk().address_at(0u32, Scope::Internal)
    }

    /// Signs every pending Ironwood spend of a PCZT built from this wallet's notes.
    pub fn sign(&self, pczt: Pczt) -> Result<Pczt, Error> {
        let pending = escrow::spends_to_sign(&pczt)?;
        let ask = SpendAuthorizingKey::from(&self.sk);
        let mut signer = Signer::new(pczt).map_err(|e| pczt_err("signer", e))?;
        for spend in pending {
            signer.sign_ironwood(escrow::rng(), spend.index, &ask).map_err(|e| pczt_err("signer", e))?;
        }
        Ok(signer.finish())
    }
}
