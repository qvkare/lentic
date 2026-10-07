//! HTTP API of the policy signer.
//!
//! Read-only:  /healthz, /v1/info, /v1/attestation, /v1/price, /v1/loans, /v1/loans/{id}, /v1/zcash
//! Key making: POST /v1/dkg, GET /v1/dkg/{id}, POST /v1/dkg/{id}/round1|round2|done (the DKG
//!             relay; it only ever carries signed or sealed packages)
//! Loans:      POST /v1/loans (terms approved by borrower and lender, bound to a DKG session)
//! Parties:    /v1/loans/{id}/gauge, /v1/loans/{id}/escrow (LTV, debt and the escrow's notes;
//!             in production these sit behind party authentication, since they are private
//!             to the loan's parties)
//! Co-signing: GET /v1/loans/{id}/plan/{kind}, POST /v1/loans/{id}/cosign,
//!             POST /v1/cosign/{session}/shares
//! Pure:       POST /v1/assess, POST /v1/liquidation-plan
//! Dev only:   POST /v1/loans/{id}/health, POST /v1/dev/price

use std::path::PathBuf;
use std::sync::Arc;

use alloy::primitives::{Address, B256};
use alloy::providers::DynProvider;
use alloy::signers::local::PrivateKeySigner;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lentic_base::{Deployment, LoanView};
use lentic_core::math::{self, LiquidationSplit, Price};
use lentic_core::terms::approval_message;
use lentic_core::{Health, LoanTerms, Role, terms_hash};
use lentic_zcash::keys;
use lentic_zcash::lightwalletd::{self, Client};
use lentic_zcash::network::ZcashNetwork;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;

use crate::attest::{self, Attestor};
use crate::book::{self, Book, HeldLoan, LoanState, Reading};
use crate::cosign::{self, Cosign, Intent};
use crate::dkg::{self, Dkg};
use crate::price::{Feed, PriceReading};
use crate::store::Store;

pub struct AppState {
    pub signer: PrivateKeySigner,
    pub signer_address: Address,
    pub config_hash: B256,
    pub attestor: Attestor,
    pub provider: DynProvider,
    pub deployment: Deployment,
    pub zcash_network: ZcashNetwork,
    pub lightwalletd: Vec<String>,
    pub dev_routes: bool,
    pub feed: Feed,
    pub book: Book,
    pub report_health: bool,
    pub round_secs: u64,
    pub loans_dir: PathBuf,
    pub store: Store,
    pub dkg: Dkg,
    pub cosign: Cosign,
    /// The lightwalletd connection used for scanning (the first configured server).
    pub zcash: Mutex<Option<Client>>,
}

impl AppState {
    pub async fn zcash_client(&self) -> anyhow::Result<Client> {
        let mut slot = self.zcash.lock().await;
        if slot.is_none() {
            let url = self.lightwalletd.first().ok_or_else(|| anyhow::anyhow!("no lightwalletd configured"))?;
            *slot = Some(lightwalletd::connect(url).await?);
        }
        Ok(slot.clone().expect("connected above"))
    }

    pub async fn drop_zcash_client(&self) {
        *self.zcash.lock().await = None;
    }

    pub async fn zcash_tip(&self) -> anyhow::Result<u32> {
        let mut client = self.zcash_client().await?;
        Ok(u32::try_from(lightwalletd::chain_info(&mut client).await?.block_height)?)
    }
}

pub type Shared = Arc<AppState>;

pub fn router(state: Shared) -> Router {
    let mut r = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/info", get(info))
        .route("/v1/attestation", get(attestation))
        .route("/v1/loans", get(loans).post(register))
        .route("/v1/loans/{id}", get(loan))
        .route("/v1/zcash", get(zcash))
        .route("/v1/price", get(price))
        .route("/v1/loans/{id}/gauge", get(gauge))
        .route("/v1/loans/{id}/escrow", get(escrow))
        .route("/v1/loans/{id}/plan/{kind}", get(cosign::plan))
        .route("/v1/loans/{id}/cosign", post(cosign::start))
        .route("/v1/cosign/{session}/shares", post(cosign::shares))
        .route("/v1/dkg", post(dkg::create))
        .route("/v1/dkg/{id}", get(dkg::view))
        .route("/v1/dkg/{id}/round1", post(dkg::round1))
        .route("/v1/dkg/{id}/round2", post(dkg::round2))
        .route("/v1/dkg/{id}/done", post(dkg::done))
        .route("/v1/assess", post(assess))
        .route("/v1/liquidation-plan", post(liquidation_plan));
    if state.dev_routes {
        r = r.route("/v1/loans/{id}/health", post(set_health)).route("/v1/dev/price", post(set_test_price));
    }
    r.layer(CorsLayer::permissive()).with_state(state)
}

