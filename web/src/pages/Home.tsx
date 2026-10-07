import { useCallback, useEffect, useState } from "react";

import { ExplorerViews } from "../components/ExplorerViews";
import { FloodDrill, type DrillLoan } from "../components/FloodDrill";
import { KeyPicker } from "../components/KeyPicker";
import { VisibilityTable } from "../components/Ledgers";
import { LoanSimulator } from "../components/LoanSimulator";
import { PolicyStage } from "../components/PolicyStage";
import { deployment, listLoans } from "../chain";
import { clock, ltvAt, short, usd } from "../format";
import { PRICE_SOURCE, policyGauge, policyPrice } from "../policy";

/** Clearly marked stand-in when the policy signer cannot be reached. */
const EXAMPLE: DrillLoan = {
  loanId: null,
  debt: 10_000_000_000,
  collateralZat: 2_000_000_000,
  actionBps: 6000,
  floodBps: 7500,
  haircutBps: 500,
  penaltyBps: 300,
  livePrice: 1360,
  liveAt: 0,
  priceKind: "fixed",
  example: true,
};

/** The open loan with the most room below action stage, so the drill has the whole way to fall. */
async function featuredLoan(): Promise<DrillLoan> {
  const d = deployment();
  if (!d) return EXAMPLE;
  const [loans, price] = await Promise.all([listLoans(d), policyPrice()]);
  const open = loans.filter((l) => l.health !== "closed" && l.health !== "liquidated");
  const candidates = (
    await Promise.all(
      open.map(async (l) => {
        const g = await policyGauge(l.loanId).catch(() => null);
        const r = g?.readings[g.readings.length - 1];
        return g && r ? { l, g, r } : null;
      }),
    )
  ).filter((c) => c !== null);
  if (candidates.length === 0) return EXAMPLE;
  candidates.sort(
    (a, b) => ltvAt(a.r.debt, a.r.collateralZat, price.usd) - ltvAt(b.r.debt, b.r.collateralZat, price.usd),
  );
  const { l, g, r } = candidates[0];
  return {
    loanId: l.loanId,
    debt: r.debt,
    collateralZat: r.collateralZat,
    actionBps: g.actionStageBps,
    floodBps: g.floodStageBps,
    haircutBps: g.haircutBps,
    penaltyBps: g.penaltyBps,
    livePrice: price.usd,
    liveAt: price.at,
    priceKind: price.kind,
    example: false,
    termsHash: l.termsHash,
    vault: l.vault,
  };
}

type Mark = "acted" | "idle";
const STEPS: { title: string; body: string; keys: [Mark, Mark, Mark]; chain: string }[] = [
  {
    title: "Three keys are made",
    body: "The borrower, the lender and the policy signer run a key generation together. The group key becomes the loan's escrow address on Zcash; all three can see into it, any two can spend from it.",
    keys: ["acted", "acted", "acted"],
    chain: "Zcash",
  },
  {
    title: "The borrower deposits ZEC",
    body: "The ZEC lands in the shielded pool at the escrow address. The lender and the policy signer check it with the viewing key; the public cannot see it.",
    keys: ["acted", "idle", "idle"],
    chain: "Zcash",
  },
  {
    title: "The lender's vault pays out",
    body: "USDC goes from the lender's own vault to a fresh Base address. LoanBook records only a hash of the terms.",
    keys: ["idle", "acted", "idle"],
    chain: "Base",
  },
  {
    title: "Every round, a reading",
    body: "The policy signer reads ZEC/USD, works out the loan-to-value and writes a change of stage to LoanBook.",
    keys: ["idle", "idle", "acted"],
    chain: "Base",
  },
  {
    title: "Repaid: the ZEC goes home",
    body: "Once the repayment is seen on Base, the borrower and the policy signer return the ZEC to the borrower's shielded address.",
    keys: ["acted", "idle", "acted"],
    chain: "Zcash",
  },
  {
    title: "Flood stage: liquidation",
    body: "The lender and the policy signer sign a spend at the live price: the debt in ZEC to the lender, the rest back to the borrower.",
    keys: ["idle", "acted", "acted"],
    chain: "Zcash",
  },
];

