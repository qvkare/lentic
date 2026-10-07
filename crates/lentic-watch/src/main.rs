//! `lentic-watch`: the chain-facing side of a loan.
//!
//!   info       chain info from one or more lightwalletd/Zaino servers (cross-checked)
//!   tree       Ironwood note commitment tree frontier at a height
//!   scan       notes, spends and balance of a viewing key (an escrow's UFVK)
//!   escrow     build: an escrow spend from the scanned notes (unsigned PCZT)
//!              finalize: fresh anchor and witnesses for a signed PCZT, proof, broadcast
//!   wallet     a single-key test wallet that funds escrows: new, show, balance, send
//!   prove, extract, broadcast   the finalize steps one by one

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use lentic_zcash::escrow::{self, Payment};
use lentic_zcash::fee::zip317_fee;
use lentic_zcash::network::{Params, ZcashNetwork};
use lentic_zcash::scan::Watcher;
use lentic_zcash::wallet::TestWallet;
use lentic_zcash::{keys, lightwalletd};
use serde_json::json;
use zcash_protocol::consensus::BlockHeight;

const TESTNET_SERVER: &str = "https://testnet.zec.rocks:443";

#[derive(Parser)]
#[command(name = "lentic-watch", version, about = "Lentic chain watcher, escrow spends and test wallet")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Where to read the chain from.
#[derive(clap::Args, Clone)]
struct Chain {
    #[arg(long, default_value = TESTNET_SERVER)]
    server: String,
    #[arg(long, default_value = "testnet")]
    network: ZcashNetwork,
}

#[derive(Subcommand)]
enum Command {
    /// Chain info from each server; reports whether they agree on the consensus branch.
    Info {
        #[arg(long, value_delimiter = ',', default_value = TESTNET_SERVER)]
        server: Vec<String>,
    },
    /// Ironwood tree state at a height (defaults to the tip).
    Tree {
        #[arg(long, default_value = TESTNET_SERVER)]
        server: String,
        #[arg(long)]
        height: Option<u64>,
    },
    /// Notes, spends and balance of a unified full viewing key from its birthday.
    Scan {
        #[arg(long)]
        ufvk: String,
        /// First block that may pay the key (the loan's key-generation height).
        #[arg(long)]
        birthday: u32,
        #[command(flatten)]
        chain: Chain,
    },
    #[command(subcommand)]
    Escrow(EscrowCommand),
    #[command(subcommand)]
    Wallet(WalletCommand),
    /// Prove a signed PCZT whose anchor and witnesses are set.
    Prove {
        #[arg(long)]
        pczt: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Extract the raw transaction from a proved PCZT (hex to stdout or --out).
    Extract {
        #[arg(long)]
        pczt: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Broadcast a raw transaction (hex file).
    Broadcast {
        #[arg(long, default_value = TESTNET_SERVER)]
        server: String,
        #[arg(long)]
        tx: PathBuf,
    },
}

#[derive(Subcommand)]
enum EscrowCommand {
    /// Build an unsigned spend of every confirmed escrow note.
    Build {
        #[arg(long)]
        ufvk: String,
        #[arg(long)]
        birthday: u32,
        /// A payment `<unified address>=<zatoshi>`; repeat for several.
        #[arg(long = "pay")]
        pays: Vec<String>,
        /// Pays everything left after the payments and the fee to this address.
        #[arg(long)]
        rest_to: Option<String>,
        /// Notes need this many confirmations to be spent.
        #[arg(long, default_value_t = 1)]
        confirmations: u32,
        #[arg(long)]
        out: PathBuf,
        #[command(flatten)]
        chain: Chain,
    },
    /// Refresh the anchor and witnesses of a signed PCZT, prove, extract and broadcast.
    Finalize {
        #[arg(long)]
        ufvk: String,
        #[arg(long)]
        birthday: u32,
        #[arg(long)]
        pczt: PathBuf,
        /// Also write the raw transaction (hex) here.
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        no_broadcast: bool,
        #[command(flatten)]
        chain: Chain,
    },
}

#[derive(Subcommand)]
enum WalletCommand {
    /// Create a test wallet (testnet or regtest only) whose birthday is the current tip.
    New {
        #[arg(long)]
        out: PathBuf,
        #[command(flatten)]
        chain: Chain,
    },
    /// Print the wallet's address and viewing key.
    Show {
        #[arg(long)]
        wallet: PathBuf,
    },
    /// Scan the wallet: notes and balance.
    Balance {
        #[arg(long)]
        wallet: PathBuf,
        #[arg(long, default_value = TESTNET_SERVER)]
        server: String,
    },
    /// Pay a unified address from the wallet; change returns to it.
    Send {
        #[arg(long)]
        wallet: PathBuf,
        #[arg(long)]
        to: String,
        #[arg(long)]
        zat: u64,
        #[arg(long, default_value_t = 1)]
        confirmations: u32,
        #[arg(long)]
        no_broadcast: bool,
        #[arg(long, default_value = TESTNET_SERVER)]
        server: String,
    },
}

/// A test wallet on disk. The key is stored in the clear: test networks only.
struct WalletFile {
    wallet: TestWallet,
    network: ZcashNetwork,
    birthday: u32,
}

impl WalletFile {
    fn load(path: &Path) -> Result<Self> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)
            .with_context(|| format!("parsing {}", path.display()))?;
        let field =
            |k: &str| v[k].as_str().map(str::to_owned).with_context(|| format!("{}: missing {k}", path.display()));
        let key: [u8; 32] =
            hex::decode(field("spendingKey")?)?.try_into().map_err(|_| anyhow::anyhow!("bad key length"))?;
        Ok(WalletFile {
            wallet: TestWallet::from_bytes(key)?,
            network: field("network")?.parse().map_err(anyhow::Error::msg)?,
            birthday: v["birthday"].as_u64().context("missing birthday")? as u32,
        })
    }

    fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = json!({
            "network": self.network,
            "birthday": self.birthday,
            "spendingKey": hex::encode(self.wallet.to_bytes()),
            "warning": "test wallet: the spending key is stored in the clear",
        });
        std::fs::write(path, serde_json::to_string_pretty(&body)?)?;
        Ok(())
    }

    fn describe(&self) -> Result<serde_json::Value> {
        let params = self.network.params();
        Ok(json!({
            "network": self.network,
            "birthday": self.birthday,
            "address": keys::encode_unified_address(&params, self.wallet.address()),
            "addressR0": keys::encode_unified_address_r0(&params, self.wallet.address()),
            "ufvk": keys::encode_ufvk(&params, &self.wallet.fvk())?,
        }))
    }
}

fn explorer(network: ZcashNetwork, txid: &str) -> Option<String> {
    (network == ZcashNetwork::Testnet).then(|| format!("https://testnet.zecblock.com/tx/{txid}"))
}

fn print(v: &serde_json::Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// Anchor at `tip` for one confirmation, one block lower for each extra one.
fn anchor_height(tip: u32, confirmations: u32) -> u32 {
    tip + 1 - confirmations.max(1)
}

async fn synced(server: &str, watcher: &mut Watcher) -> Result<(lightwalletd::Client, u32)> {
    let mut client = lightwalletd::connect(server).await?;
    let tip = watcher.sync(&mut client).await?;
    Ok((client, tip))
}

fn parse_payment(params: &Params, s: &str) -> Result<Payment> {
    let (ua, zat) = s.rsplit_once('=').context("a payment is <unified address>=<zatoshi>")?;
    Ok(Payment::new(keys::decode_orchard_receiver(params, ua)?, zat.parse().context("zatoshi")?))
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { server } => info(&server).await,
        Command::Tree { server, height } => {
            let mut c = lightwalletd::connect(&server).await?;
            let height = match height {
                Some(h) => h,
                None => lightwalletd::chain_info(&mut c).await?.block_height,
            };
            let tree = lightwalletd::ironwood_tree(&mut c, height).await?;
            print(&json!({ "height": height, "ironwoodTreeHexLen": tree.len(), "ironwoodTree": tree }))
        }
        Command::Scan { ufvk, birthday, chain } => {
            let fvk = keys::decode_ufvk(&chain.network.params(), &ufvk)?;
            let mut w = Watcher::new(fvk, birthday);
            let (mut client, tip) = synced(&chain.server, &mut w).await?;
            // The anchor computed from the scan must be the root the server reports for the tip.
            let root = lightwalletd::ironwood_root(&mut client, u64::from(tip)).await?;
            print(&json!({ "tip": tip, "anchorMatchesServer": w.anchor(tip)? == root, "scan": w.summary() }))
        }
        Command::Escrow(EscrowCommand::Build { ufvk, birthday, pays, rest_to, confirmations, out, chain }) => {
            let params = chain.network.params();
            let fvk = keys::decode_ufvk(&params, &ufvk)?;
            let mut w = Watcher::new(fvk.clone(), birthday);
            let (_, tip) = synced(&chain.server, &mut w).await?;
            let at = anchor_height(tip, confirmations);
            let (anchor, inputs) = w.spendable(at)?;
            if inputs.is_empty() {
                bail!("the escrow holds no note with {confirmations} confirmation(s) at height {tip}");
            }
            let total: u64 = inputs.iter().map(|i| i.note.value().inner()).sum();
            let mut payments = pays.iter().map(|p| parse_payment(&params, p)).collect::<Result<Vec<_>>>()?;
            if let Some(rest_to) = rest_to {
                let fee = zip317_fee(inputs.len(), payments.len() + 1);
                let paid: u64 = payments.iter().map(|p| p.value).sum();
                let rest = total.checked_sub(paid + fee).filter(|r| *r > 0).context("nothing left for --rest-to")?;
                payments.push(Payment::new(keys::decode_orchard_receiver(&params, &rest_to)?, rest));
            }
            let n_inputs = inputs.len();
            let spend =
                escrow::build_escrow_spend(params, BlockHeight::from_u32(tip + 1), &fvk, anchor, inputs, &payments)?;
            let sighash = escrow::shielded_sighash(&spend.pczt)?;
            let expiry = escrow::expiry_height(&spend.pczt);
            std::fs::write(&out, escrow::serialize(spend.pczt)?)?;
            print(&json!({
                "pczt": out,
                "inputs": n_inputs,
                "inputZat": total,
                "payments": payments.iter().map(|p| json!({
                    "address": keys::encode_unified_address(&params, p.recipient),
                    "zat": p.value,
                })).collect::<Vec<_>>(),
                "feeZat": spend.fee,
                "spendIndices": spend.spend_indices,
                "anchorHeight": at,
                "targetHeight": tip + 1,
                "expiryHeight": expiry,
                "sighash": hex::encode(sighash),
            }))
        }
        Command::Escrow(EscrowCommand::Finalize { ufvk, birthday, pczt, out, no_broadcast, chain }) => {
            let fvk = keys::decode_ufvk(&chain.network.params(), &ufvk)?;
            let signed = escrow::parse(&std::fs::read(&pczt)?)?;
            if !escrow::spends_to_sign(&signed)?.is_empty() {
                bail!("{} still has unsigned spends", pczt.display());
            }
            let mut w = Watcher::new(fvk, birthday);
            let (mut client, tip) = synced(&chain.server, &mut w).await?;
            let expiry = escrow::expiry_height(&signed);
            if tip + 1 > expiry {
                bail!("the transaction expired at height {expiry} (tip {tip}); build and sign it again");
            }
            let mut witnesses = Vec::new();
            for (index, nf) in escrow::spend_nullifiers(&signed)? {
                if let Some(note) = w.note_by_nullifier(&nf) {
                    if let Some(spent) = &note.spent {
                        bail!("escrow note {} was already spent in {}", u64::from(note.position), spent.txid);
                    }
                    witnesses.push((index, w.witness(note.position, tip)?));
                }
            }
            if witnesses.is_empty() {
                bail!("the PCZT spends none of this escrow's notes");
            }
            let refreshed = escrow::refresh_witnesses(signed, w.anchor(tip)?, witnesses)?;
            let tx = escrow::extract(escrow::prove(refreshed)?)?;
            send(&mut client, chain.network, tx, out.as_deref(), no_broadcast).await
        }
        Command::Wallet(cmd) => wallet(cmd).await,
        Command::Prove { pczt, out } => {
            let p = escrow::parse(&std::fs::read(&pczt)?)?;
            std::fs::write(&out, escrow::serialize(escrow::prove(p)?)?)?;
            println!("proved → {}", out.display());
            Ok(())
        }
        Command::Extract { pczt, out } => {
            let tx = escrow::extract(escrow::parse(&std::fs::read(&pczt)?)?)?;
            let hex = hex::encode(&tx.raw);
            match out {
                Some(path) => {
                    std::fs::write(&path, &hex)?;
                    println!("txid {} → {}", tx.txid, path.display());
                }
                None => println!("{hex}"),
            }
            Ok(())
        }
        Command::Broadcast { server, tx } => {
            let raw = hex::decode(std::fs::read_to_string(&tx)?.trim()).context("tx file must be hex")?;
            let mut c = lightwalletd::connect(&server).await?;
            lightwalletd::broadcast(&mut c, raw).await?;
            println!("broadcast ok");
            Ok(())
        }
    }
}

