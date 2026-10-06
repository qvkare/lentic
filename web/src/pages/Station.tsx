import { useState, type FormEvent } from "react";
import type { Hex } from "viem";

import { Hydrograph } from "../components/Hydrograph";
import { KeyTurnLedger, VisibilityTable } from "../components/Ledgers";
import { StaffGauge } from "../components/StaffGauge";
import { deployment, keyTurns, readLoan, type Loan } from "../chain";
import {
  HEALTH_SENTENCE,
  STAGE_SENTENCE,
  clock,
  isFlood,
  pct,
  stageOf,
  stagePrice,
  usd,
  usdc,
  when,
  zec,
} from "../format";
import { usePoll, useWidth } from "../hooks";
import { PRICE_SOURCE, policyGauge, policyInfo, setTestPrice } from "../policy";

export function Station({ loanId }: { loanId: Hex }) {
  const d = deployment()!;
  const loan = usePoll(() => readLoan(d.loanBook, loanId), [loanId], 4000);
  const gauge = usePoll(() => policyGauge(loanId), [loanId], 4000);
  const info = usePoll(policyInfo, [], 15_000);
  const turns = usePoll(
    () => (loan.data ? keyTurns(d, loan.data) : Promise.resolve([])),
    [loanId, loan.data?.updatedAt],
    8000,
  );
  const gaugeBox = useWidth<HTMLDivElement>();
  const graphBox = useWidth<HTMLDivElement>();

  const g = gauge.data;
  const latest = g?.readings[g.readings.length - 1];
  const closed = loan.data?.health === "closed";
  const ltv = closed ? null : (latest?.ltvBps ?? null);
  const stage = g && ltv !== null ? stageOf(ltv, g.actionStageBps, g.floodStageBps) : null;
  const flooded = stage === "flood" || (loan.data ? isFlood(loan.data.health) : false);
  const action = g?.actionStageBps ?? 6000;
  const flood = g?.floodStageBps ?? 7500;

  if (loan.data === null) {
    return (
      <main>
        <h1>No station with this loan id</h1>
        <p className="lede">
          LoanBook has no loan <span className="mono">{loanId}</span>. Check the id, or go back to the{" "}
          <a href="#/book">lender book</a>.
        </p>
      </main>
    );
  }

  return (
    <main>
      <header className="page-head">
        <h1>
          Station <span className="mono id">{loanId.slice(0, 10)}…{loanId.slice(-6)}</span>
        </h1>
        <p className="lede">
          The collateral sits in a 2-of-3 shielded escrow on Zcash. Any two of the borrower, the lender and the policy
          signer can move it. Lentic runs only the policy signer.
        </p>
      </header>

      <section className="station" aria-label="Gauge and readings">
        <div className="gauge-col" ref={gaugeBox.ref}>
          {gaugeBox.width > 0 && (
            <StaffGauge
              ltvBps={ltv}
              actionBps={action}
              floodBps={flood}
              width={gaugeBox.width}
              height={gaugeBox.width > 300 ? 440 : 380}
            />
          )}
          <p className="caption">Water line = loan-to-value. Action stage = margin call. Flood stage = liquidation.</p>
        </div>

        <div className="readings">
          {gauge.error && (
            <p className="notice" role="status">
              {gauge.error} Showing the last report on Base.
            </p>
          )}
          {closed ? (
            <p className="reading-stage">Closed. The collateral went back to the borrower's shielded address.</p>
          ) : latest && g ? (
            <>
              <p className="ltv-label">Loan-to-value</p>
              <p className={flooded ? "ltv flood-text" : "ltv"}>{ltv === null ? "" : pct(ltv)}</p>
              <p className={flooded ? "reading-stage flood-text" : "reading-stage"}>
                {loan.data?.health === "liquidated" ? "Liquidated." : STAGE_SENTENCE[stage ?? "below"]}
              </p>
              <dl className="readings-list">
                <dt>ZEC price at flood stage</dt>
                <dd>{fmtPrice(stagePrice(latest.debt, latest.collateralZat, flood))}</dd>
                <dt>ZEC price at action stage</dt>
                <dd>{fmtPrice(stagePrice(latest.debt, latest.collateralZat, action))}</dd>
                <dt>Debt</dt>
                <dd>
                  {usdc(latest.debt)}
                  <span className="sub">accrued interest, less repayments</span>
                </dd>
                <dt>Collateral</dt>
                <dd>
                  {zec(latest.collateralZat)}
                  <span className="sub">
                    {g.collateralSource === "terms" ? "committed in the terms" : "held by the escrow"}
                  </span>
                </dd>
                <dt>ZEC/USD</dt>
                <dd>
                  {usd(latest.priceUsd)}
                  <span className="sub">
                    {PRICE_SOURCE[latest.priceKind]} at {clock(latest.round)}
                  </span>
                </dd>
                <dt>Last report on Base</dt>
                <dd>
                  {loan.data && <LastReport loan={loan.data} />}
                </dd>
              </dl>
            </>
          ) : (
            <>
              <p className="reading-stage">No reading yet. The policy signer takes one every round.</p>
              {loan.data && (
                <dl className="readings-list">
                  <dt>Last report on Base</dt>
                  <dd>
                    <LastReport loan={loan.data} />
                  </dd>
                </dl>
              )}
            </>
          )}
          {info.data?.devRoutes && !closed && <TestPrice onDone={gauge.refresh} />}
        </div>
      </section>

      <section aria-labelledby="hydro">
        <h2 id="hydro">Hydrograph</h2>
        <p className="lede small">
          Loan-to-value at each round since the policy signer started
          {g ? `, one round every ${g.roundSecs} s` : ""}.
        </p>
        <div className="graph" ref={graphBox.ref}>
          {graphBox.width > 0 && (
            <Hydrograph readings={g?.readings ?? []} actionBps={action} floodBps={flood} width={graphBox.width} />
          )}
        </div>
      </section>

      <section aria-labelledby="keys">
        <h2 id="keys">Key turns</h2>
        <p className="lede small">
          Recorded on Base. A filled mark means that key acted; a ring means it was not needed.
        </p>
        {turns.error ? <p className="notice">{turns.error}</p> : <KeyTurnLedger turns={turns.data ?? []} />}
      </section>

      <section aria-labelledby="public">
        <h2 id="public">What is public</h2>
        <VisibilityTable />
      </section>
    </main>
  );
}

