//! `lentic-signer`: the borrower's or the lender's side of a loan's FROST group.
//!
//! With a policy signer:
//!   base-key    make a test Base key (the party's identity in the DKG and its approvals)
//!   dkg         create a session, or join one as borrower or lender (through the signer's
//!               relay; round-2 packages are sealed to their recipients)
//!   terms       approve terms (EIP-191) and register them with the policy signer
//!   cosign      the whole escrow spend for a close, refund or liquidation: the signer's
//!               plan, PCZT, two FROST rounds, fresh anchor, proof, broadcast
//! Local tools:
//!   dkg-local   run the 3-party DKG in one process and write each role's share bundle
//!   show        print the escrow address, UFVK and escrowFvkHash from a share bundle
//!   inspect     print a PCZT's sighash, pending spends (alpha) and verified payments
//!   sign-local  sign every pending escrow spend in a PCZT with two share bundles
//!   terms-hash  validate a terms JSON and print its termsHash (as LoanBook stores it)
//!   demo        offline end-to-end liquidation on a simulated Ironwood chain

mod net;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use alloy_primitives::{Address, B256};
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use lentic_core::math::{Price, liquidation_split};
use lentic_core::{Role, escrow_fvk_hash};
use lentic_frost::dkg::{KeyPackage, PublicKeyPackage};
use lentic_frost::{dkg, fvk, group_ak};
use lentic_zcash::escrow::{self, EscrowInput, Payment};
use lentic_zcash::network::ZcashNetwork;
use lentic_zcash::{fee::zip317_fee, keys, signing::sign_locally, sim::SimChain};
use orchard::keys::FullViewingKey;
use rand_core_06::OsRng;
use serde::{Deserialize, Serialize};
use zcash_protocol::consensus::BlockHeight;

#[derive(Parser)]
#[command(name = "lentic-signer", version, about = "Lentic borrower/lender FROST signer")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a new test Base key (hex) to a file and print its address.
    BaseKey {
        #[arg(long)]
        out: PathBuf,
    },
    /// Make a loan's escrow key with the policy signer (create a session, or join one).
    #[command(subcommand)]
    Dkg(DkgCommand),
    /// Approve a loan's terms, or register approved terms with the policy signer.
    #[command(subcommand)]
    Terms(TermsCommand),
    /// Co-sign an escrow spend with the policy signer and broadcast it.
    Cosign {
        #[arg(long, env = "LENTIC_POLICY_URL", default_value = "http://127.0.0.1:8787")]
        policy: String,
        #[arg(long)]
        loan_id: B256,
        #[arg(long, value_enum)]
        kind: net::Kind,
        /// This party's share bundle (from `dkg join`).
        #[arg(long)]
        share: PathBuf,
        #[arg(long, default_value = "https://testnet.zec.rocks:443")]
        server: String,
        /// Where the PCZTs, the raw transaction and the attestation are written.
        #[arg(long, default_value = "dev/cosign")]
        out_dir: PathBuf,
        #[arg(long)]
        no_broadcast: bool,
    },
    /// Run the 3-party DKG locally and write <out>/<role>.share.json for each role.
    DkgLocal {
        #[arg(long, default_value = "loan-shares")]
        out: PathBuf,
    },
    /// Print the escrow address, UFVK and escrowFvkHash for a share bundle.
    Show {
        #[arg(long)]
        share: PathBuf,
        #[arg(long, default_value = "testnet")]
        network: ZcashNetwork,
    },
    /// Print what a PCZT asks its signers to sign.
    Inspect {
        #[arg(long)]
        pczt: PathBuf,
    },
    /// Sign every pending escrow spend with two share bundles (development).
    SignLocal {
        #[arg(long)]
        pczt: PathBuf,
        #[arg(long, num_args = 2, required = true)]
        shares: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Validate loan terms (JSON, camelCase) and print keccak256(abi.encode(terms, salt)).
    TermsHash {
        #[arg(long)]
        terms: PathBuf,
        /// 32-byte hex salt shared by the three parties.
        #[arg(long)]
        salt: String,
        /// Print the loan id instead: keccak256(abi.encode(vault, termsHash)).
        #[arg(long)]
        loan_id: bool,
    },
    /// Offline end-to-end liquidation on a simulated Ironwood chain.
    Demo {
        #[arg(long, default_value_t = 20.0)]
        collateral_zec: f64,
        #[arg(long, default_value_t = 10_000.0)]
        debt_usdc: f64,
        #[arg(long, default_value_t = 800.0)]
        price_usd: f64,
    },
}