async fn send(
    client: &mut lightwalletd::Client,
    network: ZcashNetwork,
    tx: escrow::ExtractedTx,
    out: Option<&Path>,
    no_broadcast: bool,
) -> Result<()> {
    if let Some(path) = out {
        std::fs::write(path, hex::encode(&tx.raw))?;
    }
    if !no_broadcast {
        lightwalletd::broadcast(client, tx.raw.clone()).await?;
    }
    print(&json!({
        "txid": tx.txid,
        "broadcast": !no_broadcast,
        "expiryHeight": tx.expiry_height,
        "bytes": tx.raw.len(),
        "explorer": explorer(network, &tx.txid),
    }))
}

async fn info(servers: &[String]) -> Result<()> {
    let mut results = Vec::new();
    for url in servers {
        let info = async {
            let mut c = lightwalletd::connect(url).await?;
            lightwalletd::chain_info(&mut c).await
        }
        .await;
        results.push(match info {
            Ok(i) => json!({ "url": url, "info": i }),
            Err(e) => json!({ "url": url, "error": e.to_string() }),
        });
    }
    let branches: std::collections::BTreeSet<_> =
        results.iter().filter_map(|r| r["info"]["consensusBranchId"].as_str()).collect();
    print(&json!({ "servers": results, "branchesAgree": branches.len() <= 1 }))
}

