<img src="web/public/lentic-mark.svg" width="56" height="56" alt="Lentic">

# Lentic: USDC loans against shielded ZEC that never leaves the pool

> The borrower's ZEC sits in a per-loan **2-of-3 FROST** escrow (borrower · lender · TEE policy signer) inside
> the **Ironwood** shielded pool. Nothing is wrapped, bridged or taken into custody. The lender checks the
> collateral with the escrow's viewing key, USDC comes from the lender's own vault on **Base**, and on default
> the lender and the policy signer sign a liquidation at the live price: ZEC covering the debt goes
> shielded→shielded to the lender, the surplus back to the borrower.

Colosseum Crypto World's Fair · Zcash (+ Base).

## Running on Zcash testnet

Two loans run end to end with `scripts/testnet-e2e.sh` (7 Oct 2026). The escrow keys were made through the
policy signer's relay with the borrower and the lender as separate processes, and every spend was co-signed by
the policy signer over its API after it computed the payments itself. Explorer: ZecBlock testnet, which shows
only that shielded transactions happened.

| Loan | Step | Zcash testnet transaction |
|---|---|---|
| A | Deposit into the 2-of-3 escrow | [`18df5f27…91b0b18c`](https://testnet.zecblock.com/tx/18df5f277fcbd5a2ed26f34400690215814e1a0179ff299905be377e91b0b18c) |
| A | Liquidation at flood stage, lender + policy signer | [`608ffb17…8633cc9d`](https://testnet.zecblock.com/tx/608ffb1728e9583a788313a515e9750a029701ccadc8cb664cce3b518633cc9d) |
| B | Deposit | [`13b0f0cc…66be02e8`](https://testnet.zecblock.com/tx/13b0f0ccbaba518e1f5c4adce862ffdb1dcf89233d7c3181ed519a5c66be02e8) |
| B | Close after repayment, borrower + policy signer | [`7faf0a83…a6c593df`](https://testnet.zecblock.com/tx/7faf0a83178dcbcce9df8c6a3290e1073d90cc830c09f0843e8498d2a6c593df) |

The amounts, the split the signer attested and the earlier runs are in [docs/testnet.md](docs/testnet.md).

## How it works

```
Borrower CLI ── FROST share 1 ─┐                      ┌─ FROST share 2 ── Lender CLI (scan, PCZT, prove, broadcast)
                               └── policy signer API ─┤   (DKG relay: signed round 1, sealed round 2)
                                                      └─ FROST share 3 ── Policy signer (TEE, attested on Base)
Zcash (Ironwood, v6 tx): escrow notes · close / refund / liquidation built as PCZTs at event time
Base: LoanBook {termsHash, health, attestationHash} creates the lenders' LenderVaults (ERC-4626) · AttestationVerifier
```

- **Any two of three** can move the collateral. Lender + policy signer only liquidate, with the split the signer
  computes from the live price; borrower + policy signer only return the collateral once the debt is repaid on
  Base (or refund it if the loan was never funded); borrower + lender can do anything together. Lentic runs only
  the policy signer: at most 1/3.
- **The policy signer checks, then signs.** For every spend it scans the escrow from its viewing key, reads the
  debt from the vault's repayment events and the price from its own feed, works out the only payments it accepts
  and checks the PCZT against them: every escrow note spent and nothing else, exactly the planned outputs (so the
  ZIP 317 fee), a short expiry, the next block's consensus branch, no other spend in flight, and an Ironwood
  tree root that every configured lightwalletd agrees with. Then it runs two FROST rounds and returns an
  attestation (price, round, debt, payments, sighash) signed by its Base key; LoanBook stores its hash.
- **The key is made through an untrusted relay.** Each party's round-1 post is signed by its Base key; round-2
  packages travel sealed (ECIES) to their recipient, so the relay, though it is the policy signer, cannot rebuild
  the other parties' polynomials.
- **Nothing is pre-signed.** A fully signed shielded transaction can be broadcast by whoever holds it (no script,
  no timelock), so every signature is made at event time. v6 transactions separate signing from proving:
  spend-auth signatures do not commit to the anchor or witnesses, so signers never touch the note commitment tree
  and the prover refreshes the anchor right before broadcast.
- **Privacy, honestly.** Hidden: collateral amount, escrow, Zcash addresses, liquidation split. Public: that
  shielded transactions happened; on Base the terms hash, the health flag and the ordinary USDC transfers.

## Verified against source and live networks

| Claim | Evidence |
|---|---|
| A FROST 2-of-3 escrow on Zcash testnet: deposits, a co-signed liquidation and a co-signed close mined | [docs/testnet.md](docs/testnet.md), `scripts/testnet-e2e.sh` |
| Re-randomized FROST (reddsa 0.5.2 / frost-rerandomized 3.0) signatures verify as Ironwood spend-auth signatures under `rk = ak + [α]G`, α from the PCZT | `lentic-frost` tests, `escrow_e2e`, testnet |
| Sign first, then set a newer anchor and witness, prove, extract: the sighash does not change | `liquidation_by_lender_and_policy_with_anchor_refresh`, every testnet spend |
| The escrow watcher's anchor equals the server's Ironwood tree root; tree size checked after every block | `scan_e2e`, `lentic-watch scan` (`anchorMatchesServer`) |
| The policy signer refuses other payments, partial spends, foreign notes and long-lived transactions | `cosign::tests` |
| A relay that is also a participant cannot read or swap DKG packages | `relay::tests` |
| Signatures are bound to the consensus branch: an NU6.3 signature fails under NU7; re-signing works | `signatures_are_bound_to_the_consensus_branch` |
| Only vaults LoanBook created open loans; a loan id is bound to its vault | `test_open_onlyByVaultsCreatedHere`, `test_noVaultCanTakeAnotherVaultsLoanId` |
| `termsHash` identical in Rust, Solidity and `cast` | `contracts/test/vectors/terms_hash.txt` |
| Interest accrues on the outstanding principal, repayments booked principal first | `interest_accrues_only_on_outstanding_principal` |

**Not verified yet:** a real TDX quote through Automata DCAP on Base (needs a dstack CVM; the testnet runs use the
development attestor), the policy image build for `linux/amd64`, a mainnet loan.

**Upstream notes.** The RedPallas FROST ciphersuite was removed from reddsa `main` on 2026-09-25 while it moves to
the `frost` repo ([reddsa#280](https://github.com/ZcashFoundation/reddsa/pull/280),
[frost#963](https://github.com/ZcashFoundation/frost/issues/963)); Lentic pins the last release that ships it
(reddsa 0.5.2). Re-randomized FROST/RedPallas is unaudited (NCC 2023 covered frost-core only). Signing with the
PCZT's α uses the API frost-rerandomized 3.0 marks deprecated; the maintainer confirmed on 1 Oct 2026 that a
builder-chosen α is fine as long as it stays among the parties
([frost#1094](https://github.com/ZcashFoundation/frost/issues/1094)), which is why PCZTs only travel to the
policy signer over its API. The escrow's viewing key is derived from a borrower seed, not by ZIP 2005's
"Usage with FROST" derivation, so escrow notes are not quantum-recoverable yet.

## Repo layout

| Path | What |
|---|---|
| `crates/lentic-core` | Loan terms, `termsHash`, loan ids, health, debt on the outstanding principal, liquidation split |
| `crates/lentic-frost` | 2-of-3 DKG with Orchard even-y normalization, escrow FVK, re-randomized signing, the DKG relay protocol |
| `crates/lentic-zcash` | Escrow keys and addresses, the escrow watcher (compact blocks, nullifiers, witnesses), the Ironwood PCZT pipeline, ZIP 317 fee, lightwalletd client, a testnet wallet, offline chain sim |
| `crates/lentic-base` | alloy bindings for the Base contracts, loan, vault and repayment reads, health writes |
| `crates/lentic-policy` | Policy signer: dstack KMS key + TDX quote (or dev quote), DKG relay, loan registration, escrow scans, co-signing with checks and attestations |
| `crates/lentic-signer` | Borrower/lender CLI: Base keys, DKG through the signer, terms approval, `cosign` (close, refund, liquidation) |
| `crates/lentic-watch` | Chain CLI: scan a viewing key, build and finalize escrow spends, a testnet wallet, broadcast |
| `contracts/` | Foundry: `LoanBook` (creates vaults), `LenderVault`, `AttestationVerifier` + `TdxQuote`, `LoanTermsLib`; mocks; deploy scripts |
| `web/` | Vite + React + viem: home page (policy simulations, a loan simulator, a key picker), the lender book, one gauge station per loan. Design in [web/DESIGN.md](web/DESIGN.md) |
| `deploy/` | Policy signer Dockerfile, local docker-compose, dstack app compose |
| `scripts/` | `testnet-e2e.sh`, `deploy-sepolia.sh`, `dev-deploy.sh`, `smoke.sh`, `export-abis.mjs`, `screens.mjs`, `design-check.mjs` |

## Quickstart

Requirements: Rust (stable), Foundry, Node 22 + pnpm.

```bash
git submodule update --init --recursive && pnpm install
```

```bash
pnpm demo            # offline: DKG → deposit → liquidation PCZT → FROST 2-of-3 → anchor refresh → proof → valid tx
```

Two loans on Zcash testnet (needs a funded test wallet; Base is a local anvil unless `BASE=sepolia`):

```bash
cargo build --workspace
```

```bash
./target/debug/lentic-watch wallet new --out dev/testnet/funder.wallet.json
```

```bash
./target/debug/lentic-watch wallet new --out dev/testnet/lender-payout.wallet.json
```

```bash
bash scripts/testnet-e2e.sh
```

Fund the first wallet's `addressR0` (`utest1…`) from a testnet faucet before the last step.

Local stack (anvil on :8546 so it does not collide with another local chain on :8545):

```bash
pnpm local:chain     # terminal 1
```

```bash
cargo build --workspace && pnpm local:deploy    # contracts + demo loans → dev/loan.local.json
```

```bash
pnpm policy          # terminal 2: policy signer on http://127.0.0.1:8787
```

```bash
pnpm web             # terminal 3: http://localhost:5174
```

Everything offline on throwaway ports, as CI does:

```bash
pnpm smoke
```

Tests: `pnpm test` (Rust workspace incl. the Ironwood end-to-end flows, then Foundry). Design: `pnpm design:check`
and `pnpm screens`.

## The two CLIs

```
lentic-signer base-key --out borrower.key
lentic-signer dkg create --borrower 0x… --lender 0x…                       # on the policy signer
lentic-signer dkg join --session 0x… --role borrower --base-key borrower.key --out borrower.share.json
lentic-signer terms approve --terms terms.json --salt 0x… --base-key borrower.key
lentic-signer terms register --terms terms.json --salt 0x… --session 0x… --borrower-signature … --lender-signature …
lentic-signer cosign --loan-id 0x… --kind liquidation|close|refund --share lender.share.json
lentic-watch scan --ufvk uvftest1… --birthday <height>
lentic-watch wallet new|show|balance|send
lentic-watch escrow build|finalize
```

## Policy signer API

| Route | |
|---|---|
| `GET /v1/info` | signer address, attestor, measurement, registration on Base, keys and loans held |
| `GET /v1/price` | the latest ZEC/USD reading: median of five public exchange tickers |
| `POST /v1/dkg`, `GET /v1/dkg/{id}`, `POST /v1/dkg/{id}/round1`, `/round2`, `/done` | the DKG relay; the signer joins as the third participant |
| `POST /v1/loans` | terms approved by borrower and lender, bound to the DKG session's escrow key |
| `GET /v1/loans`, `GET /v1/loans/{id}` | LoanBook state |
| `GET /v1/loans/{id}/gauge`, `GET /v1/loans/{id}/escrow` | readings, debt, the escrow's notes and the co-signing record (parties only in production) |
| `GET /v1/loans/{id}/plan/{kind}` | what the signer would co-sign now (`liquidation`, `close`, `refund`) and why |
| `POST /v1/loans/{id}/cosign`, `POST /v1/cosign/{session}/shares` | the two FROST rounds; round 1 only after the checks |
| `GET /v1/attestation` | report data and quote to submit to `AttestationVerifier.register` |
| `GET /v1/zcash` | chain info from every configured lightwalletd |
| `POST /v1/assess`, `POST /v1/liquidation-plan` | calculators |
| `POST /v1/loans/{id}/health`, `POST /v1/dev/price` | development only (`--dev-routes`) |

## Limits

- The policy signer has not yet run inside a TDX CVM; on testnet it runs with the development attestor, and the
  Base Sepolia deployment accepts its development quote through a mock DCAP verifier.
- Party routes (gauge, escrow, plan) are unauthenticated: run the signer where only the parties reach it.
- The price is a median of five exchanges until an authenticated oracle is wired in.
- The escrow's viewing key is not ZIP 2005 quantum-recoverable (see above).
- Lending is licensed: Lentic is built for commercial borrowers and professional lenders, outside the EU.

## Built on

The librustzcash crates, `orchard`, `reddsa`/`frost`, alloy, OpenZeppelin Contracts and Foundry.

## License

MIT
