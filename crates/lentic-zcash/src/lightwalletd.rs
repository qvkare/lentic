//! Minimal lightwalletd / Zaino gRPC client: chain info, Ironwood tree states, broadcast.
//!
//! The policy signer reads chain data from at least two independent servers;
//! [`chain_info`] is cheap enough to cross-check them.

use serde::Serialize;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use zcash_client_backend::proto::service::{
    BlockId, ChainSpec, Empty, RawTransaction, compact_tx_streamer_client::CompactTxStreamerClient,
};

use crate::Error;

pub type Client = CompactTxStreamerClient<Channel>;

fn lw(e: impl std::fmt::Display) -> Error {
    Error::Lightwalletd(e.to_string())
}

/// Connects to `url` (`https://host:port` uses TLS with webpki roots).
pub async fn connect(url: &str) -> Result<Client, Error> {
    let mut endpoint = Endpoint::from_shared(url.to_string()).map_err(lw)?;
    if url.starts_with("https://") {
        endpoint = endpoint.tls_config(ClientTlsConfig::new().with_webpki_roots()).map_err(lw)?;
    }
    let channel = endpoint
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .connect()
        .await
        .map_err(lw)?;
    Ok(CompactTxStreamerClient::new(channel))
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainInfo {
    pub chain_name: String,
    pub block_height: u64,
    pub estimated_height: u64,
    pub consensus_branch_id: String,
    pub server: String,
    pub latest_block_hash: String,
}

pub async fn chain_info(client: &mut Client) -> Result<ChainInfo, Error> {
    let info = client.get_lightd_info(Empty {}).await.map_err(lw)?.into_inner();
    let latest = client.get_latest_block(ChainSpec {}).await.map_err(lw)?.into_inner();
    let mut hash = latest.hash.clone();
    hash.reverse();
    Ok(ChainInfo {
        chain_name: info.chain_name,
        block_height: latest.height,
        estimated_height: info.estimated_height,
        consensus_branch_id: info.consensus_branch_id,
        server: format!("{} {}", info.vendor, info.version),
        latest_block_hash: hex::encode(hash),
    })
}

/// The serialized Ironwood note commitment tree frontier at `height` (hex, as served).
pub async fn ironwood_tree(client: &mut Client, height: u64) -> Result<String, Error> {
    let state = client.get_tree_state(BlockId { height, hash: vec![] }).await.map_err(lw)?.into_inner();
    Ok(state.ironwood_tree)
}

/// Broadcasts a raw transaction; returns the server's error if rejected.
pub async fn broadcast(client: &mut Client, raw: Vec<u8>) -> Result<(), Error> {
    let resp = client.send_transaction(RawTransaction { data: raw, height: 0 }).await.map_err(lw)?.into_inner();
    if resp.error_code != 0 {
        return Err(Error::Lightwalletd(format!("rejected ({}): {}", resp.error_code, resp.error_message)));
    }
    Ok(())
}
