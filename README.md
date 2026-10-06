# Lentic — USDC loans against shielded ZEC that never leaves the pool

> The borrower's ZEC sits in a per-loan **2-of-3 FROST** escrow (borrower · lender · TEE policy signer) inside
> the **Ironwood** shielded pool. Nothing is wrapped, bridged or taken into custody. The lender checks the
> collateral with the escrow's viewing key, USDC comes from the lender's own vault on **Base**, and on default
> the lender and the policy signer sign a liquidation at the live price: ZEC covering the debt goes
> shielded→shielded to the lender, the surplus back to the borrower.

Colosseum Crypto World's Fair · Zcash (+ Base).

## How it works

```
Borrower CLI ── FROST share 1 ─┐                      ┌─ FROST share 2 ── Lender daemon (FVK watch, PCZT prove, broadcast)
                               └── frostd relay ──────┤
                                                      └─ FROST share 3 ── Policy signer (dstack/TDX, attested on Base)
Zcash (Ironwood, v6 tx): escrow notes · close / liquidation built as PCZTs at event time
Base: LenderVault (ERC-4626, one per lender) · LoanBook {termsHash, health, attestationHash} · AttestationVerifier (Automata DCAP)
```

- **Any two of three** can move the collateral. Lender + policy can only liquidate, with the split the policy signer
  computes from the live price; borrower + policy can only return the collateral once repayment is seen on Base;
  borrower + lender can do anything together. Lentic runs only the policy signer: at most 1/3.
- **Nothing is pre-signed.** A fully signed shielded transaction can be broadcast by whoever holds it (no script, no
  timelock), so every signature is made at event time. What v6 transactions give us instead is the separation of
  signing and proving: spend-auth signatures do not commit to the anchor or witnesses, so signers never touch the
  note commitment tree and the prover refreshes the anchor right before broadcast.
- **Privacy, honestly.** Hidden: collateral amount, escrow, Zcash addresses, liquidation split. Public: that shielded
  transactions happened; on Base the terms hash, the health flag and the ordinary USDC transfers of the loan.

## Verified against source and live networks (6 Oct 2026)

| Claim the design relies on | Evidence |
|---|---|
| Re-randomized FROST (reddsa 0.5.2 / frost-rerandomized 3.0) signatures verify as Orchard spend-auth signatures under `rk = ak + [α]G`, with α taken from the PCZT | `lentic-frost` tests, `escrow_e2e` |
| v6 sighash is independent of anchor and witnesses: sign first, then set a *newer* anchor and witness, prove, extract | `liquidation_by_lender_and_policy_with_anchor_refresh` (sighash compared before/after) |
| Signatures are bound to the consensus branch: an NU6.3 signature is rejected on the NU7 transaction; re-signing works | `signatures_are_bound_to_the_consensus_branch` |
| Ironwood PCZTs build under NU7 (testnet is already past NU7 at 4,465,026) | same test, real testnet params |
| Mainnet NU6.3 at 3,428,143; branch ids 0x37A5165B / 0x77190AD9 | `zcash_protocol` 0.11.0-pre.0; live: mainnet tip on `37a5165b`, testnet tip on `77190ad9` (`lentic-watch info`) |
| `termsHash` encoding identical in Rust, Solidity and `cast` | shared vector `contracts/test/vectors/terms_hash.txt` |
| The policy signer's dev quote parses on chain with the same measurement/report-data layout | smoke test registers the service-generated quote |

**Not yet verified:** a real TDX quote through Automata DCAP on Base (needs a dstack CVM), the policy image build in
Docker (no daemon on the dev machine), a broadcast on a public network.

**Upstream caveat:** the RedPallas FROST ciphersuite was removed from reddsa `main` on 2026-09-25 while it moves to the
`frost` repo ([reddsa#280](https://github.com/ZcashFoundation/reddsa/pull/280), [frost#963](https://github.com/ZcashFoundation/frost/issues/963)).
Lentic pins the last release that ships it (reddsa 0.5.2). Re-randomized FROST/RedPallas is unaudited (NCC 2023 covered
frost-core only), and signing with a PCZT-supplied randomizer uses an API frost-rerandomized 3.0 deprecates (frost#1094).

## Repo layout

| Path | What |
|---|---|
| `crates/lentic-core` | Loan terms + `termsHash`, health, LTV / accrued debt / liquidation split |
| `crates/lentic-frost` | 2-of-3 DKG with Orchard even-y normalization, escrow FVK from `ak` + borrower seed, re-randomized signing |
| `crates/lentic-zcash` | Escrow address/UFVK, Ironwood PCZT pipeline (build → check outputs → sign → refresh anchor → prove → extract), ZIP 317 fee, lightwalletd client, offline chain sim |
| `crates/lentic-base` | alloy bindings for the Base contracts, loan/vault reads, health writes |
| `crates/lentic-policy` | Policy signer service: dstack KMS key + TDX quote (or dev quote), assess / liquidation-plan API, Base + lightwalletd reads |
| `crates/lentic-signer` | Borrower/lender CLI: DKG, share bundles, escrow keys, PCZT inspect/sign, `terms-hash`, offline `demo` |
| `crates/lentic-watch` | Chain CLI: server info, Ironwood tree state, prove, extract, broadcast |
| `contracts/` | Foundry: `LenderVault`, `LoanBook`, `AttestationVerifier` + `TdxQuote`, `LoanTermsLib`; mocks; deploy scripts |
| `web/` | Vite + React + viem: lender panel and borrower status page |
| `deploy/` | Policy signer Dockerfile, local docker-compose, dstack app compose |
| `scripts/` | `export-abis.mjs`, `dev-deploy.sh`, `smoke.sh` |

## Quickstart

Requirements: Rust (stable), Foundry, Node 22 + pnpm.

```bash
git submodule update --init --recursive && pnpm install
```

```bash
pnpm demo            # offline: DKG → deposit → liquidation PCZT → FROST 2-of-3 → anchor refresh → proof → valid tx
```

Local stack (anvil on :8546 so it does not collide with another local chain on :8545):

```bash
pnpm local:chain     # terminal 1
```

```bash
cargo build --workspace && pnpm local:deploy    # contracts + one funded demo loan → dev/loan.local.json
```

```bash
pnpm policy          # terminal 2: policy signer on http://127.0.0.1:8787 (dev attestor, dev routes)
```

```bash
pnpm web             # terminal 3: http://localhost:5174
```

Everything at once on throwaway ports, as CI does:

```bash
pnpm smoke
```

Tests: `pnpm test` (Rust workspace incl. the Ironwood end-to-end flows, then Foundry).

## Policy signer API

| Route | |
|---|---|
| `GET /v1/info` | signer address, attestor, measurement, whether it is registered on Base |
| `GET /v1/attestation` | report data and quote to submit to `AttestationVerifier.register` |
| `GET /v1/loans`, `GET /v1/loans/{id}` | LoanBook state |
| `GET /v1/zcash` | chain info from every configured lightwalletd and whether they agree |
| `POST /v1/assess` | terms + salt (+ loan id) → terms match on chain, accrued debt, LTV, health |
| `POST /v1/liquidation-plan` | terms + price + debt → the exact outputs the signer will co-sign |
| `POST /v1/loans/{id}/health` | dev only (`--dev-routes`): write health from the signer key |

## Next

Networked DKG/signing over frostd; FVK scanning into a wallet DB (`zcash_client_sqlite`) in `lentic-watch`; the policy
loop (oracle → health → liquidation co-signing) and the policy signer's FROST share storage; Pyth ZEC/USD on Base;
dstack deployment and real DCAP registration on Base Sepolia.

## License

MIT
