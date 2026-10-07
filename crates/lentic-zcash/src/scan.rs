//! Watching an escrow (or a test wallet) from its full viewing key.
//!
//! Compact blocks come from lightwalletd; every Ironwood action is trial-decrypted with the
//! key's external and internal IVKs, every action's nullifier is checked against the notes
//! found so far, and every note commitment is appended to an in-memory note commitment tree
//! that starts from the server's tree state just before the birthday. The tree yields the
//! anchor and witnesses a spend needs, so the scanner is all a coordinator or the policy
//! signer needs to see what an escrow holds and to spend it.
//!
//! After each block the tree size is compared with the size the server reports for that
//! block, and each block must extend the previous one; either mismatch aborts the sync.

use std::collections::BTreeMap;

use incrementalmerkletree::frontier::Frontier;
use incrementalmerkletree::{Marking, Position, Retention};
use orchard::keys::{FullViewingKey, PreparedIncomingViewingKey, Scope};
use orchard::note::Nullifier;
use orchard::note_encryption::{CompactAction, IronwoodDomain};
use orchard::tree::{MerkleHashOrchard, MerklePath};
use orchard::{Anchor, Note};
use serde::Serialize;
use shardtree::ShardTree;
use shardtree::store::memory::MemoryShardStore;
use zcash_client_backend::proto::compact_formats::CompactBlock;
use zcash_client_backend::proto::service::{BlockId, BlockRange};
use zcash_note_encryption::try_compact_note_decryption;

use crate::Error;
use crate::escrow::EscrowInput;
use crate::lightwalletd::{self, Client};

type Tree = ShardTree<MemoryShardStore<MerkleHashOrchard, u32>, 32, 16>;

/// Checkpoints (one per scanned block) kept for choosing an anchor.
const MAX_CHECKPOINTS: usize = 1_000;
/// Blocks requested per `GetBlockRange` call.
const RANGE_CHUNK: u32 = 1_000;

fn scan_err(e: impl std::fmt::Display) -> Error {
    Error::Scan(e.to_string())
}

/// Where a note was spent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpentIn {
    pub txid: String,
    pub height: u32,
}

/// A note received by the watched key.
#[derive(Clone, Debug)]
pub struct ScannedNote {
    pub note: Note,
    pub scope: Scope,
    pub position: Position,
    pub height: u32,
    /// Display (byte-reversed) hex of the receiving transaction.
    pub txid: String,
    pub nullifier: Nullifier,
    pub spent: Option<SpentIn>,
}

impl ScannedNote {
    pub fn value(&self) -> u64 {
        self.note.value().inner()
    }
}

/// A note as reported to people (CLI, API).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteSummary {
    pub value_zat: u64,
    pub height: u32,
    pub position: u64,
    pub txid: String,
    pub change: bool,
    pub spent: Option<SpentIn>,
}

/// Balances and notes of a watched key.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub birthday: u32,
    pub synced_to: u32,
    /// Ironwood note commitments in the tree after `synced_to`, as scanned.
    pub ironwood_tree_size: u64,
    /// The same, as the server last reported it (checked after every block).
    pub server_ironwood_tree_size: Option<u32>,
    /// Unspent value in notes mined at or below `synced_to`.
    pub unspent_zat: u64,
    pub notes: Vec<NoteSummary>,
}

/// Incremental scanner for one full viewing key.
pub struct Watcher {
    fvk: FullViewingKey,
    ivks: Vec<(Scope, PreparedIncomingViewingKey)>,
    birthday: u32,
    tree: Tree,
    next_position: u64,
    synced_to: Option<u32>,
    /// Internal byte order hash of the last scanned block.
    last_hash: Option<Vec<u8>>,
    server_tree_size: Option<u32>,
    notes: Vec<ScannedNote>,
    by_nullifier: BTreeMap<[u8; 32], usize>,
}

impl Watcher {
    /// A watcher that will start at `birthday` (the first block that may pay the key).
    pub fn new(fvk: FullViewingKey, birthday: u32) -> Self {
        let ivks = [Scope::External, Scope::Internal].into_iter().map(|s| (s, fvk.to_ivk(s).prepare())).collect();
        Watcher {
            fvk,
            ivks,
            birthday: birthday.max(1),
            tree: ShardTree::new(MemoryShardStore::empty(), MAX_CHECKPOINTS),
            next_position: 0,
            synced_to: None,
            last_hash: None,
            server_tree_size: None,
            notes: Vec::new(),
            by_nullifier: BTreeMap::new(),
        }
    }

