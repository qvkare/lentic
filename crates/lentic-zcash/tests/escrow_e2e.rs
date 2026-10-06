//! End-to-end escrow flows on an offline Ironwood chain: FROST DKG → escrow FVK → deposit →
//! PCZT → output checks → 2-of-3 FROST signatures → anchor refresh after signing → proof →
//! extracted v6 transaction. This is the protocol's critical path.

use lentic_core::Role;
use lentic_core::math::{Price, liquidation_split};
use lentic_frost::{dkg, fvk, group_ak};
use lentic_zcash::escrow::{self, EscrowInput, Payment};
use lentic_zcash::fee::zip317_fee;
use lentic_zcash::keys;
use lentic_zcash::network::regtest;
use lentic_zcash::signing::sign_locally;
use lentic_zcash::sim::SimChain;
use orchard::keys::{FullViewingKey, Scope, SpendingKey};
use rand_core_06::{OsRng, RngCore};
use zcash_protocol::consensus::BlockHeight;

const ZEC: u64 = 100_000_000;
const USDC: u128 = 1_000_000;
const TARGET: u32 = 1_000;

fn external_address() -> orchard::Address {
    loop {
        let mut b = [0u8; 32];
        OsRng.fill_bytes(&mut b);
        if let Some(sk) = Option::<SpendingKey>::from(SpendingKey::from_bytes(b)) {
            return FullViewingKey::from(&sk).address_at(0u32, Scope::External);
        }
    }
}

struct Loan {
    dkg: dkg::LocalDkg,
    fvk: FullViewingKey,
}

fn open_loan() -> Loan {
    let dkg = dkg::run_local(&mut OsRng).unwrap();
    let ak = group_ak(&dkg.public).unwrap();
    // The borrower samples the viewing seed; lender and policy derive the same FVK from it.
    let seed = fvk::fvk_seed(&mut OsRng);
    let fvk = fvk::derive_fvk(ak, &seed).unwrap();
    assert_eq!(fvk.to_bytes(), fvk::derive_fvk(ak, &seed).unwrap().to_bytes());
    Loan { dkg, fvk }
}

#[test]
fn escrow_address_and_ufvk_round_trip() {
    let loan = open_loan();
    let params = regtest();
    let ua = keys::encode_unified_address(&params, keys::escrow_address(&loan.fvk));
    assert_eq!(keys::decode_orchard_receiver(&params, &ua).unwrap(), keys::escrow_address(&loan.fvk));
    let ufvk = keys::encode_ufvk(&params, &loan.fvk).unwrap();
    assert_eq!(keys::decode_ufvk(&params, &ufvk).unwrap().to_bytes(), loan.fvk.to_bytes());
}

