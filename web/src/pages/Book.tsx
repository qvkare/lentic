import { useState, type FormEvent } from "react";

import { MiniGauge } from "../components/StaffGauge";
import { deployment, listLoans, vaultSummary, type Loan } from "../chain";
import { HEALTH_SENTENCE, clock, isFlood, pct, short, usd, usdcWhole, when } from "../format";
import { usePoll } from "../hooks";
import { PRICE_SOURCE, policyGauge, policyInfo, policyPrice, type Gauge } from "../policy";

type Row = { loan: Loan; gauge: Gauge | null };

async function stations(): Promise<Row[]> {
  const d = deployment()!;
  const loans = await listLoans(d);
  return Promise.all(loans.map(async (loan) => ({ loan, gauge: await policyGauge(loan.loanId).catch(() => null) })));
}

export function Book() {
  const d = deployment()!;
  const vault = usePoll(() => (d.lenderVault ? vaultSummary(d.lenderVault) : Promise.resolve(null)), [d.lenderVault]);
  const rows = usePoll(stations, [], 5000);
  const info = usePoll(policyInfo, [], 15_000);
  const price = usePoll(policyPrice, [], 5000);
  const v = vault.data;

  return (
    <main>
      <header className="page-head">
        <h1>Lender book</h1>
        <p className="lede">
          Every loan is a gauge station. Its water line is the loan-to-value; at flood stage the lender and the policy
          signer may liquidate.
        </p>
      </header>

      <section aria-labelledby="vault">
        <h2 id="vault">{v?.name ?? "Lender vault"}</h2>
        <p className="lede small">Your own vault on Base. Lentic runs no pool and holds no funds.</p>
        {vault.error && <p className="notice">{vault.error}</p>}
        <dl className="figures">
          <div>
            <dt>Total assets</dt>
            <dd>{v ? usdcWhole(v.totalAssets) : ""}</dd>
          </div>
          <div>
            <dt>Idle</dt>
            <dd>{v ? usdcWhole(v.idle) : ""}</dd>
          </div>
          <div>
            <dt>Lent out</dt>
            <dd>{v ? usdcWhole(v.outstanding) : ""}</dd>
          </div>
          <div>
            <dt>Lentic fee</dt>
            <dd>{v ? `${v.feeBps / 100} % of interest` : ""}</dd>
          </div>
        </dl>
      </section>

      <section aria-labelledby="stations">
        <h2 id="stations">Stations</h2>
        {rows.error && <p className="notice">{rows.error}</p>}
        {rows.data && rows.data.length === 0 && (
          <p className="quiet">No loans yet. A station appears here when your vault pays out a loan.</p>
        )}
        {rows.data && rows.data.length > 0 && (
          <table className="ledger stations">
            <thead>
              <tr>
                <th scope="col">Station</th>
                <th scope="col" className="gauge-cell">
                  <span className="visually-hidden">Gauge</span>
                </th>
                <th scope="col" className="num">
                  Loan-to-value
                </th>
                <th scope="col">Stage</th>
                <th scope="col" className="num">
                  Last report on Base
                </th>
              </tr>
            </thead>
            <tbody>{rows.data.map((r) => <StationRow key={r.loan.loanId} row={r} />)}</tbody>
          </table>
        )}
        <OpenStation />
      </section>

      <section aria-labelledby="policy">
        <h2 id="policy">Policy signer</h2>
        <p className="lede small">
          The third key of every escrow, running in a TEE. Its code identity is checked on Base before LoanBook accepts
          its reports.
        </p>
        {info.error && <p className="notice">{info.error}</p>}
        <dl className="readings-list wide">
          <dt>Signer</dt>
          <dd className="mono">{info.data?.signerAddress ?? ""}</dd>
          <dt>Attestation</dt>
          <dd>
            {info.data
              ? info.data.registeredOnBase
                ? `Registered on Base from a ${info.data.attestor === "dstack" ? "TDX quote" : "development quote"}`
                : "Not registered on Base"
              : ""}
          </dd>
          <dt>Measurement</dt>
          <dd className="mono">{info.data ? short(info.data.measurement, 10) : ""}</dd>
          <dt>ZEC/USD</dt>
          <dd>
            {price.data ? usd(price.data.usd) : ""}
            {price.data && (
              <span className="sub">
                {PRICE_SOURCE[price.data.kind]} at {clock(price.data.at)}
              </span>
            )}
          </dd>
          <dt>Rounds</dt>
          <dd>
            {info.data
              ? `Every ${info.data.roundSecs} s for ${info.data.loansHeld} loan${info.data.loansHeld === 1 ? "" : "s"}${
                  info.data.reportsHealth ? ", stage changes written to LoanBook" : ""
                }`
              : ""}
          </dd>
        </dl>
      </section>
    </main>
  );
}

function StationRow({ row }: { row: Row }) {
  const { loan, gauge } = row;
  const latest = gauge?.readings[gauge.readings.length - 1];
  const ltv = loan.health === "closed" ? null : (latest?.ltvBps ?? null);
  const flooded = isFlood(loan.health) || (ltv !== null && gauge !== null && ltv >= gauge.floodStageBps);
  return (
    <tr className={flooded ? "flood-row" : undefined}>
      <th scope="row" data-label="Station">
        <a className="mono" href={`#/station/${loan.loanId}`}>
          {short(loan.loanId, 6)}
        </a>
      </th>
      <td className="gauge-cell" data-label="Gauge">
        <MiniGauge ltvBps={ltv} actionBps={gauge?.actionStageBps ?? 6000} floodBps={gauge?.floodStageBps ?? 7500} />
      </td>
      <td className="num" data-label="Loan-to-value">
        {ltv === null ? "No reading" : pct(ltv)}
      </td>
      <td data-label="Stage">{HEALTH_SENTENCE[loan.health]}</td>
      <td className="num" data-label="Last report on Base">
        {loan.oracleRound === 0n && loan.health === "active" ? "None yet" : when(loan.updatedAt)}
      </td>
    </tr>
  );
}

function OpenStation() {
  const [id, setId] = useState("");
  const [error, setError] = useState<string | null>(null);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const v = id.trim();
    if (!/^0x[0-9a-fA-F]{64}$/.test(v)) {
      setError("A loan id is 0x followed by 64 hex characters.");
      return;
    }
    window.location.hash = `#/station/${v}`;
  };
  return (
    <form className="panel open-station" onSubmit={submit}>
      <label className="field">
        <span>Open a station by loan id</span>
        <input
          className="mono"
          value={id}
          onChange={(e) => {
            setId(e.target.value);
            setError(null);
          }}
          placeholder="0x…"
          spellCheck={false}
          aria-invalid={error !== null}
        />
      </label>
      <button className="btn secondary" type="submit">
        Open station
      </button>
      {error && <p className="sub">{error}</p>}
    </form>
  );
}