#[derive(Subcommand)]
enum DkgCommand {
    /// Open a DKG session on the policy signer for a borrower and a lender.
    Create {
        #[arg(long, env = "LENTIC_POLICY_URL", default_value = "http://127.0.0.1:8787")]
        policy: String,
        /// The loan-specific Base address that will receive the USDC.
        #[arg(long)]
        borrower: Address,
        /// The lender's vault owner.
        #[arg(long)]
        lender: Address,
    },
    /// Take part in a session as borrower or lender; writes this party's share bundle.
    Join {
        #[arg(long, env = "LENTIC_POLICY_URL", default_value = "http://127.0.0.1:8787")]
        policy: String,
        #[arg(long)]
        session: String,
        #[arg(long)]
        role: Role,
        /// File with this party's Base key (hex).
        #[arg(long)]
        base_key: PathBuf,
        #[arg(long, default_value = "testnet")]
        network: ZcashNetwork,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 600)]
        timeout_secs: u64,
    },
}

#[derive(Subcommand)]
enum TermsCommand {
    /// Sign (EIP-191) a loan's terms for the policy signer. The loan id is derived:
    /// keccak256(abi.encode(vault, termsHash)), as LoanBook computes it.
    Approve {
        #[arg(long)]
        terms: PathBuf,
        #[arg(long)]
        salt: B256,
        #[arg(long)]
        base_key: PathBuf,
    },
    /// Hand terms approved by both parties to the policy signer.
    Register {
        #[arg(long, env = "LENTIC_POLICY_URL", default_value = "http://127.0.0.1:8787")]
        policy: String,
        #[arg(long)]
        terms: PathBuf,
        #[arg(long)]
        salt: B256,
        #[arg(long)]
        session: String,
        #[arg(long)]
        borrower_signature: String,
        #[arg(long)]
        lender_signature: String,
    },
}

/// Everything one role keeps for one loan. The FVK seed is shared by all three roles.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShareBundle {
    pub role: Role,
    pub key_package: KeyPackage,
    pub public_key_package: PublicKeyPackage,
    #[serde(with = "hex::serde")]
    pub fvk_seed: [u8; 32],
    /// The DKG session on the policy signer (absent for `dkg-local` shares).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Chain height when the escrow key was made: scanning starts here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birthday: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    /// The policy signer's Base address, which must sign its attestations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_signer: Option<Address>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<ZcashNetwork>,
}

impl ShareBundle {
    fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    fn fvk(&self) -> Result<FullViewingKey> {
        Ok(fvk::derive_fvk(group_ak(&self.public_key_package)?, &self.fvk_seed)?)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::BaseKey { out } => net::new_base_key(&out),
        Command::Dkg(DkgCommand::Create { policy, borrower, lender }) => {
            net::dkg_create(&net::Policy::new(&policy), borrower, lender).await
        }
        Command::Dkg(DkgCommand::Join { policy, session, role, base_key, network, out, timeout_secs }) => {
            let key = net::load_base_key(&base_key)?;
            net::dkg_join(
                &net::Policy::new(&policy),
                &session,
                role,
                &key,
                network,
                &out,
                std::time::Duration::from_secs(timeout_secs),
            )
            .await
        }
        Command::Terms(TermsCommand::Approve { terms, salt, base_key }) => {
            net::terms_approve(&terms, salt, &net::load_base_key(&base_key)?)
        }
        Command::Terms(TermsCommand::Register {
            policy,
            terms,
            salt,
            session,
            borrower_signature,
            lender_signature,
        }) => {
            net::terms_register(
                &net::Policy::new(&policy),
                &terms,
                salt,
                &session,
                &borrower_signature,
                &lender_signature,
            )
            .await
        }
        Command::Cosign { policy, loan_id, kind, share, server, out_dir, no_broadcast } => {
            net::cosign(&net::Policy::new(&policy), loan_id, kind, &share, &server, &out_dir, no_broadcast).await
        }
        Command::DkgLocal { out } => dkg_local(&out),
        Command::Show { share, network } => show(&ShareBundle::load(&share)?, network),
        Command::Inspect { pczt } => inspect(&pczt),
        Command::SignLocal { pczt, shares, out } => sign_local_cmd(&pczt, &shares, &out),
        Command::TermsHash { terms, salt, loan_id } => {
            let terms: lentic_core::LoanTerms = serde_json::from_str(&std::fs::read_to_string(&terms)?)?;
            terms.validate().map_err(anyhow::Error::msg)?;
            let salt: lentic_core::terms::Salt = salt.parse().context("salt must be 32-byte hex")?;
            let hash = lentic_core::terms_hash(&terms, salt);
            if loan_id {
                println!("{}", lentic_core::loan_id(terms.lender_vault, hash));
            } else {
                println!("{hash}");
            }
            Ok(())
        }
        Command::Demo { collateral_zec, debt_usdc, price_usd } => demo(collateral_zec, debt_usdc, price_usd),
    }
}