#[test]
fn liquidation_by_lender_and_policy_with_anchor_refresh() {
    let loan = open_loan();
    let mut chain = SimChain::new();
    let collateral = 20 * ZEC;
    let (note, position) = chain.deposit(&loan.fvk, collateral).unwrap();
    chain.append_noise(3);
    let anchor_at_signing = chain.checkpoint();

    // Policy signer computes the split with the live price.
    let lender = external_address();
    let borrower = external_address();
    let fee = zip317_fee(1, 2);
    let split = liquidation_split(collateral, fee, 10_000 * USDC, Price::from_usd(800.0), 500, 300).unwrap();
    assert!(split.borrower_zat > 0 && split.shortfall == 0);

    let spend = escrow::build_escrow_spend(
        regtest(),
        BlockHeight::from_u32(TARGET),
        &loan.fvk,
        anchor_at_signing,
        vec![EscrowInput { note, merkle_path: chain.witness(position) }],
        &[Payment::new(lender, split.lender_zat), Payment::new(borrower, split.borrower_zat)],
    )
    .unwrap();
    assert_eq!(spend.fee, fee);
    let sighash = escrow::shielded_sighash(&spend.pczt).unwrap();

    // What the signers see: no witness, no anchor. The sighash does not change (v6).
    let signer_view = escrow::redact_witnesses(spend.pczt.clone(), &spend.spend_indices);
    assert_eq!(escrow::shielded_sighash(&signer_view).unwrap(), sighash);

    // The policy signer only signs the split it computed.
    escrow::check_payments(&signer_view, &[(lender, split.lender_zat), (borrower, split.borrower_zat)]).unwrap();
    assert!(escrow::check_payments(&signer_view, &[(lender, collateral - fee)]).is_err());
    assert!(escrow::check_payments(&signer_view, &[(lender, split.lender_zat), (lender, split.borrower_zat)]).is_err());

    let pending = escrow::spends_to_sign(&signer_view).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].index, spend.spend_indices[0]);
    assert_eq!(pending[0].value, Some(collateral));

    // Lender + policy sign (the borrower is absent in a liquidation).
    let sigs = sign_locally(&signer_view, &loan.dkg.keys, &loan.dkg.public, [Role::Lender, Role::Policy]).unwrap();
    let signed = escrow::apply_spend_auth_signatures(spend.pczt, &sigs).unwrap();
    assert!(escrow::spends_to_sign(&signed).unwrap().is_empty());

    // The chain moves on before broadcast: refresh anchor + witness, then prove. No spend key.
    chain.append_noise(5);
    let anchor_at_broadcast = chain.checkpoint();
    assert_ne!(anchor_at_signing.to_bytes(), anchor_at_broadcast.to_bytes());
    let refreshed =
        escrow::refresh_witnesses(signed, anchor_at_broadcast, vec![(spend.spend_indices[0], chain.witness(position))])
            .unwrap();
    assert_eq!(escrow::shielded_sighash(&refreshed).unwrap(), sighash);

    let bytes = escrow::serialize(escrow::prove(refreshed).unwrap()).unwrap();
    let tx = escrow::extract(escrow::parse(&bytes).unwrap()).unwrap();
    assert!(!tx.raw.is_empty());
    assert_eq!(tx.expiry_height, TARGET + 40);
}

#[test]
fn close_by_borrower_and_policy() {
    let loan = open_loan();
    let mut chain = SimChain::new();
    let (a, pa) = chain.deposit(&loan.fvk, 15 * ZEC).unwrap();
    let (b, pb) = chain.deposit(&loan.fvk, 5 * ZEC).unwrap(); // a margin top-up is just another note
    let anchor = chain.checkpoint();
    let borrower = external_address();
    let fee = zip317_fee(2, 1);

    let spend = escrow::build_escrow_spend(
        regtest(),
        BlockHeight::from_u32(TARGET),
        &loan.fvk,
        anchor,
        vec![
            EscrowInput { note: a, merkle_path: chain.witness(pa) },
            EscrowInput { note: b, merkle_path: chain.witness(pb) },
        ],
        &[Payment::new(borrower, 20 * ZEC - fee)],
    )
    .unwrap();
    escrow::check_payments(&spend.pczt, &[(borrower, 20 * ZEC - fee)]).unwrap();
    let sigs = sign_locally(&spend.pczt, &loan.dkg.keys, &loan.dkg.public, [Role::Borrower, Role::Policy]).unwrap();
    assert_eq!(sigs.len(), 2);
    let signed = escrow::apply_spend_auth_signatures(spend.pczt, &sigs).unwrap();
    escrow::extract(escrow::prove(signed).unwrap()).unwrap();
}

#[test]
fn signatures_from_another_group_are_rejected() {
    let loan = open_loan();
    let stranger = open_loan();
    let mut chain = SimChain::new();
    let (note, pos) = chain.deposit(&loan.fvk, 2 * ZEC).unwrap();
    let anchor = chain.checkpoint();
    let spend = escrow::build_escrow_spend(
        regtest(),
        BlockHeight::from_u32(TARGET),
        &loan.fvk,
        anchor,
        vec![EscrowInput { note, merkle_path: chain.witness(pos) }],
        &[Payment::new(external_address(), 2 * ZEC - zip317_fee(1, 1))],
    )
    .unwrap();
    let forged =
        sign_locally(&spend.pczt, &stranger.dkg.keys, &stranger.dkg.public, [Role::Lender, Role::Policy]).unwrap();
    assert!(escrow::apply_spend_auth_signatures(spend.pczt, &forged).is_err());
}