    pub fn fvk(&self) -> &FullViewingKey {
        &self.fvk
    }

    pub fn birthday(&self) -> u32 {
        self.birthday
    }

    /// The last block applied, if any.
    pub fn synced_to(&self) -> Option<u32> {
        self.synced_to
    }

    pub fn notes(&self) -> &[ScannedNote] {
        &self.notes
    }

    /// The note spent by an action with nullifier `nf`, if it is one of ours.
    pub fn note_by_nullifier(&self, nf: &Nullifier) -> Option<&ScannedNote> {
        self.by_nullifier.get(&nf.to_bytes()).map(|&i| &self.notes[i])
    }

    /// Starts the tree from a known frontier: the Ironwood tree as of the end of block
    /// `birthday - 1`, whose hash (internal byte order) is `prev_hash` if known.
    pub fn start_from(
        &mut self,
        frontier: Frontier<MerkleHashOrchard, 32>,
        prev_hash: Option<Vec<u8>>,
    ) -> Result<(), Error> {
        if self.synced_to.is_some() {
            return Err(scan_err("watcher already started"));
        }
        self.next_position = frontier.value().map(|f| u64::from(f.position()) + 1).unwrap_or(0);
        let start = self.birthday - 1;
        self.tree
            .insert_frontier(frontier, Retention::Checkpoint { id: start, marking: Marking::None })
            .map_err(scan_err)?;
        self.synced_to = Some(start);
        self.last_hash = prev_hash;
        Ok(())
    }

    /// Fetches the tree state before the birthday (first call) and applies every block up to
    /// the server's tip. Returns the tip.
    pub async fn sync(&mut self, client: &mut Client) -> Result<u32, Error> {
        if self.synced_to.is_none() {
            let state = client
                .get_tree_state(BlockId { height: u64::from(self.birthday - 1), hash: vec![] })
                .await
                .map_err(|e| Error::Lightwalletd(e.to_string()))?
                .into_inner();
            let frontier = state.ironwood_tree().map_err(scan_err)?.to_frontier();
            let mut hash = hex::decode(&state.hash).map_err(scan_err)?;
            hash.reverse();
            self.start_from(frontier, Some(hash))?;
        }
        let tip = u32::try_from(lightwalletd::chain_info(client).await?.block_height).map_err(scan_err)?;
        let mut from = self.synced_to.expect("started above") + 1;
        while from <= tip {
            let to = tip.min(from + RANGE_CHUNK - 1);
            let mut stream = client
                .get_block_range(BlockRange {
                    start: Some(BlockId { height: u64::from(from), hash: vec![] }),
                    end: Some(BlockId { height: u64::from(to), hash: vec![] }),
                    pool_types: vec![],
                })
                .await
                .map_err(|e| Error::Lightwalletd(e.to_string()))?
                .into_inner();
            while let Some(block) = stream.message().await.map_err(|e| Error::Lightwalletd(e.to_string()))? {
                self.apply_block(&block)?;
            }
            if self.synced_to != Some(to) {
                return Err(scan_err(format!("server stopped at {:?}, expected block {to}", self.synced_to)));
            }
            from = to + 1;
        }
        Ok(tip)
    }

