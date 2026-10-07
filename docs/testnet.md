# Testnet record

Every transaction Lentic has made on public test networks, with links. Zcash testnet runs NU7
(consensus branch `0x77190ad9`, 25-second blocks since height 4,465,026); the Ironwood pool has
been active there since 4,134,000. Chain data comes from the public lightwalletd at
`testnet.zec.rocks:443`. A public explorer shows only that a shielded transaction happened:
amounts, addresses and the escrow stay hidden.

Explorer: [ZecBlock testnet](https://testnet.zecblock.com). (`testnet.zcashexplorer.app` stopped
indexing at height 4,331,234, so it does not show these.)

## Run 1: two loans co-signed through the policy signer's API (7 Oct 2026)

The whole protocol with the policy signer as the third key (`scripts/testnet-e2e.sh`):

1. The escrow key is made through the signer's relay. The borrower and the lender run as
   separate processes; round-2 packages travel sealed to their recipients.
2. Both parties approve the terms (EIP-191), and the signer accepts them only for the escrow
   key it helped make.
3. A real deposit goes into the escrow and the lender's vault funds the loan on Base.
4. Every spend is co-signed by the signer only after it has worked out the payments itself:
   from its own scan of the escrow, its own price and the vault's repayment events.

The Base half of this run was a local chain; the Zcash half is public.

| Loan | Step | Transaction | What happened |
|---|---|---|---|
| A | Deposit | [`18df5f27…91b0b18c`](https://testnet.zecblock.com/tx/18df5f277fcbd5a2ed26f34400690215814e1a0179ff299905be377e91b0b18c) | 0.01 TAZ into an escrow made through the relay |
| A | Liquidation | [`608ffb17…8633cc9d`](https://testnet.zecblock.com/tx/608ffb1728e9583a788313a515e9750a029701ccadc8cb664cce3b518633cc9d) | ZEC/USD set to 750 (a test price), LTV 80 % against the 75 % flood stage. Lender and signer co-signed: 0.00867369 TAZ to the lender (6.000001 USDC owed plus the 3 % penalty, priced at 750 USD less the 5 % haircut), 0.00122631 TAZ back to the borrower, 0.0001 TAZ fee. Attestation `0x528e279c…99a86e` |
| B | Deposit | [`13b0f0cc…66be02e8`](https://testnet.zecblock.com/tx/13b0f0ccbaba518e1f5c4adce862ffdb1dcf89233d7c3181ed519a5c66be02e8) | 0.01 TAZ |
| B | Close | [`7faf0a83…a6c593df`](https://testnet.zecblock.com/tx/7faf0a83178dcbcce9df8c6a3290e1073d90cc830c09f0843e8498d2a6c593df) | The borrower repaid 6.010001 USDC into the vault; borrower and signer returned 0.0099 TAZ to the borrower. Attestation `0x886ef6c0…853045` |

On Base the signer reported flood stage for A, then wrote `Liquidated` and `Closed` to LoanBook
with each attestation's hash once it saw the escrow notes spent.

The liquidation attestation the signer returned with its shares (signed by its Base key, hash in
LoanBook):

```json
{
  "protocol": "lentic-attestation-v1", "kind": "liquidation", "priceUsd": 750.0,
  "owed": { "principal": 6000000, "interest": 1 }, "collateralZat": 1000000, "feeZat": 10000,
  "payments": [ { "zat": 867369, "address": "<lender payout>" }, { "zat": 122631, "address": "<borrower return>" } ],
  "sighash": "9dcc3386513bce4c66cbc8adcc0197020a75e915a51383783a58e92817fb8f60", "expiryHeight": 4475233
}
```

## Run 0: the escrow pipeline on a public network (7 Oct 2026)

Goal: prove the Zcash half end to end before the policy signer co-signs over its API. The FROST
group was made in one process (`lentic-signer dkg-local`) and signed with two local shares; every
other step is the production code path: scanning the escrow from its viewing key, building the
spend from the scanned notes, refreshing the anchor and witness after signing, proving and
broadcasting.

| Step | Transaction | Height | What happened |
|---|---|---|---|
| Faucet | [`8c7c6d14…e21358fa`](https://testnet.zecblock.com/tx/8c7c6d14a55b6a103fd4557fd943098d0002572250a6e06fb209e9e9e21358fa) | 4,475,135 | 0.125 TAZ from the Valar faucet into a test wallet (`lentic-watch wallet`) |
| Deposit | [`20b73c0c…8946b5c2`](https://testnet.zecblock.com/tx/20b73c0c9eef81074c4f67c2f3711ac3cae859861603d61985451d578946b5c2) | 4,475,151 | 0.02 TAZ into the loan's 2-of-3 escrow; the escrow's viewing key sees the Ironwood note at position 506,500 |
| Liquidation | [`95495428…45137d98`](https://testnet.zecblock.com/tx/95495428d0acf20e92d6f3a88335338f9a2e0599c5bdf1bff0ee9cdb45137d98) | 4,475,154 | Lender + policy shares (re-randomized FROST, the PCZT's α), anchor refreshed after signing: 0.012 TAZ to the lender, 0.0079 TAZ back to the borrower, 0.0001 TAZ fee (ZIP 317) |

Escrow address (revision 2): `zutest1cqaw9evjyg5aektwce9x7lmk6t7wsqczfhf56da95jhayadn49gzjxdj90a6u9lhkpajctwtwler87djgql0gcv63p7u7s337uaqqhth`.

What each party saw, from its viewing key (`lentic-watch scan` / `wallet balance`):

- escrow: one 0.02 TAZ note, spent at 4,475,154 in `95495428…`;
- lender: one 0.012 TAZ note from `95495428…`;
- borrower: 0.0079 TAZ back from `95495428…`, next to the change of its deposit.

Reproduce (testnet, needs a funded test wallet):

```bash
lentic-signer dkg-local --out dev/testnet/loan0
lentic-signer show --share dev/testnet/loan0/lender.share.json --network testnet
lentic-watch wallet send --wallet dev/testnet/funder.wallet.json --to <escrowAddress> --zat 2000000
lentic-watch escrow build --ufvk <ufvk> --birthday <height> --pay <lender>=1200000 --rest-to <borrower> --out liq.pczt
lentic-signer sign-local --pczt liq.pczt --shares lender.share.json policy.share.json --out liq.signed.pczt
lentic-watch escrow finalize --ufvk <ufvk> --birthday <height> --pczt liq.signed.pczt
```
