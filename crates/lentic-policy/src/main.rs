//! `lentic-policy`: the policy signer service, the only key share Lentic operates (1 of 3).
//!
//! Runs inside a dstack/TDX CVM in production (`--attestor dstack`, key from the dstack KMS)
//! and locally against anvil in development (`--attestor dev`, anvil account #3).

mod api;
mod attest;
mod book;
mod cosign;
mod dkg;
mod price;
mod store;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use alloy::network::EthereumWallet;
use alloy::primitives::B256;
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use lentic_base::Deployment;
use lentic_zcash::network::ZcashNetwork;
use tracing::info;

#[derive(Parser)]
#[command(name = "lentic-policy", version, about = "Lentic policy signer (TEE-held third FROST share)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

// Parsed once at startup; boxing the larger variant would buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum Command {
    /// Run the policy signer HTTP API.
    Serve(ServeArgs),
    /// Print the measurement AttestationVerifier must be deployed with for this attestor.
    Measurement {
        #[arg(long, value_enum, default_value_t = AttestorKind::Dev)]
        attestor: AttestorKind,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum AttestorKind {
    Dev,
    Dstack,
}

#[derive(Parser)]
struct ServeArgs {
    #[arg(long, env = "LENTIC_LISTEN", default_value = "127.0.0.1:8787")]
    listen: SocketAddr,
    #[arg(long, env = "BASE_RPC_URL", default_value = "http://127.0.0.1:8546")]
    base_rpc: String,
    /// contracts/deployments/<chainId>.json
    #[arg(long, env = "LENTIC_DEPLOYMENT", default_value = "contracts/deployments/31337.json")]
    deployment: PathBuf,
    #[arg(long, env = "LENTIC_ATTESTOR", value_enum, default_value_t = AttestorKind::Dev)]
    attestor: AttestorKind,
    /// Dev only: the policy signer's Base key (default: anvil account #3).
    #[arg(
        long,
        env = "POLICY_ETH_KEY",
        default_value = "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6"
    )]
    dev_key: String,
    #[arg(long, env = "LENTIC_CONFIG_ID", default_value = attest::DEV_CONFIG_ID)]
    config_id: String,
    #[arg(long, env = "ZCASH_NETWORK", default_value = "testnet")]
    zcash_network: ZcashNetwork,
    /// lightwalletd / Zaino endpoints (repeat or comma-separate; at least two in production).
    #[arg(long, env = "LIGHTWALLETD", value_delimiter = ',', default_value = "https://testnet.zec.rocks:443")]
    lightwalletd: Vec<String>,
    /// Enable state-changing operator routes (local development only).
    #[arg(long, env = "LENTIC_DEV_ROUTES", default_value_t = false)]
    dev_routes: bool,
    /// Directory of held loans (`<loanId>.json` with loanId, salt, terms and DKG session).
    #[arg(long, env = "LENTIC_LOANS_DIR", default_value = "dev/loans")]
    loans_dir: PathBuf,
    /// The signer's escrow key shares and co-signing record (an encrypted volume in a CVM).
    #[arg(long, env = "LENTIC_STATE_DIR", default_value = "dev/policy")]
    state_dir: PathBuf,
    /// Seconds between rounds (price reading + every held loan).
    #[arg(long, env = "LENTIC_ROUND_SECS", default_value_t = 15)]
    round_secs: u64,
    /// Write health changes to LoanBook from the signer key.
    #[arg(long, env = "LENTIC_REPORT_HEALTH", default_value_t = false)]
    report_health: bool,
    /// Fixed ZEC/USD instead of the exchange median (offline development).
    #[arg(long, env = "LENTIC_PRICE_USD")]
    price_usd: Option<f64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,lentic_policy=debug".into()),
        )
        .init();

    match Cli::parse().command {
        Command::Measurement { attestor } => {
            let quote = match attestor {
                AttestorKind::Dev => attest::dev_quote(&[0; 64]),
                AttestorKind::Dstack => {
                    let client = dstack_sdk::dstack_client::DstackClient::new(None);
                    attest::Attestor::Dstack(client).quote(&[0; 64]).await?
                }
            };
            println!("{}", attest::measurement(&quote)?);
            Ok(())
        }
        Command::Serve(args) => serve(args).await,
    }
}

async fn serve(args: ServeArgs) -> Result<()> {
    let (signer, attestor) = match args.attestor {
        AttestorKind::Dev => {
            (args.dev_key.parse::<PrivateKeySigner>().context("POLICY_ETH_KEY")?, attest::Attestor::Dev)
        }
        AttestorKind::Dstack => {
            let client = dstack_sdk::dstack_client::DstackClient::new(None);
            // Deterministic per app (compose hash) via the dstack KMS: a restarted CVM with the
            // same code gets the same key, nothing outside the TEE ever sees it.
            let key = client
                .get_key(Some("lentic/policy/base".into()), Some("signing".into()))
                .await
                .context("dstack GetKey")?;
            let bytes = key.decode_key().context("dstack key hex")?;
            let signer = PrivateKeySigner::from_bytes(&B256::from_slice(&bytes[..32]))?;
            (signer, attest::Attestor::Dstack(client))
        }
    };
    let deployment = Deployment::load(&args.deployment)?;
    let provider: DynProvider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer.clone()))
        .connect_http(args.base_rpc.parse().context("BASE_RPC_URL")?)
        .erased();
    let chain_id = provider.get_chain_id().await.context("Base RPC unreachable")?;
    anyhow::ensure!(
        chain_id == deployment.chain_id,
        "RPC chain {chain_id} does not match deployment chain {}",
        deployment.chain_id
    );

    let loans = book::load_dir(&args.loans_dir)?;
    let store = store::Store::open(&args.state_dir)?;
    let shares: Vec<dkg::PolicyShare> = store.all("shares")?;
    let intents: Vec<cosign::Intent> = store.all("intents")?;
    let feed = match args.price_usd {
        Some(usd) => price::Feed::Fixed(usd),
        None => price::Feed::exchanges(),
    };
    let (n_loans, n_shares) = (loans.len(), shares.len());
    let state = Arc::new(api::AppState {
        signer_address: signer.address(),
        signer,
        config_hash: attest::config_hash(&args.config_id),
        attestor,
        provider,
        deployment,
        zcash_network: args.zcash_network,
        lightwalletd: args.lightwalletd,
        dev_routes: args.dev_routes,
        feed,
        book: book::Book::new(loans),
        report_health: args.report_health,
        round_secs: args.round_secs.max(1),
        loans_dir: args.loans_dir,
        store,
        dkg: dkg::Dkg::with_shares(shares),
        cosign: cosign::Cosign::with_intents(intents),
        zcash: Default::default(),
    });
    info!(
        signer = %state.signer_address,
        attestor = state.attestor.kind(),
        chain_id,
        loans = n_loans,
        escrow_keys = n_shares,
        "policy signer starting"
    );

    let rounds = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(rounds.round_secs));
        loop {
            tick.tick().await;
            rounds.book.round(&rounds).await;
        }
    });

    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    info!("listening on http://{}", args.listen);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