const AUDIENCE: { who: string; does: string; policy: string }[] = [
  {
    who: "Long-term ZEC holders",
    does: "Borrow USDC for a tax bill, a house deposit or a business without selling ZEC or leaving the shielded pool.",
    policy: "Loan, credit line",
  },
  {
    who: "Lenders and credit funds",
    does: "Lend USDC from their own vault against collateral they can check on Zcash and never move alone.",
    policy: "Loan",
  },
  {
    who: "Zcash treasuries",
    does: "Pay grants and salaries in USDC while the treasury stays in ZEC.",
    policy: "Loan, credit line",
  },
  {
    who: "OTC desks",
    does: "Settle ZEC against USDC between parties who do not trust each other, with no custodian and no bridge.",
    policy: "OTC settlement",
  },
  {
    who: "Long-term savers",
    does: "Leave an heir a way to recover the ZEC without ever sharing a seed phrase.",
    policy: "Recovery",
  },
];

/** Two loans on Zcash testnet and Base Sepolia (docs/testnet.md, run 2): real transactions. */
const TESTNET: { loan: string; step: string; chain: "Zcash" | "Base"; tx: string; what: string }[] = [
  {
    loan: "A",
    step: "Deposit",
    chain: "Zcash",
    tx: "f0c1e5d6e70a3286777774817cab936f2cf7f8bb01dc79074636cdb8d34f93f0",
    what: "0.01 TAZ into a 2-of-3 escrow whose key was made through the policy signer's relay.",
  },
  {
    loan: "A",
    step: "Fund",
    chain: "Base",
    tx: "0x066eb300d0a74b0b32ff30b625fc38ec5a40da2a6fd1de8effe3bca56af1b01b",
    what: "6 USDC from the lender's vault. LoanBook records only a hash of the terms.",
  },
  {
    loan: "A",
    step: "Flood stage",
    chain: "Base",
    tx: "0xc3608142618feb16dc0b46cfa8168a29d4785210d59c0b2a049b265bd3ef89e7",
    what: "ZEC/USD set to 750 as a test price: the policy signer reports the loan liquidatable.",
  },
  {
    loan: "A",
    step: "Liquidation",
    chain: "Zcash",
    tx: "8f77eee426e01da6b2187c3d5a12c0da9a5d3a2dc03b082f11e6178474b20ab5",
    what: "The lender and the policy signer co-sign the split the signer computed: 0.00867 TAZ to the lender, 0.00123 TAZ back to the borrower.",
  },
  {
    loan: "A",
    step: "Outcome",
    chain: "Base",
    tx: "0x90665d8fa787e3d87519eb21955a1acf3f27151d27281a04a6ef32d6252480e4",
    what: "Liquidated, with the hash of the signer's attestation.",
  },
  {
    loan: "B",
    step: "Deposit",
    chain: "Zcash",
    tx: "334c6b254a9790e1aa0c2ff826fbe773a06ea91c1d48d0a39d8178d4526e6354",
    what: "0.01 TAZ into a second escrow.",
  },
  {
    loan: "B",
    step: "Repay",
    chain: "Base",
    tx: "0xb6b135322c6a59e0475ee59aad95810b52861e8e2cefcca064522598120b5503",
    what: "6.01 USDC back into the vault: principal and interest.",
  },
  {
    loan: "B",
    step: "Close",
    chain: "Zcash",
    tx: "29c9d0cfc44e81c7c60a0d679d12a3dc76f3c82d72aa1c96a0352be4e7539266",
    what: "The borrower and the policy signer return the ZEC to the borrower.",
  },
  {
    loan: "B",
    step: "Outcome",
    chain: "Base",
    tx: "0xf8f02b149aef7de1b46b7b283020535b608e0275758ba3e84a1cc72c70b4add9",
    what: "Closed, with the hash of the signer's attestation.",
  },
];

const txUrl = (chain: "Zcash" | "Base", tx: string) =>
  chain === "Zcash" ? `https://testnet.zecblock.com/tx/${tx}` : `https://sepolia.basescan.org/tx/${tx}`;

const PROOFS: { claim: string; evidence: string }[] = [
  {
    claim: "Two of three FROST shares produce a valid Zcash spend authorization; one share alone cannot.",
    evidence: "crates/lentic-frost/src/sign.rs",
  },
  {
    claim: "Signers never need the note tree: the anchor is refreshed after signing and the signature still holds.",
    evidence: "crates/lentic-zcash/tests/escrow_e2e.rs",
  },
  {
    claim: "A signature made under one network upgrade is rejected under the next, and re-signing works.",
    evidence: "signatures_are_bound_to_the_consensus_branch",
  },
  {
    claim: "The terms hash is the same in Rust, in Solidity and from cast.",
    evidence: "contracts/test/vectors/terms_hash.txt",
  },
];

