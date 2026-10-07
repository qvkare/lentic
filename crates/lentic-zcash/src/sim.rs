//! Offline Ironwood chain simulation: deposit notes to an escrow and witness them against an
//! in-memory note commitment tree. Lets tests and the local demo exercise the full PCZT
//! pipeline (including anchor refresh after signing) without a node.

use incrementalmerkletree::{Position, Retention};
use orchard::bundle::BundleVersion;
use orchard::keys::{FullViewingKey, Scope};
use orchard::note::ExtractedNoteCommitment;
use orchard::note_encryption::IronwoodDomain;
use orchard::tree::MerkleHashOrchard;
use orchard::value::NoteValue;
use orchard::{Anchor, Note, tree::MerklePath};
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use shardtree::{ShardTree, store::memory::MemoryShardStore};
use zcash_client_backend::proto::compact_formats::CompactOrchardAction;
use zcash_note_encryption::try_note_decryption;
use zcash_protocol::memo::Memo;

use crate::Error;

type Tree = ShardTree<MemoryShardStore<MerkleHashOrchard, u32>, 32, 16>;

/// The compact actions of a fresh Ironwood bundle paying `value` to `recipient` (with the
/// builder's dummy padding), as lightwalletd serves them in a compact block.
pub fn compact_payment(recipient: orchard::Address, value: u64) -> Result<Vec<CompactOrchardAction>, Error> {
    let version = BundleVersion::ironwood_v3();
    let mut builder = orchard::builder::Builder::new(
        orchard::builder::BundleType::DEFAULT,
        version,
        version.default_flags(),
        Anchor::empty_tree(),
    )
    .map_err(|e| Error::Builder(format!("{e:?}")))?;
    builder
        .add_output(None, recipient, NoteValue::from_raw(value), Memo::Empty.encode().into_bytes())
        .map_err(|e| Error::Builder(format!("{e:?}")))?;
    let (bundle, _) = builder
        .build::<i64>(&mut UnwrapErr(SysRng))
        .map_err(|e| Error::Builder(format!("{e:?}")))?
        .ok_or_else(|| Error::Builder("empty bundle".into()))?;
    Ok(bundle.actions().iter().map(CompactOrchardAction::from).collect())
}

/// An in-memory Ironwood commitment tree.
pub struct SimChain {
    tree: Tree,
    next_position: u64,
    next_checkpoint: u32,
}

impl Default for SimChain {
    fn default() -> Self {
        Self::new()
    }
}

impl SimChain {
    pub fn new() -> Self {
        SimChain { tree: ShardTree::new(MemoryShardStore::empty(), 100), next_position: 0, next_checkpoint: 1 }
    }

    /// Creates an Ironwood note paying `value` zatoshi to `fvk`'s default address, as a
    /// depositor's wallet would, and appends it to the tree (marked for witnessing).
    pub fn deposit(&mut self, fvk: &FullViewingKey, value: u64) -> Result<(Note, Position), Error> {
        let recipient = fvk.address_at(0u32, Scope::External);
        let version = BundleVersion::ironwood_v3();
        let mut builder = orchard::builder::Builder::new(
            orchard::builder::BundleType::DEFAULT,
            version,
            version.default_flags(),
            Anchor::empty_tree(),
        )
        .map_err(|e| Error::Builder(format!("{e:?}")))?;
        builder
            .add_output(None, recipient, NoteValue::from_raw(value), Memo::Empty.encode().into_bytes())
            .map_err(|e| Error::Builder(format!("{e:?}")))?;
        let (bundle, meta) = builder
            .build::<i64>(&mut UnwrapErr(SysRng))
            .map_err(|e| Error::Builder(format!("{e:?}")))?
            .ok_or_else(|| Error::Builder("empty bundle".into()))?;
        let action = &bundle.actions()[meta.output_action_index(0).expect("one output")];
        let ivk = fvk.to_ivk(Scope::External).prepare();
        let (note, _, _) = try_note_decryption(&IronwoodDomain::for_action(action), &ivk, action)
            .ok_or_else(|| Error::Builder("escrow cannot decrypt its own deposit".into()))?;
        let position = self.append(note.commitment().into(), true);
        Ok((note, position))
    }

    /// Appends unrelated commitments (other users' notes), moving the tree forward.
    pub fn append_noise(&mut self, count: usize) {
        for i in 0..count {
            let mut bytes = [0u8; 32];
            bytes[..8].copy_from_slice(&(self.next_position + i as u64).to_le_bytes());
            bytes[8] = 0xA5;
            let cmx = Option::from(ExtractedNoteCommitment::from_bytes(&bytes))
                .unwrap_or_else(|| ExtractedNoteCommitment::from_bytes(&[0; 32]).unwrap());
            self.append(cmx, false);
        }
    }

    fn append(&mut self, cmx: ExtractedNoteCommitment, marked: bool) -> Position {
        let leaf = MerkleHashOrchard::from_cmx(&cmx);
        let retention = if marked { Retention::Marked } else { Retention::Ephemeral };
        self.tree.append(leaf, retention).expect("tree has room");
        let position = Position::from(self.next_position);
        self.next_position += 1;
        position
    }

    /// Closes a "block": returns the anchor that witnesses produced now will commit to.
    pub fn checkpoint(&mut self) -> Anchor {
        let id = self.next_checkpoint;
        self.next_checkpoint += 1;
        self.tree.checkpoint(id).expect("checkpoint ids increase");
        Anchor::from(self.tree.root_at_checkpoint_depth(Some(0)).expect("root").expect("checkpointed"))
    }

    /// Witness for `position` at the latest checkpoint.
    pub fn witness(&self, position: Position) -> MerklePath {
        self.tree.witness_at_checkpoint_depth(position, 0).expect("witness").expect("marked leaf").into()
    }
}