const fmtPrice = (v: number | null) => (v === null ? "" : `${usd(v)} per ZEC`);

/** The policy signer's last report; a loan that was only opened has none yet. */
function LastReport({ loan }: { loan: Loan }) {
  if (loan.oracleRound === 0n && loan.health === "active") {
    return (
      <>
        None yet<span className="sub">opened {when(loan.openedAt)}</span>
      </>
    );
  }
  return (
    <>
      <span className={isFlood(loan.health) ? "flood-text" : undefined}>{HEALTH_SENTENCE[loan.health]}</span>
      <span className="sub">{when(loan.updatedAt)}</span>
    </>
  );
}

function TestPrice({ onDone }: { onDone: () => void }) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const run = async (price: number | null) => {
    setBusy(true);
    setMessage(null);
    try {
      const r = await setTestPrice(price);
      setMessage(price === null ? `Back to the exchange price: ${usd(r.usd)}.` : `Test price set to ${usd(r.usd)}.`);
      onDone();
    } catch (e) {
      setMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const submit = (e: FormEvent) => {
    e.preventDefault();
    const n = Number(value);
    if (!Number.isFinite(n) || n <= 0) {
      setMessage("Enter a price above zero, in USD per ZEC.");
      return;
    }
    run(n);
  };

  return (
    <form className="panel test-price" onSubmit={submit}>
      <label className="field">
        <span>Test price (local only)</span>
        <span className="field-unit">
          <input
            inputMode="decimal"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder="650"
            aria-describedby="test-price-help"
          />
          <span className="unit">USD</span>
        </span>
      </label>
      <div className="actions">
        <button className="btn" type="submit" disabled={busy}>
          Set test price
        </button>
        <button className="btn secondary" type="button" disabled={busy} onClick={() => run(null)}>
          Use exchange price
        </button>
      </div>
      <p id="test-price-help" className="sub">
        {message ??
          "The policy signer reads this instead of the exchanges until you clear it, and reports the resulting stage to LoanBook."}
      </p>
    </form>
  );
}
