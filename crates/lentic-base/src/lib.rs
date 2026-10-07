//! Base side of a Lentic loan: typed contract bindings (generated from `contracts/abi/*.json`,
//! refreshed by `scripts/export-abis.mjs`) and the reads/writes the services need.

use std::path::Path;

use alloy::primitives::{Address, B256, U256};
use alloy::providers::Provider;
use alloy::rpc::types::Filter;
use alloy::sol;
use alloy::sol_types::SolEvent;
use anyhow::{Context, Result};
use lentic_core::Health;
use serde::{Deserialize, Serialize};

sol!(
    #[sol(rpc)]
    LoanBook,
    "../../contracts/abi/LoanBook.json"
);
sol!(
    #[sol(rpc)]
    LenderVault,
    "../../contracts/abi/LenderVault.json"
);
sol!(
    #[sol(rpc)]
    AttestationVerifier,
    "../../contracts/abi/AttestationVerifier.json"
);
sol!(
    #[sol(rpc)]
    MockUSDC,
    "../../contracts/abi/MockUSDC.json"
);

/// `contracts/deployments/<chainId>.json` as written by the deploy scripts.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    pub chain_id: u64,
    #[serde(default)]
    pub start_block: u64,
    pub loan_book: Address,
    pub attestation_verifier: Address,
    #[serde(default)]
    pub lender_vault: Option<Address>,
    #[serde(default)]
    pub usdc: Option<Address>,
    #[serde(default)]
    pub policy_signer: Option<Address>,
    #[serde(default)]
    pub lender: Option<Address>,
    #[serde(default)]
    pub borrower: Option<Address>,
    #[serde(default)]
    pub dev_config_hash: Option<B256>,
}

impl Deployment {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }
}

/// A loan as recorded in `LoanBook`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoanView {
    pub loan_id: B256,
    pub terms_hash: B256,
    pub vault: Address,
    pub health: Health,
    pub opened_at: u64,
    pub updated_at: u64,
    pub oracle_round: u64,
    pub attestation_hash: B256,
}

pub async fn read_loan<P: Provider>(provider: &P, loan_book: Address, loan_id: B256) -> Result<Option<LoanView>> {
    let r = LoanBook::new(loan_book, provider).loans(loan_id).call().await?;
    let health = Health::from_u8(r.health).context("unknown health value")?;
    if health == Health::None {
        return Ok(None);
    }
    Ok(Some(LoanView {
        loan_id,
        terms_hash: r.termsHash,
        vault: r.vault,
        health,
        opened_at: r.openedAt,
        updated_at: r.updatedAt,
        oracle_round: r.oracleRound,
        attestation_hash: r.attestationHash,
    }))
}

/// Every loan opened since `from_block`, oldest first (from `LoanOpened` events).
pub async fn list_loan_ids<P: Provider>(provider: &P, loan_book: Address, from_block: u64) -> Result<Vec<B256>> {
    let filter =
        Filter::new().address(loan_book).event_signature(LoanBook::LoanOpened::SIGNATURE_HASH).from_block(from_block);
    let logs = provider.get_logs(&filter).await?;
    logs.into_iter().map(|log| Ok(log.log_decode::<LoanBook::LoanOpened>()?.inner.data.loanId)).collect()
}

/// Reports a loan's health (only an attested policy signer's transaction succeeds).
pub async fn update_health<P: Provider>(
    provider: &P,
    loan_book: Address,
    loan_id: B256,
    health: Health,
    oracle_round: u64,
    attestation_hash: B256,
) -> Result<B256> {
    let receipt = LoanBook::new(loan_book, provider)
        .updateHealth(loan_id, health as u8, oracle_round, attestation_hash)
        .send()
        .await?
        .with_timeout(Some(std::time::Duration::from_secs(60)))
        .get_receipt()
        .await?;
    anyhow::ensure!(receipt.status(), "updateHealth reverted");
    Ok(receipt.transaction_hash)
}

pub async fn is_policy_signer<P: Provider>(provider: &P, verifier: Address, account: Address) -> Result<bool> {
    Ok(AttestationVerifier::new(verifier, provider).isPolicySigner(account).call().await?)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSummary {
    pub address: Address,
    pub owner: Address,
    pub total_assets: U256,
    pub idle: U256,
    pub total_outstanding: U256,
    pub fee_bps: u16,
}

pub async fn vault_summary<P: Provider>(provider: &P, vault: Address) -> Result<VaultSummary> {
    let v = LenderVault::new(vault, provider);
    Ok(VaultSummary {
        address: vault,
        owner: v.owner().call().await?,
        total_assets: v.totalAssets().call().await?,
        idle: v.idle().call().await?,
        total_outstanding: v.totalOutstanding().call().await?,
        fee_bps: v.feeBps().call().await?,
    })
}

/// A loan's repayment record in its LenderVault (public on Base: these are USDC transfers).
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultPosition {
    pub principal: u128,
    pub principal_repaid: u128,
    pub interest_paid: u128,
    pub written_off: bool,
}

/// A loan's repayments into its vault (`LoanRepaid` events), with their block times.
pub async fn repayments<P: Provider>(
    provider: &P,
    vault: Address,
    loan_id: B256,
    from_block: u64,
) -> Result<Vec<lentic_core::Repayment>> {
    let filter = Filter::new()
        .address(vault)
        .event_signature(LenderVault::LoanRepaid::SIGNATURE_HASH)
        .topic1(loan_id)
        .from_block(from_block);
    let mut out = Vec::new();
    for log in provider.get_logs(&filter).await? {
        let event = log.log_decode::<LenderVault::LoanRepaid>()?.inner.data;
        let at = match log.block_timestamp {
            Some(t) => t,
            None => {
                let number = log.block_number.context("log without a block number")?;
                provider
                    .get_block_by_number(number.into())
                    .await?
                    .context("block of a repayment not found")?
                    .header
                    .timestamp
            }
        };
        let amount = event.principalPart + event.interestPart;
        out.push(lentic_core::Repayment { at, amount: amount.try_into().context("repayment exceeds u128")? });
    }
    Ok(out)
}

pub async fn vault_position<P: Provider>(provider: &P, vault: Address, loan_id: B256) -> Result<VaultPosition> {
    let p = LenderVault::new(vault, provider).positions(loan_id).call().await?;
    Ok(VaultPosition {
        principal: p.principal,
        principal_repaid: p.principalRepaid,
        interest_paid: p.interestPaid,
        written_off: p.writtenOff,
    })
}
