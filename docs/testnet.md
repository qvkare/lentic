# Testnet record

Every transaction Lentic has made on public test networks, with links. Zcash testnet runs NU7
(consensus branch `0x77190ad9`, 25-second blocks since height 4,465,026); the Ironwood pool has
been active there since 4,134,000. Chain data comes from the public lightwalletd at
`testnet.zec.rocks:443`. A public explorer shows only that a shielded transaction happened:
amounts, addresses and the escrow stay hidden.

Explorer: [ZecBlock testnet](https://testnet.zecblock.com). (`testnet.zcashexplorer.app` stopped
indexing at height 4,331,234, so it does not show these.)

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