pub struct ApiError(pub StatusCode, pub String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::BAD_GATEWAY, format!("{e:#}"))
    }
}

pub fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

pub type ApiResult<T> = Result<Json<T>, ApiError>;

async fn info(State(s): State<Shared>) -> ApiResult<Value> {
    let registered =
        lentic_base::is_policy_signer(&s.provider, s.deployment.attestation_verifier, s.signer_address).await?;
    let quote = s.attestor.quote(&attest::report_data(s.signer_address, s.config_hash)).await?;
    Ok(Json(json!({
        "service": "lentic-policy",
        "version": env!("CARGO_PKG_VERSION"),
        "signerAddress": s.signer_address,
        "configHash": s.config_hash,
        "attestor": s.attestor.kind(),
        "measurement": attest::measurement(&quote)?,
        "registeredOnBase": registered,
        "baseChainId": s.deployment.chain_id,
        "loanBook": s.deployment.loan_book,
        "attestationVerifier": s.deployment.attestation_verifier,
        "zcashNetwork": s.zcash_network,
        "devRoutes": s.dev_routes,
        "loansHeld": s.book.loans().await.len(),
        "escrowKeysHeld": s.dkg.share_count().await,
        "roundSecs": s.round_secs,
        "reportsHealth": s.report_health,
        "priceKind": s.book.latest_price().await.map(|p| p.kind),
    })))
}

async fn price(State(s): State<Shared>) -> ApiResult<PriceReading> {
    s.book.latest_price().await.map(Json).ok_or(ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "No price reading yet. The first round is still running.".into(),
    ))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gauge {
    pub loan_id: B256,
    pub action_stage_bps: u32,
    pub flood_stage_bps: u32,
    /// Liquidation terms, so a party can work out the split at any price.
    pub haircut_bps: u32,
    pub penalty_bps: u32,
    /// "escrow" when the signer reads the escrow itself, "terms" for terms-only loans.
    pub collateral_source: &'static str,
    pub round_secs: u64,
    pub readings: Vec<Reading>,
    pub state: Option<LoanState>,
    pub intent: Option<Intent>,
    pub last_error: Option<String>,
}

async fn held_or_404(s: &Shared, id: &B256) -> Result<HeldLoan, ApiError> {
    s.book
        .held(id)
        .await
        .ok_or(ApiError(StatusCode::NOT_FOUND, format!("This policy signer holds no terms for loan {id}.")))
}

async fn gauge(State(s): State<Shared>, Path(id): Path<B256>) -> ApiResult<Gauge> {
    let held = held_or_404(&s, &id).await?;
    Ok(Json(Gauge {
        loan_id: id,
        action_stage_bps: held.terms.margin_ltv_bps,
        flood_stage_bps: held.terms.liquidation_ltv_bps,
        haircut_bps: held.terms.haircut_bps,
        penalty_bps: held.terms.penalty_bps,
        collateral_source: if held.dkg_session.is_some() { "escrow" } else { "terms" },
        round_secs: s.round_secs,
        readings: s.book.readings(&id).await,
        state: s.book.state(&id).await,
        intent: s.cosign.intent(&id).await,
        last_error: s.book.last_error().await,
    }))
}

/// The escrow as the signer sees it: address, notes, spends.
async fn escrow(State(s): State<Shared>, Path(id): Path<B256>) -> ApiResult<Value> {
    let held = held_or_404(&s, &id).await?;
    let session =
        held.dkg_session.as_deref().ok_or_else(|| bad_request("this loan has no escrow held by the signer"))?;
    let share = s.dkg.share(session).await.ok_or_else(|| bad_request("the escrow key is missing"))?;
    let params = s.zcash_network.params();
    let addr = keys::escrow_address(&share.fvk()?);
    let (tip, _) = s.book.escrow_notes(&s, &held).await?;
    let scan = s.book.watchers.lock().await.get(&id).map(|w| w.summary());
    Ok(Json(json!({
        "loanId": id,
        "escrowAddress": keys::encode_unified_address(&params, addr),
        "escrowAddressR0": keys::encode_unified_address_r0(&params, addr),
        "escrowFvkHash": share.escrow_fvk_hash,
        "birthday": share.birthday,
        "tip": tip,
        "scan": scan,
        "state": s.book.state(&id).await,
        "intent": s.cosign.intent(&id).await,
    })))
}

#[derive(Deserialize)]
pub struct TestPrice {
    /// USD per ZEC; null returns to the exchange feed.
    pub usd: Option<f64>,
}

