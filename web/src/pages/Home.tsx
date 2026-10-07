import { useCallback, useEffect, useState } from "react";

import { ExplorerViews } from "../components/ExplorerViews";
import { FloodDrill, type DrillLoan } from "../components/FloodDrill";
import { KeyPicker } from "../components/KeyPicker";
import { VisibilityTable } from "../components/Ledgers";
import { LoanSimulator } from "../components/LoanSimulator";
import { PolicyStage } from "../components/PolicyStage";
import { deployment, listLoans } from "../chain";
import { clock, ltvAt, usd } from "../format";
import { useReducedMotion } from "../hooks";
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
  const reduced = useReducedMotion();
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
  const toTry = () => document.getElementById("try")?.scrollIntoView({ behavior: reduced ? "auto" : "smooth" });

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
            <button className="btn" type="button" onClick={toTry}>
              Try a loan
            </button>
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

      <section aria-labelledby="policies">
        <div className="section-head">
          <h2 id="policies">One escrow, four policies</h2>
          <p className="lede small">
            The keys and the policy signer stay the same; the policy decides what the escrow is for. Loans run today.
            Credit lines, OTC settlement and recovery are simulated here with the same keys.
          </p>
        </div>
        <PolicyStage price={price} priceLabel={priceLabel} />
      </section>

      <section aria-labelledby="try-title" id="try">
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

      <section aria-labelledby="how">
        <div className="section-head">
          <h2 id="how">How a loan runs</h2>
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

      <section aria-labelledby="keys">
        <div className="section-head">
          <h2 id="keys">Two of three keys</h2>
          <p className="lede small">
            Pick keys to see what they can do in each policy. Lentic runs only the policy signer, whose code is attested
            on Base before LoanBook accepts its reports.
          </p>
        </div>
        <KeyPicker />
      </section>

      <section aria-labelledby="private">
        <div className="section-head">
          <h2 id="private">What stays private</h2>
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

      <section aria-labelledby="who">
        <div className="section-head">
          <h2 id="who">Who it is for</h2>
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

      <section aria-labelledby="proven">
        <div className="section-head">
          <h2 id="proven">Proven in code</h2>
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

      <section aria-labelledby="limits" className="limits">
        <div className="section-head">
          <h2 id="limits">Limits</h2>
        </div>
        <ul>
          <li>Re-randomized FROST over RedPallas has not been audited yet.</li>
          <li>
            Credit lines, OTC settlement and recovery are simulations: the escrow, its keys and the policy signer exist;
            their rules are not written yet.
          </li>
          <li>The gauge uses the collateral committed in the terms until the escrow watcher reads the escrow itself.</li>
          <li>ZEC/USD is a median of five exchanges until an authenticated oracle is wired in.</li>
          <li>Lending is licensed: Lentic is built for commercial borrowers and professional lenders, outside the EU.</li>
        </ul>
      </section>
    </main>
  );
}
