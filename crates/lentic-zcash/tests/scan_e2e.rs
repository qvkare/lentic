//! The escrow watcher against compact blocks built from real Ironwood bundles: it finds the
//! deposit, its anchor and witness produce a spend whose proof verifies, it marks the note
//! spent when the spend is mined, and a test wallet sees its own change.

use incrementalmerkletree::frontier::Frontier;
use lentic_core::Role;
use lentic_frost::{dkg, fvk, group_ak};
use lentic_zcash::escrow::{self, Payment};
use lentic_zcash::fee::zip317_fee;
use lentic_zcash::network::regtest;
use lentic_zcash::scan::Watcher;
use lentic_zcash::signing::sign_locally;
use lentic_zcash::sim::compact_payment;
use lentic_zcash::wallet::TestWallet;
use orchard::keys::FullViewingKey;
use rand_core_06::{OsRng, RngCore};
use zcash_client_backend::proto::compact_formats::{ChainMetadata, CompactBlock, CompactOrchardAction, CompactTx};
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::{BlockHeight, BranchId};

const ZEC: u64 = 100_000_000;
const BIRTHDAY: u32 = 100;
const TARGET: u32 = 1_000;

/// Hands out blocks in order, keeping hashes chained and the Ironwood tree size.
struct Chain {
    height: u32,
    tree_size: u32,
    last_hash: Vec<u8>,
}

impl Chain {
    fn new() -> Self {
        Chain { height: BIRTHDAY, tree_size: 0, last_hash: vec![0; 32] }
    }

    fn block(&mut self, txs: Vec<Vec<CompactOrchardAction>>) -> CompactBlock {
        let mut hash = vec![0u8; 32];
        OsRng.fill_bytes(&mut hash);
        let vtx = txs
            .into_iter()
            .enumerate()
            .map(|(i, actions)| {
                let mut txid = vec![0u8; 32];
                OsRng.fill_bytes(&mut txid);
                self.tree_size += actions.len() as u32;
                CompactTx { index: i as u64, txid, ironwood_actions: actions, ..Default::default() }
            })
            .collect();
        let block = CompactBlock {
            height: u64::from(self.height),
            hash: hash.clone(),
            prev_hash: std::mem::replace(&mut self.last_hash, hash),
            vtx,
            chain_metadata: Some(ChainMetadata { ironwood_commitment_tree_size: self.tree_size, ..Default::default() }),
            ..Default::default()
        };
        self.height += 1;
        block
    }
}

fn stranger() -> orchard::Address {
    TestWallet::generate().address()
}

fn mined_actions(raw: &[u8]) -> Vec<CompactOrchardAction> {
    let tx = Transaction::read(raw, BranchId::Nu6_3).unwrap();
    tx.ironwood_bundle().unwrap().actions().iter().map(CompactOrchardAction::from).collect()
}

fn watcher(fvk: &FullViewingKey) -> Watcher {
    let mut w = Watcher::new(fvk.clone(), BIRTHDAY);
    w.start_from(Frontier::empty(), Some(vec![0; 32])).unwrap();
    w
}

