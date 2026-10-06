//! Network selection.

use serde::{Deserialize, Serialize};
use zcash_protocol::consensus::{BlockHeight, BranchId, Network, NetworkUpgrade, Parameters};
use zcash_protocol::local_consensus::LocalNetwork;

/// A regtest network with every upgrade through NU6.3 active from low heights, so v6
/// transactions with Ironwood bundles can be built from height 10.
pub fn regtest() -> LocalNetwork {
    LocalNetwork {
        overwinter: Some(BlockHeight::from_u32(1)),
        sapling: Some(BlockHeight::from_u32(2)),
        blossom: Some(BlockHeight::from_u32(3)),
        heartwood: Some(BlockHeight::from_u32(4)),
        canopy: Some(BlockHeight::from_u32(5)),
        nu5: Some(BlockHeight::from_u32(6)),
        nu6: Some(BlockHeight::from_u32(7)),
        nu6_1: Some(BlockHeight::from_u32(8)),
        nu6_2: Some(BlockHeight::from_u32(9)),
        nu6_3: Some(BlockHeight::from_u32(10)),
        nu7: None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZcashNetwork {
    Mainnet,
    Testnet,
    Regtest,
}

impl std::str::FromStr for ZcashNetwork {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mainnet" | "main" => Ok(ZcashNetwork::Mainnet),
            "testnet" | "test" => Ok(ZcashNetwork::Testnet),
            "regtest" => Ok(ZcashNetwork::Regtest),
            _ => Err(format!("unknown Zcash network `{s}` (mainnet, testnet, regtest)")),
        }
    }
}

impl std::fmt::Display for ZcashNetwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ZcashNetwork::Mainnet => "mainnet",
            ZcashNetwork::Testnet => "testnet",
            ZcashNetwork::Regtest => "regtest",
        })
    }
}

/// Parameters for a [`ZcashNetwork`], usable wherever `P: Parameters` is expected.
#[derive(Clone, Copy, Debug)]
pub enum Params {
    Public(Network),
    Regtest(LocalNetwork),
}

impl ZcashNetwork {
    pub fn params(self) -> Params {
        match self {
            ZcashNetwork::Mainnet => Params::Public(Network::MainNetwork),
            ZcashNetwork::Testnet => Params::Public(Network::TestNetwork),
            ZcashNetwork::Regtest => Params::Regtest(regtest()),
        }
    }
}

impl Parameters for Params {
    fn network_type(&self) -> zcash_protocol::consensus::NetworkType {
        match self {
            Params::Public(n) => n.network_type(),
            Params::Regtest(n) => n.network_type(),
        }
    }

    fn activation_height(&self, nu: NetworkUpgrade) -> Option<BlockHeight> {
        match self {
            Params::Public(n) => n.activation_height(nu),
            Params::Regtest(n) => n.activation_height(nu),
        }
    }
}

/// Consensus branch a transaction targeting `height` must commit to. A signed but
/// unbroadcast transaction must be re-signed if this changes before it is mined.
pub fn branch_at<P: Parameters>(params: &P, height: u32) -> BranchId {
    BranchId::for_height(params, BlockHeight::from_u32(height))
}

/// Whether Ironwood (NU6.3) is active at `height`.
pub fn ironwood_active<P: Parameters>(params: &P, height: u32) -> bool {
    params.is_nu_active(NetworkUpgrade::Nu6_3, BlockHeight::from_u32(height))
}