async fn set_test_price(State(s): State<Shared>, Json(req): Json<TestPrice>) -> ApiResult<PriceReading> {
    if let Some(usd) = req.usd
        && !(usd.is_finite() && usd > 0.0)
    {
        return Err(bad_request("The test price must be a positive number of USD."));
    }
    s.book.set_test_price(req.usd).await;
    s.book.round(&s).await;
    price(State(s)).await
}

async fn attestation(State(s): State<Shared>) -> ApiResult<Value> {
    let rd = attest::report_data(s.signer_address, s.config_hash);
    let quote = s.attestor.quote(&rd).await?;
    Ok(Json(json!({
        "signerAddress": s.signer_address,
        "reportData": format!("0x{}", hex::encode(rd)),
        "measurement": attest::measurement(&quote)?,
        "quote": format!("0x{}", hex::encode(&quote)),
        "submit": "AttestationVerifier.register(quote)",
    })))
}

async fn loans(State(s): State<Shared>) -> ApiResult<Vec<LoanView>> {
    let ids = lentic_base::list_loan_ids(&s.provider, s.deployment.loan_book, s.deployment.start_block).await?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(view) = lentic_base::read_loan(&s.provider, s.deployment.loan_book, id).await? {
            out.push(view);
        }
    }
    Ok(Json(out))
}