function Keys({ keys }: { keys: [Mark, Mark, Mark] }) {
  const names = ["Borrower", "Lender", "Policy signer"];
  return (
    <>
      {keys.map((k, i) => (
        <td key={names[i]} className="key-col" data-label={names[i]}>
          <span
            className={k === "acted" ? "key-mark acted" : "key-mark"}
            role="img"
            aria-label={`${names[i]}: ${k === "acted" ? "signs" : "not needed"}`}
          />
        </td>
      ))}
    </>
  );
}

export function Home() {
  const [loan, setLoan] = useState<DrillLoan | null>(null);
  const load = useCallback(() => {
    featuredLoan()
      .then(setLoan)
      .catch(() => setLoan(EXAMPLE));
  }, []);
  useEffect(load, [load]);

  const terms = loan ?? EXAMPLE;
  const price = terms.livePrice;
  const priceLabel = terms.example
    ? `${usd(price)}, an example`
    : `${usd(price)}, ${PRICE_SOURCE[terms.priceKind]} at ${clock(terms.liveAt)}`;

  return (
    <main className="home">
      <section className="hero" aria-labelledby="claim">
        <div className="hero-copy">
          <h1 id="claim">ZEC that works without leaving the pool.</h1>
          <p className="hero-lede">
            Lentic is a shielded escrow on Zcash that only two of three keys can move: yours, a counterparty's and an
            attested policy signer's. The policy decides what the escrow is for. The first one lends USDC on Base
            against it; the same escrow can back a credit line, settle a trade or pass savings to an heir.
          </p>
          <div className="hero-actions">
            <a className="btn" href="#/?s=try">
              Try a loan
            </a>
            <a href="#/book">Open the lender book</a>
            {loan?.loanId && <a href={`#/station/${loan.loanId}`}>Watch a station live</a>}
          </div>
        </div>
        <div className="hero-drill">
          {loan ? (
            <FloodDrill
              loan={loan}
              still={window.location.hash.includes("drill=end")}
              onReplay={() => {
                if (window.location.hash.includes("drill=end")) window.location.hash = "#/";
                load();
              }}
            />
          ) : (
            <p className="quiet">Taking a reading from the policy signer.</p>
          )}
        </div>
      </section>

      <section id="policies" aria-labelledby="policies-title">
        <div className="section-head">
          <h2 id="policies-title">One escrow, four policies</h2>
          <p className="lede small">
            The keys and the policy signer stay the same; the policy decides what the escrow is for. Loans run on Zcash
            testnet and Base Sepolia today.
            Credit lines, OTC settlement and recovery are simulated here with the same keys.
          </p>
        </div>
        <PolicyStage price={price} priceLabel={priceLabel} />
      </section>

      <section id="try" aria-labelledby="try-title">
        <div className="section-head">
          <h2 id="try-title">Try a loan</h2>
          <p className="lede small">
            Lock ZEC, choose what to borrow, then move the price. The gauge is the one every loan gets.
          </p>
        </div>
        <LoanSimulator
          price={price}
          priceLabel={priceLabel}
          terms={{
            actionBps: terms.actionBps,
            floodBps: terms.floodBps,
            haircutBps: terms.haircutBps,
            penaltyBps: terms.penaltyBps,
          }}
        />
      </section>

      <section id="how" aria-labelledby="how-title">
        <div className="section-head">
          <h2 id="how-title">How a loan runs</h2>
          <p className="lede small">Each step names the keys that sign it and the chain it happens on.</p>
        </div>
        <table className="ledger steps">
          <thead>
            <tr>
              <th scope="col" className="step-n">
                Step
              </th>
              <th scope="col">What happens</th>
              <th scope="col" className="key-col">
                Borrower
              </th>
              <th scope="col" className="key-col">
                Lender
              </th>
              <th scope="col" className="key-col">
                Policy signer
              </th>
              <th scope="col">Chain</th>
            </tr>
          </thead>
          <tbody>
            {STEPS.map((s, i) => (
              <tr key={s.title} className={i === STEPS.length - 1 ? "flood-row" : undefined}>
                <td className="step-n" data-label="Step">
                  {i + 1}
                </td>
                <th scope="row" data-label="What happens">
                  <span className="step-title">{s.title}</span>
                  <span className="step-body">{s.body}</span>
                </th>
                <Keys keys={s.keys} />
                <td data-label="Chain">{s.chain}</td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="note">Steps 5 and 6 are alternatives: a loan ends one way or the other.</p>
      </section>

      <section id="keys" aria-labelledby="keys-title">
        <div className="section-head">
          <h2 id="keys-title">Two of three keys</h2>
          <p className="lede small">
            Pick keys to see what they can do in each policy. Lentic runs only the policy signer, whose code is attested
            on Base before LoanBook accepts its reports.
          </p>
        </div>
        <KeyPicker />
      </section>

      <section id="private" aria-labelledby="private-title">
        <div className="section-head">
          <h2 id="private-title">What stays private</h2>
          <p className="lede small">Shielded from the public, not from the escrow's own keyholders.</p>
        </div>
        <ExplorerViews
          loan={
            loan && loan.loanId && loan.termsHash && loan.vault
              ? { loanId: loan.loanId, termsHash: loan.termsHash, vault: loan.vault, debt: loan.debt }
              : null
          }
        />
        <VisibilityTable />
      </section>

      <section id="who" aria-labelledby="who-title">
        <div className="section-head">
          <h2 id="who-title">Who it is for</h2>
          <p className="lede small">The people who already hold ZEC, or want to trade it, without giving up the pool.</p>
        </div>
        <table className="ledger audience">
          <thead>
            <tr>
              <th scope="col">Who</th>
              <th scope="col">What they do with Lentic</th>
              <th scope="col">Policy</th>
            </tr>
          </thead>
          <tbody>
            {AUDIENCE.map((a) => (
              <tr key={a.who}>
                <th scope="row" data-label="Who">
                  {a.who}
                </th>
                <td data-label="What they do">{a.does}</td>
                <td data-label="Policy">{a.policy}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section id="testnet" aria-labelledby="testnet-title">
        <div className="section-head">
          <h2 id="testnet-title">On testnet</h2>
          <p className="lede small">
            Two loans on Zcash testnet and Base Sepolia. A Zcash explorer shows only that shielded transactions
            happened; Base shows the terms hash, the stage and the USDC.
          </p>
        </div>
        <table className="ledger proofs testnet">
          <thead>
            <tr>
              <th scope="col">Loan</th>
              <th scope="col">Step</th>
              <th scope="col">Chain</th>
              <th scope="col">Transaction</th>
              <th scope="col">What happened</th>
            </tr>
          </thead>
          <tbody>
            {TESTNET.map((t) => (
              <tr key={t.tx}>
                <td data-label="Loan">{t.loan}</td>
                <th scope="row" data-label="Step">
                  {t.step}
                </th>
                <td data-label="Chain">{t.chain}</td>
                <td data-label="Transaction">
                  <a className="mono" href={txUrl(t.chain, t.tx)}>
                    {short(t.tx, 6)}
                  </a>
                </td>
                <td data-label="What happened">{t.what}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section id="proven" aria-labelledby="proven-title">
        <div className="section-head">
          <h2 id="proven-title">Proven in code</h2>
          <p className="lede small">Each claim is a test that runs in CI.</p>
        </div>
        <table className="ledger proofs">
          <thead>
            <tr>
              <th scope="col">Claim</th>
              <th scope="col">Evidence</th>
            </tr>
          </thead>
          <tbody>
            {PROOFS.map((p) => (
              <tr key={p.claim}>
                <th scope="row" data-label="Claim">
                  {p.claim}
                </th>
                <td data-label="Evidence">
                  <code>{p.evidence}</code>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section id="limits" aria-labelledby="limits-title" className="limits">
        <div className="section-head">
          <h2 id="limits-title">Limits</h2>
        </div>
        <ul>
          <li>Re-randomized FROST over RedPallas has not been audited yet.</li>
          <li>
            Credit lines, OTC settlement and recovery are simulations: the escrow, its keys and the policy signer exist;
            their rules are not written yet.
          </li>
          <li>
            The policy signer runs with a development attestation until it moves into a TDX enclave; on Base Sepolia a
            mock verifier accepts that attestation.
          </li>
          <li>ZEC/USD is a median of five exchanges until an authenticated oracle is wired in.</li>
          <li>Lending is licensed: Lentic is built for commercial borrowers and professional lenders, outside the EU.</li>
        </ul>
      </section>
    </main>
  );
}
