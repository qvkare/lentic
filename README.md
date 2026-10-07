<img src="web/public/lentic-mark.svg" width="56" height="56" alt="Lentic">

# Lentic: USDC loans against shielded ZEC that never leaves the pool

> The borrower's ZEC sits in a per-loan **2-of-3 FROST** escrow (borrower · lender · TEE policy signer) inside
> the **Ironwood** shielded pool. Nothing is wrapped, bridged or taken into custody. The lender checks the
> collateral with the escrow's viewing key, USDC comes from the lender's own vault on **Base**, and on default
> the lender and the policy signer sign a liquidation at the live price: ZEC covering the debt goes
> shielded→shielded to the lender, the surplus back to the borrower.

Colosseum Crypto World's Fair · Zcash (+ Base).

## Running on testnet

Two loans run end to end on Zcash testnet and Base Sepolia (`scripts/testnet-e2e.sh`, 7 Oct 2026). The escrow keys
were made through the policy signer's relay with the borrower and the lender as separate processes, the vault lends
Circle's test USDC, and every escrow spend was co-signed by the policy signer over its API after it worked out the
payments itself. A Zcash explorer shows only that shielded transactions happened.

| Loan | Step | Chain | Transaction | What happened |
|---|---|---|---|---|
| A | Deposit | Zcash | [`f0c1e5d6…d34f93f0`](https://testnet.zecblock.com/tx/f0c1e5d6e70a3286777774817cab936f2cf7f8bb01dc79074636cdb8d34f93f0) | 0.01 TAZ into a 2-of-3 escrow made through the signer's relay |
| A | Fund | Base | [`0x066eb300…6af1b01b`](https://sepolia.basescan.org/tx/0x066eb300d0a74b0b32ff30b625fc38ec5a40da2a6fd1de8effe3bca56af1b01b) | 6 USDC from the lender's vault to the borrower's Base address; LoanBook stores only the terms hash |
| A | Flood stage | Base | [`0xc3608142…d3ef89e7`](https://sepolia.basescan.org/tx/0xc3608142618feb16dc0b46cfa8168a29d4785210d59c0b2a049b265bd3ef89e7) | ZEC/USD set to 750 (a test price), LTV 80 %: the signer reports `Liquidating` |
| A | Liquidation | Zcash | [`8f77eee4…74b20ab5`](https://testnet.zecblock.com/tx/8f77eee426e01da6b2187c3d5a12c0da9a5d3a2dc03b082f11e6178474b20ab5) | Lender + signer co-sign the signer's split: 0.00867369 TAZ to the lender, 0.00122631 TAZ to the borrower, 0.0001 TAZ fee |
| A | Outcome | Base | [`0x90665d8f…252480e4`](https://sepolia.basescan.org/tx/0x90665d8fa787e3d87519eb21955a1acf3f27151d27281a04a6ef32d6252480e4) | The signer sees the escrow spent and writes `Liquidated` with attestation `0x7a921155…c0a649` |
| A | Write-off | Base | [`0x6ce7324f…9c56ab43`](https://sepolia.basescan.org/tx/0x6ce7324f45d9cc61d863e0e54068717d402f5084e8c8cf713334d83d9c56ab43) | The lender holds the ZEC; the 6 USDC principal leaves the vault's books |
| B | Deposit | Zcash | [`334c6b25…526e6354`](https://testnet.zecblock.com/tx/334c6b254a9790e1aa0c2ff826fbe773a06ea91c1d48d0a39d8178d4526e6354) | 0.01 TAZ into a second escrow |
| B | Fund | Base | [`0xa8a45ea8…b5e8a4a0`](https://sepolia.basescan.org/tx/0xa8a45ea834e2441261e02f87a8f4164d4f30bd41849b9b1d2ff73276b5e8a4a0) | 6 USDC |
| B | Repay | Base | [`0xb6b13532…120b5503`](https://sepolia.basescan.org/tx/0xb6b135322c6a59e0475ee59aad95810b52861e8e2cefcca064522598120b5503) | 6.010001 USDC, principal first; 10 % of the interest goes to Lentic's fee recipient |
| B | Close | Zcash | [`29c9d0cf…e7539266`](https://testnet.zecblock.com/tx/29c9d0cfc44e81c7c60a0d679d12a3dc76f3c82d72aa1c96a0352be4e7539266) | Borrower + signer return 0.0099 TAZ to the borrower |
| B | Outcome | Base | [`0xf8f02b14…70b4add9`](https://sepolia.basescan.org/tx/0xf8f02b149aef7de1b46b7b283020535b608e0275758ba3e84a1cc72c70b4add9) | `Closed`, with attestation `0x83542482…5cc94b` |

Base Sepolia contracts:

| Contract | Address |
|---|---|
| LoanBook | [`0xc09454…302284`](https://sepolia.basescan.org/address/0xc0945436c9d0F0d247a5d35AA9F8EB0AE2302284) |
| LenderVault (created by LoanBook) | [`0xfE5cb9…eB29f0`](https://sepolia.basescan.org/address/0xfE5cb917a79b06010f18Be6219ca829358eB29f0) |
| AttestationVerifier | [`0x0A80B0…6dbB9b`](https://sepolia.basescan.org/address/0x0A80B04B30b9ca3F17470806D5FaB939766dbB9b) |
| MockDcapAttestation (development attestor) | [`0x5AAbdF…33E183`](https://sepolia.basescan.org/address/0x5AAbdF2A2a4fE0c91EBc0E99329a4FD8Bc33E183) |
| USDC (Circle's Base Sepolia test token) | [`0x036CbD…3dCF7e`](https://sepolia.basescan.org/address/0x036CbD53842c5426634e7929541eC2318f3dCF7e) |
| Policy signer key (development attestor) | [`0x021662…41Aa02`](https://sepolia.basescan.org/address/0x021662E126617c4EA8508E233e04034cab41Aa02) |

The policy signer runs with the development attestor (a mock DCAP verifier accepts its development quote); the
amounts, the attested split and earlier runs are in [docs/testnet.md](docs/testnet.md).

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
| Two loans on Zcash testnet and Base Sepolia: deposits, a co-signed liquidation and a co-signed close mined, outcomes and attestation hashes in LoanBook | [Running on testnet](#running-on-testnet), `scripts/testnet-e2e.sh` |
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