async fn loan(State(s): State<Shared>, Path(id): Path<B256>) -> ApiResult<LoanView> {
    lentic_base::read_loan(&s.provider, s.deployment.loan_book, id)
        .await?
        .map(Json)
        .ok_or(ApiError(StatusCode::NOT_FOUND, format!("unknown loan {id}")))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Register {
    pub loan_id: B256,
    pub salt: B256,
    pub terms: LoanTerms,
    /// The DKG session that made the escrow key the terms commit to.
    pub dkg_session: String,
    /// EIP-191 signatures over `approval_message(loanId, termsHash)`.
    #[serde(with = "hex::serde")]
    pub borrower_signature: Vec<u8>,
    #[serde(with = "hex::serde")]
    pub lender_signature: Vec<u8>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Registered {
    pub loan_id: B256,
    pub terms_hash: B256,
    pub escrow_address: String,
    pub escrow_address_r0: String,
    pub birthday: u32,
}

/// Takes a loan into the signer's book: terms both parties signed, bound to an escrow key the
/// signer helped make.
async fn register(State(s): State<Shared>, Json(req): Json<Register>) -> ApiResult<Registered> {
    let terms = &req.terms;
    terms.validate().map_err(bad_request)?;
    if terms.chain_id != s.deployment.chain_id {
        return Err(bad_request(format!(
            "terms are for chain {}, this signer serves {}",
            terms.chain_id, s.deployment.chain_id
        )));
    }
    let share = s.dkg.share(&req.dkg_session).await.ok_or_else(|| bad_request("unknown or unfinished DKG session"))?;
    if terms.escrow_fvk_hash != share.escrow_fvk_hash {
        return Err(bad_request("the terms commit to a different escrow key than this DKG session made"));
    }
    let borrower = share.identities[&Role::Borrower];
    let lender = share.identities[&Role::Lender];
    if terms.borrower != borrower {
        return Err(bad_request(format!("the terms pay {}, the DKG session's borrower is {borrower}", terms.borrower)));
    }
    let hash = terms_hash(terms, req.salt);
    let message = approval_message(req.loan_id, hash);
    let recover =
        |sig: &[u8]| lentic_frost::relay::recover_signer(&message, sig).map_err(|e| bad_request(e.to_string()));
    if recover(&req.borrower_signature)? != borrower {
        return Err(bad_request("the borrower's approval is not signed by the borrower"));
    }
    if recover(&req.lender_signature)? != lender {
        return Err(bad_request("the lender's approval is not signed by the lender"));
    }
    let owner = lentic_base::vault_owner(&s.provider, terms.lender_vault).await?;
    if owner != lender {
        return Err(bad_request(format!(
            "vault {} is owned by {owner}, not the DKG session's lender {lender}",
            terms.lender_vault
        )));
    }
    let params = s.zcash_network.params();
    for (who, ua) in
        [("borrower return", &terms.borrower_return_address), ("lender payout", &terms.lender_zcash_address)]
    {
        keys::decode_orchard_receiver(&params, ua).map_err(|e| bad_request(format!("{who} address: {e}")))?;
    }

    let held =
        HeldLoan { loan_id: req.loan_id, salt: req.salt, terms: req.terms.clone(), dkg_session: Some(req.dkg_session) };
    if !s.book.hold(held.clone()).await {
        return Err(ApiError(StatusCode::CONFLICT, format!("loan {} is already held", req.loan_id)));
    }
    book::save(&s.loans_dir, &held)?;
    let addr = keys::escrow_address(&share.fvk()?);
    tracing::info!(loan = %req.loan_id, %hash, "loan registered");
    Ok(Json(Registered {
        loan_id: req.loan_id,
        terms_hash: hash,
        escrow_address: keys::encode_unified_address(&params, addr),
        escrow_address_r0: keys::encode_unified_address_r0(&params, addr),
        birthday: share.birthday,
    }))
}

/// Chain info from every configured lightwalletd, and whether they agree.
async fn zcash(State(s): State<Shared>) -> ApiResult<Value> {
    let mut servers = Vec::new();
    for url in &s.lightwalletd {
        let result = async {
            let mut client = lightwalletd::connect(url).await?;
            lightwalletd::chain_info(&mut client).await
        }
        .await;
        servers.push(match result {
            Ok(info) => json!({ "url": url, "ok": true, "info": info }),
            Err(e) => json!({ "url": url, "ok": false, "error": e.to_string() }),
        });
    }
    let branches: std::collections::BTreeSet<String> =
        servers.iter().filter_map(|v| v["info"]["consensusBranchId"].as_str().map(str::to_owned)).collect();
    Ok(Json(json!({
        "network": s.zcash_network,
        "servers": servers,
        "branchesAgree": branches.len() <= 1,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessRequest {
    pub terms: LoanTerms,
    pub salt: B256,
    /// When set, the terms must match this loan's on-chain termsHash.
    pub loan_id: Option<B256>,
    pub collateral_zat: u64,
    pub price_usd_e8: u64,
    #[serde(default)]
    pub elapsed_secs: u64,
    #[serde(default)]
    pub repaid: u128,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessResponse {
    pub terms_hash: B256,
    pub matches_on_chain: Option<bool>,
    pub debt: u128,
    pub ltv_bps: Option<u32>,
    pub health: Health,
}

async fn assess(State(s): State<Shared>, Json(req): Json<AssessRequest>) -> ApiResult<AssessResponse> {
    req.terms.validate().map_err(bad_request)?;
    let hash = terms_hash(&req.terms, req.salt);
    let matches_on_chain = match req.loan_id {
        Some(id) => Some(
            lentic_base::read_loan(&s.provider, s.deployment.loan_book, id)
                .await?
                .is_some_and(|l| l.terms_hash == hash),
        ),
        None => None,
    };
    let debt = math::accrued_debt(req.terms.principal, req.terms.apr_bps, req.elapsed_secs).saturating_sub(req.repaid);
    let price = Price { usd_e8: req.price_usd_e8 };
    Ok(Json(AssessResponse {
        terms_hash: hash,
        matches_on_chain,
        debt,
        ltv_bps: math::ltv_bps(debt, req.collateral_zat, price),
        health: math::assess(&req.terms, debt, req.collateral_zat, price),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRequest {
    pub terms: LoanTerms,
    pub collateral_zat: u64,
    /// Number of escrow notes the liquidation spends (fee depends on it).
    #[serde(default = "one")]
    pub spends: usize,
    pub debt: u128,
    pub price_usd_e8: u64,
}

fn one() -> usize {
    1
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanResponse {
    pub split: LiquidationSplit,
    pub lender_address: String,
    pub borrower_address: String,
    pub liquidatable: bool,
}

/// The split at a given price and debt (a calculator; co-signing uses the signer's own numbers).
async fn liquidation_plan(Json(req): Json<PlanRequest>) -> ApiResult<PlanResponse> {
    req.terms.validate().map_err(bad_request)?;
    let price = Price { usd_e8: req.price_usd_e8 };
    let liquidatable = math::assess(&req.terms, req.debt, req.collateral_zat, price) == Health::Liquidating;
    let fee = lentic_zcash::fee::zip317_fee(req.spends, 2);
    let split =
        math::liquidation_split(req.collateral_zat, fee, req.debt, price, req.terms.haircut_bps, req.terms.penalty_bps)
            .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(PlanResponse {
        split,
        lender_address: req.terms.lender_zcash_address,
        borrower_address: req.terms.borrower_return_address,
        liquidatable,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthRequest {
    pub health: Health,
    pub oracle_round: u64,
    #[serde(default)]
    pub attestation_hash: B256,
}

async fn set_health(State(s): State<Shared>, Path(id): Path<B256>, Json(req): Json<HealthRequest>) -> ApiResult<Value> {
    let tx = lentic_base::update_health(
        &s.provider,
        s.deployment.loan_book,
        id,
        req.health,
        req.oracle_round,
        req.attestation_hash,
    )
    .await?;
    Ok(Json(json!({ "tx": tx })))
}