async fn wallet(cmd: WalletCommand) -> Result<()> {
    match cmd {
        WalletCommand::New { out, chain } => {
            if chain.network == ZcashNetwork::Mainnet {
                bail!("test wallets keep their key in the clear; use them on testnet or regtest only");
            }
            if out.exists() {
                bail!("{} exists; not overwriting a wallet", out.display());
            }
            let mut client = lightwalletd::connect(&chain.server).await?;
            let tip = u32::try_from(lightwalletd::chain_info(&mut client).await?.block_height)?;
            let file = WalletFile { wallet: TestWallet::generate(), network: chain.network, birthday: tip };
            file.save(&out)?;
            print(&file.describe()?)
        }
        WalletCommand::Show { wallet } => print(&WalletFile::load(&wallet)?.describe()?),
        WalletCommand::Balance { wallet, server } => {
            let file = WalletFile::load(&wallet)?;
            let mut w = Watcher::new(file.wallet.fvk(), file.birthday);
            let (_, tip) = synced(&server, &mut w).await?;
            print(&json!({ "tip": tip, "wallet": file.describe()?, "scan": w.summary() }))
        }
        WalletCommand::Send { wallet, to, zat, confirmations, no_broadcast, server } => {
            let file = WalletFile::load(&wallet)?;
            let params = file.network.params();
            let recipient = keys::decode_orchard_receiver(&params, &to)?;
            let mut w = Watcher::new(file.wallet.fvk(), file.birthday);
            let (mut client, tip) = synced(&server, &mut w).await?;
            let (anchor, inputs) = w.spendable(anchor_height(tip, confirmations))?;
            if inputs.is_empty() {
                bail!(
                    "the wallet has no note with {confirmations} confirmation(s); fund {}",
                    file.describe()?["address"]
                );
            }
            let spend = escrow::build_spend(
                params,
                BlockHeight::from_u32(tip + 1),
                &file.wallet.fvk(),
                anchor,
                inputs,
                &[Payment::new(recipient, zat)],
                Some(file.wallet.change_address()),
            )?;
            let tx = escrow::extract(escrow::prove(file.wallet.sign(spend.pczt)?)?)?;
            send(&mut client, file.network, tx, None, no_broadcast).await
        }
    }
}