#[test]
fn unbalanced_spend_is_refused() {
    let loan = open_loan();
    let mut chain = SimChain::new();
    let (note, pos) = chain.deposit(&loan.fvk, ZEC).unwrap();
    let anchor = chain.checkpoint();
    let result = escrow::build_escrow_spend(
        regtest(),
        BlockHeight::from_u32(TARGET),
        &loan.fvk,
        anchor,
        vec![EscrowInput { note, merkle_path: chain.witness(pos) }],
        &[Payment::new(external_address(), ZEC)], // forgets the fee
    );
    assert!(result.is_err());
}

/// The sighash commits to the consensus branch, so a spend signed under NU6.3 is
/// not valid under NU7. Same inputs and outputs, targets on either side of the upgrade.
#[test]
fn signatures_are_bound_to_the_consensus_branch() {
    use zcash_protocol::consensus::{BranchId, Network};

    let mut params = regtest();
    params.nu7 = Some(BlockHeight::from_u32(2_000));
    assert_eq!(lentic_zcash::network::branch_at(&params, 1_000), BranchId::Nu6_3);
    assert_eq!(lentic_zcash::network::branch_at(&params, 2_000), BranchId::Nu7);

    let loan = open_loan();
    let mut chain = SimChain::new();
    let (note, pos) = chain.deposit(&loan.fvk, 3 * ZEC).unwrap();
    let anchor = chain.checkpoint();
    let to = external_address();
    let payments = [Payment::new(to, 3 * ZEC - zip317_fee(1, 1))];
    let build = |height: u32| {
        escrow::build_escrow_spend(
            params,
            BlockHeight::from_u32(height),
            &loan.fvk,
            anchor,
            vec![EscrowInput { note, merkle_path: chain.witness(pos) }],
            &payments,
        )
        .unwrap()
    };
    let nu6_3 = build(1_000);
    let nu7 = build(2_000);
    assert_ne!(escrow::shielded_sighash(&nu6_3.pczt).unwrap(), escrow::shielded_sighash(&nu7.pczt).unwrap());

    // A signature made for the NU6.3 transaction does not verify on the NU7 one.
    let sigs = sign_locally(&nu6_3.pczt, &loan.dkg.keys, &loan.dkg.public, [Role::Lender, Role::Policy]).unwrap();
    let foreign: Vec<_> = sigs.iter().map(|(_, s)| (nu7.spend_indices[0], *s)).collect();
    assert!(escrow::apply_spend_auth_signatures(nu7.pczt.clone(), &foreign).is_err());

    // Re-signing under NU7 works and yields a valid transaction.
    let sigs = sign_locally(&nu7.pczt, &loan.dkg.keys, &loan.dkg.public, [Role::Lender, Role::Policy]).unwrap();
    escrow::extract(escrow::prove(escrow::apply_spend_auth_signatures(nu7.pczt, &sigs).unwrap()).unwrap()).unwrap();

    // The real testnet is already past NU7 (4,465,026); the builder accepts Ironwood there.
    let testnet_height = 4_470_000;
    assert_eq!(lentic_zcash::network::branch_at(&Network::TestNetwork, testnet_height), BranchId::Nu7);
    escrow::build_escrow_spend(
        Network::TestNetwork,
        BlockHeight::from_u32(testnet_height),
        &loan.fvk,
        anchor,
        vec![EscrowInput { note, merkle_path: chain.witness(pos) }],
        &payments,
    )
    .unwrap();
}
