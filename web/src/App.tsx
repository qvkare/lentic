import { useState } from "react";
import type { Hex } from "viem";

import { CHAIN_ID, RPC_URL, deployment, healthHistory, listLoans, readLoan, vaultSummary, type Health } from "./chain";
import { HEALTH_LABEL, short, time, usdc } from "./format";
import { usePoll } from "./hooks";
import { policyInfo } from "./policy";

type Tab = "lender" | "borrower";

export function App() {
  const [tab, setTab] = useState<Tab>("lender");
  const d = deployment();

  return (
    <div className="shell">
      <header className="top">
        <div className="brand">
          <span className="mark" aria-hidden />
          <div>
            <h1>Lentic</h1>
            <p>USDC against shielded ZEC that never leaves the pool</p>
          </div>
        </div>
        <nav className="tabs" role="tablist">
          <button role="tab" aria-selected={tab === "lender"} onClick={() => setTab("lender")}>
            Lender
          </button>
          <button role="tab" aria-selected={tab === "borrower"} onClick={() => setTab("borrower")}>
            Borrower
          </button>
        </nav>
      </header>

      {!d ? (
        <section className="card">
          <h2>No deployment for chain {CHAIN_ID}</h2>
          <p className="muted">
            Run <code>pnpm local:chain</code> and <code>pnpm local:deploy</code>, then reload.
          </p>
        </section>
      ) : tab === "lender" ? (
        <LenderView />
      ) : (
        <BorrowerView />
      )}

      <footer className="foot muted">
        Chain {CHAIN_ID} · {RPC_URL} · LoanBook {d ? short(d.loanBook) : "—"}
      </footer>
    </div>
  );
}

function HealthBadge({ health }: { health: Health }) {
  return <span className={`badge h-${health}`}>{HEALTH_LABEL[health]}</span>;
}

function PolicyCard() {
  const { data, error } = usePoll(policyInfo, []);
  return (
    <section className="card">
      <h2>Policy signer</h2>
      <p className="muted">The only key share Lentic operates (1 of 3). Its code identity is checked on Base.</p>
      {error && <p className="error">{error}</p>}
      {data && (
        <dl className="kv">
          <dt>Signer</dt>
          <dd className="mono">{short(data.signerAddress)}</dd>
          <dt>Attestation</dt>
          <dd>
            {data.registeredOnBase ? <span className="badge h-active">Registered</span> : <span className="badge h-liquidating">Not registered</span>}{" "}
            <span className="muted">via {data.attestor === "dstack" ? "TDX quote (dstack)" : "dev quote"}</span>
          </dd>
          <dt>Measurement</dt>
          <dd className="mono">{short(data.measurement, 8)}</dd>
          <dt>Zcash</dt>
          <dd>{data.zcashNetwork}</dd>
        </dl>
      )}
    </section>
  );
}

function LenderView() {
  const d = deployment()!;
  const vault = usePoll(() => (d.lenderVault ? vaultSummary(d.lenderVault) : Promise.resolve(null)), [d.lenderVault]);
  const loans = usePoll(() => listLoans(d), [d.loanBook]);
  const v = vault.data;

  return (
    <main className="grid">
      <section className="card span2">
        <h2>{v?.name ?? "Lender vault"}</h2>
        <p className="muted">Your own ERC-4626 vault. Lentic runs no pool and never holds funds.</p>
        {vault.error && <p className="error">{vault.error}</p>}
        {v && (
          <div className="stats">
            <Stat label="Total assets" value={usdc(v.totalAssets)} />
            <Stat label="Idle" value={usdc(v.idle)} />
            <Stat label="Lent out" value={usdc(v.outstanding)} />
            <Stat label="Lentic fee" value={`${(v.feeBps / 100).toFixed(0)}% of interest`} />
          </div>
        )}
      </section>
      <PolicyCard />
      <section className="card span3">
        <h2>Loans</h2>
        {loans.error && <p className="error">{loans.error}</p>}
        {loans.data && loans.data.length === 0 && <p className="muted">No loans yet.</p>}
        {loans.data && loans.data.length > 0 && (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Loan</th>
                  <th>Health</th>
                  <th>Terms commitment</th>
                  <th>Opened</th>
                  <th>Oracle round</th>
                </tr>
              </thead>
              <tbody>
                {loans.data.map((l) => (
                  <tr key={l.loanId}>
                    <td className="mono">{short(l.loanId)}</td>
                    <td>
                      <HealthBadge health={l.health} />
                    </td>
                    <td className="mono">{short(l.termsHash)}</td>
                    <td>{time(l.openedAt)}</td>
                    <td>{l.oracleRound.toString()}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </main>
  );
}

function BorrowerView() {
  const d = deployment()!;
  const loans = usePoll(() => listLoans(d), [d.loanBook]);
  const [picked, setPicked] = useState<string>("");
  const loanId = (picked || loans.data?.[0]?.loanId || "") as Hex;
  const valid = /^0x[0-9a-fA-F]{64}$/.test(loanId);
  const loan = usePoll(() => (valid ? readLoan(d.loanBook, loanId) : Promise.resolve(null)), [loanId]);
  const history = usePoll(() => (valid ? healthHistory(d, loanId) : Promise.resolve([])), [loanId]);

  return (
    <main className="grid">
      <section className="card span2">
        <h2>Loan status</h2>
        <label className="field">
          <span>Loan id</span>
          <input
            className="mono"
            placeholder="0x…"
            value={picked || loanId}
            onChange={(e) => setPicked(e.target.value.trim())}
            spellCheck={false}
          />
        </label>
        {loan.error && <p className="error">{loan.error}</p>}
        {valid && loan.data === null && !loan.error && <p className="muted">No loan with this id.</p>}
        {loan.data && (
          <dl className="kv">
            <dt>Health</dt>
            <dd>
              <HealthBadge health={loan.data.health} />
            </dd>
            <dt>Terms commitment</dt>
            <dd className="mono">{short(loan.data.termsHash, 10)}</dd>
            <dt>Lender vault</dt>
            <dd className="mono">{short(loan.data.vault)}</dd>
            <dt>Opened</dt>
            <dd>{time(loan.data.openedAt)}</dd>
            <dt>Last update</dt>
            <dd>{time(loan.data.updatedAt)}</dd>
          </dl>
        )}
        {history.data && history.data.length > 0 && (
          <>
            <h3>Policy signer reports</h3>
            <ol className="timeline">
              {history.data.map((h) => (
                <li key={h.tx}>
                  <HealthBadge health={h.health} /> <span className="muted">round {h.oracleRound.toString()} · block {h.block.toString()}</span>
                </li>
              ))}
            </ol>
          </>
        )}
      </section>
      <section className="card">
        <h2>What is public</h2>
        <ul className="plain">
          <li>
            <strong>Zcash:</strong> that shielded transactions happened. Not the amount, not the escrow, not your return address.
          </li>
          <li>
            <strong>Base, LoanBook:</strong> a hash of the terms, the health flag, a hash of any liquidation attestation.
          </li>
          <li>
            <strong>Base, USDC leg:</strong> the loan payout, interest and repayments are ordinary USDC transfers. Use a fresh
            address for each loan.
          </li>
        </ul>
      </section>
    </main>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="stat">
      <span className="muted">{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
