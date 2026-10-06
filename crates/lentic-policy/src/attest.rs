//! Binding the policy signer's Base key to the code that holds it.
//!
//! REPORTDATA = signer address (20) ‖ 0 (12) ‖ config hash (32), checked on chain by
//! `AttestationVerifier.register`. In a dstack CVM the quote comes from the guest agent; in
//! development a structurally identical, unsigned quote is built locally (mirrors
//! `contracts/src/mocks/DevQuote.sol`) and the local deployment uses a mock DCAP verifier.

use alloy::primitives::{Address, B256, keccak256};
use anyhow::{Context, Result, bail};
use dstack_sdk::dstack_client::DstackClient;

pub const HEADER_LEN: usize = 48;
pub const BODY_LEN: usize = 584;
pub const MRTD_OFFSET: usize = HEADER_LEN + 136;
pub const RTMR0_OFFSET: usize = HEADER_LEN + 328;
pub const REPORT_DATA_OFFSET: usize = HEADER_LEN + 520;

/// Config identity used by the local deployment (`DeployLocal.DEV_CONFIG_HASH`).
pub const DEV_CONFIG_ID: &str = "lentic-policy-dev";

pub fn config_hash(config_id: &str) -> B256 {
    keccak256(config_id.as_bytes())
}

pub fn report_data(signer: Address, config_hash: B256) -> [u8; 64] {
    let mut rd = [0u8; 64];
    rd[..20].copy_from_slice(signer.as_slice());
    rd[32..].copy_from_slice(config_hash.as_slice());
    rd
}

/// keccak256(MRTD ‖ RTMR0..3) of a TDX v4 quote: what `AttestationVerifier` pins.
pub fn measurement(quote: &[u8]) -> Result<B256> {
    if quote.len() < HEADER_LEN + BODY_LEN {
        bail!("quote too short ({} bytes)", quote.len());
    }
    if u16::from_le_bytes([quote[0], quote[1]]) != 4 {
        bail!("unsupported quote version");
    }
    let mut pre = Vec::with_capacity(240);
    pre.extend_from_slice(&quote[MRTD_OFFSET..MRTD_OFFSET + 48]);
    pre.extend_from_slice(&quote[RTMR0_OFFSET..RTMR0_OFFSET + 192]);
    Ok(keccak256(pre))
}

/// The unsigned development quote (same registers as DevQuote.sol).
pub fn dev_quote(report_data: &[u8; 64]) -> Vec<u8> {
    let mut q = vec![0u8; HEADER_LEN + BODY_LEN];
    q[0] = 4;
    q[MRTD_OFFSET..MRTD_OFFSET + 48].fill(0x11);
    for i in 0..192 {
        q[RTMR0_OFFSET + i] = 0x20 + (i / 48) as u8;
    }
    q[REPORT_DATA_OFFSET..REPORT_DATA_OFFSET + 64].copy_from_slice(report_data);
    q
}

pub enum Attestor {
    Dev,
    Dstack(DstackClient),
}

impl Attestor {
    pub fn kind(&self) -> &'static str {
        match self {
            Attestor::Dev => "dev",
            Attestor::Dstack(_) => "dstack",
        }
    }

    pub async fn quote(&self, report_data: &[u8; 64]) -> Result<Vec<u8>> {
        match self {
            Attestor::Dev => Ok(dev_quote(report_data)),
            Attestor::Dstack(client) => {
                let resp = client.get_quote(report_data.to_vec()).await.context("dstack GetQuote")?;
                hex::decode(resp.quote.trim_start_matches("0x")).context("quote hex")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_quote_round_trips_report_data_and_matches_solidity_measurement() {
        let signer: Address = "0x90F79bf6EB2c4f870365E785982E1f101E93b906".parse().unwrap();
        let rd = report_data(signer, config_hash(DEV_CONFIG_ID));
        let q = dev_quote(&rd);
        assert_eq!(&q[REPORT_DATA_OFFSET..REPORT_DATA_OFFSET + 20], signer.as_slice());
        assert!(q[REPORT_DATA_OFFSET + 20..REPORT_DATA_OFFSET + 32].iter().all(|b| *b == 0));
        // DevQuote.measurement() in Solidity: keccak256(0x11×48 ‖ 0x20×48 ‖ 0x21×48 ‖ 0x22×48 ‖ 0x23×48)
        let mut pre = vec![0x11u8; 48];
        for r in 0..4u8 {
            pre.extend(std::iter::repeat_n(0x20 + r, 48));
        }
        assert_eq!(measurement(&q).unwrap(), keccak256(pre));
    }
}
