//! Escrow viewing key and addresses.
//!
//! Ironwood keeps Orchard keys and addresses unchanged (ZIP 229): the escrow's deposit address
//! is the FVK's default external Orchard address, encoded as a shielded-only unified address,
//! and funds sent to it after NU6.3 land in the Ironwood pool.

use orchard::keys::{FullViewingKey, Scope};
use zcash_keys::address::{Address, UnifiedAddress};
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_protocol::consensus::Parameters;

use crate::Error;

/// The escrow's deposit address (diversifier index 0, external scope).
pub fn escrow_address(fvk: &FullViewingKey) -> orchard::Address {
    fvk.address_at(0u32, Scope::External)
}

/// Encodes an Orchard address as a unified address for `params` (ZIP 316 revision 2,
/// `zu1…` / `zutest1…`).
pub fn encode_unified_address<P: Parameters>(params: &P, addr: orchard::Address) -> String {
    UnifiedAddress::from_receivers(Some(addr), None, None, None, None)
        .expect("an Orchard receiver is a valid UA")
        .encode(params)
}

/// The same address in the original revision 0 encoding (`u1…` / `utest1…`), for wallets,
/// faucets and explorers that do not read revision 2 yet.
pub fn encode_unified_address_r0<P: Parameters>(params: &P, addr: orchard::Address) -> String {
    use zcash_address::unified::{self, Encoding, Revision, Uitem};
    unified::Address::try_from_items(
        Revision::R0,
        vec![Uitem::Data(unified::Receiver::Orchard(addr.to_raw_address_bytes()))],
    )
    .expect("an Orchard receiver is a valid UA")
    .encode(&params.network_type())
}

/// Decodes a unified address and returns its Orchard receiver (the only kind Lentic pays to).
pub fn decode_orchard_receiver<P: Parameters>(params: &P, ua: &str) -> Result<orchard::Address, Error> {
    match Address::decode(params, ua) {
        Some(Address::Unified(ua)) => {
            ua.orchard().copied().ok_or_else(|| Error::Address("unified address has no Orchard receiver".into()))
        }
        Some(_) => Err(Error::Address("not a unified address".into())),
        None => Err(Error::Address(format!("cannot decode `{ua}` for this network"))),
    }
}

/// The escrow's UFVK encoding (what every party imports into its watcher).
pub fn encode_ufvk<P: Parameters>(params: &P, fvk: &FullViewingKey) -> Result<String, Error> {
    let ufvk =
        UnifiedFullViewingKey::from_orchard_fvk(fvk.clone()).map_err(|e| Error::Address(format!("UFVK: {e:?}")))?;
    Ok(ufvk.encode(params))
}

/// Parses a UFVK and returns its Orchard FVK.
pub fn decode_ufvk<P: Parameters>(params: &P, encoded: &str) -> Result<FullViewingKey, Error> {
    let ufvk = UnifiedFullViewingKey::decode(params, encoded).map_err(Error::Address)?;
    ufvk.orchard().cloned().ok_or_else(|| Error::Address("UFVK has no Orchard component".into()))
}