fn dkg_local(out: &Path) -> Result<()> {
    let dkg = dkg::run_local(&mut OsRng)?;
    let seed = fvk::fvk_seed(&mut OsRng); // sampled by the borrower in the networked flow
    std::fs::create_dir_all(out)?;
    for (role, key) in dkg.keys {
        let bundle = ShareBundle {
            role,
            key_package: key,
            public_key_package: dkg.public.clone(),
            fvk_seed: seed,
            session: None,
            birthday: None,
            policy: None,
            policy_signer: None,
            network: None,
        };
        let path = out.join(format!("{role}.share.json"));
        std::fs::write(&path, serde_json::to_string_pretty(&bundle)?)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

pub(crate) fn show(bundle: &ShareBundle, network: ZcashNetwork) -> Result<()> {
    let params = network.params();
    let fvk = bundle.fvk()?;
    let out = serde_json::json!({
        "role": bundle.role,
        "network": network,
        "escrowAddress": keys::encode_unified_address(&params, keys::escrow_address(&fvk)),
        "escrowAddressR0": keys::encode_unified_address_r0(&params, keys::escrow_address(&fvk)),
        "ufvk": keys::encode_ufvk(&params, &fvk)?,
        "escrowFvkHash": escrow_fvk_hash(&fvk.to_bytes()),
        "fingerprint": fvk::fvk_fingerprint(&fvk),
        "session": bundle.session,
        "birthday": bundle.birthday,
        "policySigner": bundle.policy_signer,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

fn inspect(path: &Path) -> Result<()> {
    let pczt = escrow::parse(&std::fs::read(path)?)?;
    let pending: Vec<_> = escrow::spends_to_sign(&pczt)?
        .into_iter()
        .map(|s| serde_json::json!({ "index": s.index, "alpha": hex::encode(s.alpha), "value": s.value }))
        .collect();
    let payments: Vec<_> = escrow::payments(&pczt)?
        .into_iter()
        .map(|p| serde_json::json!({ "index": p.index, "recipient": hex::encode(p.recipient.to_raw_address_bytes()), "value": p.value }))
        .collect();
    let out = serde_json::json!({
        "sighash": hex::encode(escrow::shielded_sighash(&pczt)?),
        "pendingSpends": pending,
        "payments": payments,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

fn sign_local_cmd(pczt_path: &Path, share_paths: &[PathBuf], out: &Path) -> Result<()> {
    let bundles: Vec<ShareBundle> = share_paths.iter().map(|p| ShareBundle::load(p)).collect::<Result<_>>()?;
    if bundles[0].role == bundles[1].role {
        bail!("need shares of two different roles");
    }
    if bundles[0].public_key_package != bundles[1].public_key_package {
        bail!("shares belong to different loans");
    }
    let keys: BTreeMap<Role, KeyPackage> = bundles.iter().map(|b| (b.role, b.key_package.clone())).collect();
    let pczt = escrow::parse(&std::fs::read(pczt_path)?)?;
    let sigs = sign_locally(&pczt, &keys, &bundles[0].public_key_package, [bundles[0].role, bundles[1].role])?;
    let signed = escrow::apply_spend_auth_signatures(pczt, &sigs)?;
    std::fs::write(out, escrow::serialize(signed)?)?;
    println!("signed {} spend(s) → {}", sigs.len(), out.display());
    Ok(())
}

fn demo(collateral_zec: f64, debt_usdc: f64, price_usd: f64) -> Result<()> {
    let params = ZcashNetwork::Regtest.params();
    let collateral = (collateral_zec * 1e8).round() as u64;
    let debt = (debt_usdc * 1e6).round() as u128;
    let step = |n: u8, s: &str| println!("\n[{n}] {s}");

    step(1, "DKG: borrower, lender and policy signer create the loan's 2-of-3 group");
    let group = dkg::run_local(&mut OsRng)?;
    let fvk = fvk::derive_fvk(group_ak(&group.public)?, &fvk::fvk_seed(&mut OsRng))?;
    println!("    escrow address  {}", keys::encode_unified_address(&params, keys::escrow_address(&fvk)));
    println!("    escrowFvkHash   {}", escrow_fvk_hash(&fvk.to_bytes()));

    step(2, "Borrower deposits collateral into the escrow (simulated Ironwood chain)");
    let mut chain = SimChain::new();
    let (note, position) = chain.deposit(&fvk, collateral)?;
    chain.append_noise(3);
    let anchor = chain.checkpoint();
    println!("    {collateral_zec} ZEC note at position {}", u64::from(position));

    step(3, "Price drops; the policy signer sizes the liquidation with the live price");
    let lender = keys::escrow_address(&fvk::derive_fvk(group_ak(&dkg::run_local(&mut OsRng)?.public)?, &[1; 32])?);
    let borrower = keys::escrow_address(&fvk::derive_fvk(group_ak(&dkg::run_local(&mut OsRng)?.public)?, &[2; 32])?);
    let fee = zip317_fee(1, 2);
    let split = liquidation_split(collateral, fee, debt, Price::from_usd(price_usd), 500, 300)?;
    println!(
        "    lender {:.8} ZEC · borrower {:.8} ZEC · fee {} zat · shortfall {} µUSDC",
        split.lender_zat as f64 / 1e8,
        split.borrower_zat as f64 / 1e8,
        split.fee_zat,
        split.shortfall
    );

    step(4, "Lender daemon builds the PCZT; signers receive it without witness or anchor");
    let mut payments = vec![Payment::new(lender, split.lender_zat)];
    let mut expected = vec![(lender, split.lender_zat)];
    if split.borrower_zat > 0 {
        payments.push(Payment::new(borrower, split.borrower_zat));
        expected.push((borrower, split.borrower_zat));
    }
    let spend = escrow::build_escrow_spend(
        params,
        BlockHeight::from_u32(1_000),
        &fvk,
        anchor,
        vec![EscrowInput { note, merkle_path: chain.witness(position) }],
        &payments,
    )?;
    let signer_view = escrow::redact_witnesses(spend.pczt.clone(), &spend.spend_indices);
    escrow::check_payments(&signer_view, &expected)?;
    println!("    sighash {}", hex::encode(escrow::shielded_sighash(&signer_view)?));

    step(5, "Lender + policy signer produce the FROST signature (borrower absent)");
    let sigs = sign_locally(&signer_view, &group.keys, &group.public, [Role::Lender, Role::Policy])?;
    let signed = escrow::apply_spend_auth_signatures(spend.pczt, &sigs)?;

    step(6, "Chain moves on; lender refreshes anchor + witness after signing, proves, extracts");
    chain.append_noise(5);
    let fresh = chain.checkpoint();
    let refreshed = escrow::refresh_witnesses(signed, fresh, vec![(spend.spend_indices[0], chain.witness(position))])?;
    let started = std::time::Instant::now();
    let tx = escrow::extract(escrow::prove(refreshed)?)?;
    println!("    proved in {:.1}s · txid {} · {} bytes", started.elapsed().as_secs_f64(), tx.txid, tx.raw.len());
    println!("\nok: liquidation transaction is valid (signatures + Ironwood proof verified)");
    Ok(())
}