    /// Applies one compact block (must be the next one). Every check runs before the watcher
    /// changes, so a rejected block leaves it as it was.
    pub fn apply_block(&mut self, block: &CompactBlock) -> Result<(), Error> {
        let height = u32::try_from(block.height).map_err(scan_err)?;
        let expected = self.synced_to.map(|h| h + 1).unwrap_or(self.birthday);
        if height != expected {
            return Err(scan_err(format!("expected block {expected}, got {height}")));
        }
        if let Some(last) = &self.last_hash
            && !block.prev_hash.is_empty()
            && &block.prev_hash != last
        {
            return Err(scan_err(format!("block {height} does not extend the scanned chain (reorg); rescan")));
        }

        // Parse and trial-decrypt everything first.
        let mut parsed = Vec::new();
        for tx in &block.vtx {
            let mut txid = tx.txid.clone();
            txid.reverse();
            let txid = hex::encode(txid);
            for action in &tx.ironwood_actions {
                let bad =
                    |e: zcash_client_backend::proto::CompactFormatError| scan_err(format!("block {height}: {e:?}"));
                let nf = action.nf().map_err(bad)?;
                let cmx = action.cmx().map_err(bad)?;
                let compact = CompactAction::try_from(action).map_err(bad)?;
                let domain = IronwoodDomain::for_compact_action(&compact);
                let found = self.ivks.iter().find_map(|(scope, ivk)| {
                    try_compact_note_decryption(&domain, ivk, &compact).map(|(note, _)| (*scope, note))
                });
                parsed.push((txid.clone(), nf, cmx, found));
            }
        }
        let size_after = self.next_position + parsed.len() as u64;
        if let Some(meta) = &block.chain_metadata
            && meta.ironwood_commitment_tree_size != 0
            && u64::from(meta.ironwood_commitment_tree_size) != size_after
        {
            return Err(scan_err(format!(
                "Ironwood tree size after block {height}: server {}, scanned {size_after}",
                meta.ironwood_commitment_tree_size
            )));
        }

        for (txid, nf, cmx, found) in parsed {
            if let Some(&i) = self.by_nullifier.get(&nf.to_bytes())
                && self.notes[i].spent.is_none()
            {
                self.notes[i].spent = Some(SpentIn { txid: txid.clone(), height });
            }
            let position = Position::from(self.next_position);
            let retention = if found.is_some() { Retention::Marked } else { Retention::Ephemeral };
            self.tree.append(MerkleHashOrchard::from_cmx(&cmx), retention).map_err(scan_err)?;
            self.next_position += 1;
            if let Some((scope, note)) = found {
                let nullifier = note.nullifier(&self.fvk);
                self.by_nullifier.insert(nullifier.to_bytes(), self.notes.len());
                self.notes.push(ScannedNote { note, scope, position, height, txid, nullifier, spent: None });
            }
        }
        self.tree.checkpoint(height).map_err(scan_err)?;
        if let Some(meta) = &block.chain_metadata
            && meta.ironwood_commitment_tree_size != 0
        {
            self.server_tree_size = Some(meta.ironwood_commitment_tree_size);
        }
        self.synced_to = Some(height);
        self.last_hash = (!block.hash.is_empty()).then(|| block.hash.clone());
        Ok(())
    }

    /// Unspent notes mined at or below `height`.
    pub fn unspent_at(&self, height: u32) -> impl Iterator<Item = &ScannedNote> {
        self.notes.iter().filter(move |n| n.spent.is_none() && n.height <= height)
    }

    /// Unspent value mined at or below `height`.
    pub fn balance_at(&self, height: u32) -> u64 {
        self.unspent_at(height).map(ScannedNote::value).sum()
    }

    /// The Ironwood anchor as of the end of block `height` (must be a scanned block within
    /// the last checkpoints).
    pub fn anchor(&self, height: u32) -> Result<Anchor, Error> {
        let root = self
            .tree
            .root_at_checkpoint_id(&height)
            .map_err(scan_err)?
            .ok_or_else(|| scan_err(format!("no checkpoint at height {height}")))?;
        Ok(Anchor::from(root))
    }

    /// The witness of a note against the anchor at `height`.
    pub fn witness(&self, position: Position, height: u32) -> Result<MerklePath, Error> {
        let path = self
            .tree
            .witness_at_checkpoint_id(position, &height)
            .map_err(scan_err)?
            .ok_or_else(|| scan_err(format!("no checkpoint at height {height}")))?;
        Ok(path.into())
    }

    /// Every unspent note mined at or below `anchor_height`, witnessed against that anchor.
    pub fn spendable(&self, anchor_height: u32) -> Result<(Anchor, Vec<EscrowInput>), Error> {
        let anchor = self.anchor(anchor_height)?;
        let inputs = self
            .unspent_at(anchor_height)
            .map(|n| Ok(EscrowInput { note: n.note, merkle_path: self.witness(n.position, anchor_height)? }))
            .collect::<Result<Vec<_>, Error>>()?;
        Ok((anchor, inputs))
    }

    pub fn summary(&self) -> Summary {
        let synced_to = self.synced_to.unwrap_or(self.birthday - 1);
        Summary {
            birthday: self.birthday,
            synced_to,
            ironwood_tree_size: self.next_position,
            server_ironwood_tree_size: self.server_tree_size,
            unspent_zat: self.balance_at(synced_to),
            notes: self
                .notes
                .iter()
                .map(|n| NoteSummary {
                    value_zat: n.value(),
                    height: n.height,
                    position: u64::from(n.position),
                    txid: n.txid.clone(),
                    change: n.scope == Scope::Internal,
                    spent: n.spent.clone(),
                })
                .collect(),
        }
    }
}
