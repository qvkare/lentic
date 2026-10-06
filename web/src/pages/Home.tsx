import { useCallback, useEffect, useState } from "react";

import { FloodDrill, type DrillLoan } from "../components/FloodDrill";
import { VisibilityTable } from "../components/Ledgers";
import { deployment, listLoans } from "../chain";
import { ltvAt } from "../format";
import { policyGauge, policyPrice } from "../policy";

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

const PAIRS: { keys: [Mark, Mark, Mark]; who: string; can: string }[] = [
  {
    keys: ["acted", "acted", "idle"],
    who: "Borrower and lender",
    can: "Anything they agree on, without the policy signer. A cooperative close.",
  },
  {
    keys: ["idle", "acted", "acted"],
    who: "Lender and policy signer",
    can: "Liquidate, only at flood stage, with the split the policy signer computes from the live price.",
  },
  {
    keys: ["acted", "idle", "acted"],
    who: "Borrower and policy signer",
    can: "Return the collateral, once repayment is seen on Base or if the USDC never arrived.",
  },
  {
    keys: ["idle", "idle", "idle"],
    who: "Any one key, Lentic included",
    can: "Nothing.",
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
  const load = useCallback(() => {
    featuredLoan()
      .then(setLoan)
      .catch(() => setLoan(EXAMPLE));
  }, []);
  useEffect(load, [load]);

  return (
    <main className="home">
      <section className="hero" aria-labelledby="claim">
        <div className="hero-copy">
          <h1 id="claim">The collateral never leaves the pool.</h1>
          <p className="hero-lede">
            Borrow USDC on Base against ZEC that stays shielded on Zcash, in an escrow only two of three keys can move:
            the borrower's, the lender's and an attested policy signer's. Nothing is wrapped, bridged or held by Lentic.
          </p>
          <div className="hero-actions">
            <a className="btn" href="#/book">
              Open the lender book
            </a>
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
            Lentic runs only the policy signer, so Lentic alone can do nothing. Its code is attested on Base before
            LoanBook accepts its reports.
          </p>
        </div>
        <table className="ledger pairs">
          <thead>
            <tr>
              <th scope="col">Together</th>
              <th scope="col" className="key-col">
                Borrower
              </th>
              <th scope="col" className="key-col">
                Lender
              </th>
              <th scope="col" className="key-col">
                Policy signer
              </th>
              <th scope="col">Can</th>
            </tr>
          </thead>
          <tbody>
            {PAIRS.map((p) => (
              <tr key={p.who}>
                <th scope="row" data-label="Together">
                  {p.who}
                </th>
                <Keys keys={p.keys} />
                <td data-label="Can">{p.can}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section aria-labelledby="private">
        <div className="section-head">
          <h2 id="private">What stays private</h2>
          <p className="lede small">Shielded from the public, not from the loan's own parties.</p>
        </div>
        <VisibilityTable />
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
          <li>The gauge uses the collateral committed in the terms until the escrow watcher reads the escrow itself.</li>
          <li>ZEC/USD is a median of five exchanges until an authenticated oracle is wired in.</li>
          <li>Lending is licensed: Lentic is built for commercial borrowers and professional lenders, outside the EU.</li>
        </ul>
      </section>
    </main>
  );
}
