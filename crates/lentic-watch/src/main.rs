//! `lentic-watch`: the lender's (and borrower's) chain-facing daemon.
//!
//! Infrastructure commands (UFVK scanning into a wallet DB comes next):
//!   info       chain info from one or more lightwalletd/Zaino servers (cross-checked)
//!   tree       Ironwood note commitment tree frontier at a height
//!   prove      create the Ironwood proof for a signed PCZT (no spend key needed)
//!   extract    turn a proved PCZT into a raw transaction
//!   broadcast  send a raw transaction

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lentic_zcash::{escrow, lightwalletd};

#[derive(Parser)]
#[command(name = "lentic-watch", version, about = "Lentic chain watcher and broadcaster")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Chain info from each server; reports whether they agree on the consensus branch.
    Info {
        #[arg(long, value_delimiter = ',', default_value = "https://testnet.zec.rocks:443")]
        server: Vec<String>,
    },
    /// Ironwood tree state at a height (defaults to the tip).
    Tree {
        #[arg(long, default_value = "https://testnet.zec.rocks:443")]
        server: String,
        #[arg(long)]
        height: Option<u64>,
    },
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
        #[arg(long, default_value = "https://testnet.zec.rocks:443")]
        server: String,
        #[arg(long)]
        tx: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Info { server } => {
            let mut results = Vec::new();
            for url in &server {
                let info = async {
                    let mut c = lightwalletd::connect(url).await?;
                    lightwalletd::chain_info(&mut c).await
                }
                .await;
                results.push(match info {
                    Ok(i) => serde_json::json!({ "url": url, "info": i }),
                    Err(e) => serde_json::json!({ "url": url, "error": e.to_string() }),
                });
            }
            let branches: std::collections::BTreeSet<_> =
                results.iter().filter_map(|r| r["info"]["consensusBranchId"].as_str()).collect();
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "servers": results, "branchesAgree": branches.len() <= 1 })
                )?
            );
        }
        Command::Tree { server, height } => {
            let mut c = lightwalletd::connect(&server).await?;
            let height = match height {
                Some(h) => h,
                None => lightwalletd::chain_info(&mut c).await?.block_height,
            };
            let tree = lightwalletd::ironwood_tree(&mut c, height).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "height": height,
                    "ironwoodTreeHexLen": tree.len(),
                    "ironwoodTree": tree,
                }))?
            );
        }
        Command::Prove { pczt, out } => {
            let p = escrow::parse(&std::fs::read(&pczt)?)?;
            std::fs::write(&out, escrow::serialize(escrow::prove(p)?)?)?;
            println!("proved → {}", out.display());
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
        }
        Command::Broadcast { server, tx } => {
            let raw = hex::decode(std::fs::read_to_string(&tx)?.trim()).context("tx file must be hex")?;
            let mut c = lightwalletd::connect(&server).await?;
            lightwalletd::broadcast(&mut c, raw).await?;
            println!("broadcast ok");
        }
    }
    Ok(())
}