#[test]
fn watcher_finds_witnesses_and_sees_the_spend() {
    let group = dkg::run_local(&mut OsRng).unwrap();
    let escrow_fvk = fvk::derive_fvk(group_ak(&group.public).unwrap(), &fvk::fvk_seed(&mut OsRng)).unwrap();
    let escrow_addr = lentic_zcash::keys::escrow_address(&escrow_fvk);

    let mut chain = Chain::new();
    let mut w = watcher(&escrow_fvk);
    // Someone else's payment, then the deposit next to noise, then a quiet block.
    w.apply_block(&chain.block(vec![compact_payment(stranger(), ZEC).unwrap()])).unwrap();
    let deposit_height = chain.height;
    w.apply_block(
        &chain
            .block(vec![compact_payment(stranger(), 3 * ZEC).unwrap(), compact_payment(escrow_addr, 2 * ZEC).unwrap()]),
    )
    .unwrap();
    w.apply_block(&chain.block(vec![])).unwrap();
    let anchor_height = chain.height - 1;

    let summary = w.summary();
    assert_eq!(summary.notes.len(), 1);
    assert_eq!(summary.notes[0].height, deposit_height);
    assert_eq!(summary.unspent_zat, 2 * ZEC);
    assert_eq!(w.balance_at(deposit_height - 1), 0);

    // Spend it 2-of-3: the proof only verifies if the scanned anchor and witness are right.
    let (anchor, inputs) = w.spendable(anchor_height).unwrap();
    let (lender, borrower) = (stranger(), stranger());
    let fee = zip317_fee(1, 2);
    let spend = escrow::build_escrow_spend(
        regtest(),
        BlockHeight::from_u32(TARGET),
        &escrow_fvk,
        anchor,
        inputs,
        &[Payment::new(lender, ZEC), Payment::new(borrower, ZEC - fee)],
    )
    .unwrap();
    let sigs = sign_locally(&spend.pczt, &group.keys, &group.public, [Role::Lender, Role::Policy]).unwrap();
    let signed = escrow::apply_spend_auth_signatures(spend.pczt, &sigs).unwrap();

    // The coordinator maps each spend back to its note by nullifier.
    let ours: Vec<_> = escrow::spend_nullifiers(&signed)
        .unwrap()
        .into_iter()
        .filter_map(|(i, nf)| w.note_by_nullifier(&nf).map(|n| (i, n.position)))
        .collect();
    assert_eq!(ours.len(), 1);
    assert_eq!(ours[0].0, spend.spend_indices[0]);

    let tx = escrow::extract(escrow::prove(signed).unwrap()).unwrap();

    // Mined: the watcher marks the note spent and does not mistake the payouts for its own.
    let spend_height = chain.height;
    w.apply_block(&chain.block(vec![mined_actions(&tx.raw)])).unwrap();
    let summary = w.summary();
    assert_eq!(summary.notes.len(), 1);
    assert_eq!(summary.unspent_zat, 0);
    let spent = summary.notes[0].spent.as_ref().unwrap();
    assert_eq!(spent.height, spend_height);
}

#[test]
fn test_wallet_pays_and_keeps_its_change() {
    let wallet = TestWallet::generate();
    let restored = TestWallet::from_bytes(wallet.to_bytes()).unwrap();
    assert_eq!(restored.fvk().to_bytes(), wallet.fvk().to_bytes());

    let mut chain = Chain::new();
    let mut w = watcher(&wallet.fvk());
    w.apply_block(&chain.block(vec![compact_payment(wallet.address(), ZEC).unwrap()])).unwrap();
    let anchor_height = chain.height - 1;

    let (anchor, inputs) = w.spendable(anchor_height).unwrap();
    let escrow = stranger();
    let spend = escrow::build_spend(
        regtest(),
        BlockHeight::from_u32(TARGET),
        &wallet.fvk(),
        anchor,
        inputs,
        &[Payment::new(escrow, ZEC / 4)],
        Some(wallet.change_address()),
    )
    .unwrap();
    assert_eq!(spend.fee, zip317_fee(1, 2));
    let tx = escrow::extract(escrow::prove(wallet.sign(spend.pczt).unwrap()).unwrap()).unwrap();

    w.apply_block(&chain.block(vec![mined_actions(&tx.raw)])).unwrap();
    let summary = w.summary();
    assert_eq!(summary.notes.len(), 2);
    assert!(summary.notes[0].spent.is_some());
    assert!(summary.notes[1].change);
    assert_eq!(summary.unspent_zat, ZEC - ZEC / 4 - spend.fee);

    // Not enough for the payment plus the fee.
    let (anchor, inputs) = w.spendable(chain.height - 1).unwrap();
    assert!(
        escrow::build_spend(
            regtest(),
            BlockHeight::from_u32(TARGET),
            &wallet.fvk(),
            anchor,
            inputs,
            &[Payment::new(escrow, ZEC)],
            Some(wallet.change_address()),
        )
        .is_err()
    );
}

#[test]
fn watcher_rejects_gaps_reorgs_and_tree_size_mismatches() {
    let wallet = TestWallet::generate();
    let mut chain = Chain::new();
    let mut w = watcher(&wallet.fvk());
    let first = chain.block(vec![compact_payment(stranger(), ZEC).unwrap()]);
    w.apply_block(&first).unwrap();

    // Skipping a block.
    let _skipped = chain.block(vec![]);
    assert!(w.apply_block(&chain.block(vec![])).is_err());

    // A block that does not extend the last one.
    let mut fork = Chain { height: first.height as u32 + 1, tree_size: chain.tree_size, last_hash: vec![9; 32] };
    assert!(w.apply_block(&fork.block(vec![])).is_err());

    // A server reporting a tree size the actions do not add up to.
    let mut honest = Chain { height: first.height as u32 + 1, tree_size: 2, last_hash: first.hash.clone() };
    let mut lying = honest.block(vec![compact_payment(stranger(), ZEC).unwrap()]);
    lying.chain_metadata.as_mut().unwrap().ironwood_commitment_tree_size += 1;
    assert!(w.apply_block(&lying).is_err());
}
