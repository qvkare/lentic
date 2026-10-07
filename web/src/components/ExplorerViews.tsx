import { useState } from "react";

import { short } from "../format";

export type ExplorerLoan = { loanId: string; termsHash: string; vault: string; debt: number } | null;

type View = "zcash" | "lender" | "base";

/** The same loan seen three ways: a public Zcash explorer, the lender's viewing key, Base. */
export function ExplorerViews({ loan }: { loan: ExplorerLoan }) {
  const [view, setView] = useState<View>("zcash");
  const tabs: { id: View; label: string }[] = [
    { id: "zcash", label: "Public Zcash explorer" },
    { id: "lender", label: "Lender, with the viewing key" },
    { id: "base", label: "Base explorer" },
  ];

  return (
    <div className="explorer">
      <div className="branch-buttons" role="tablist" aria-label="Views">
        {tabs.map((t) => (
          <button
            key={t.id}
            role="tab"
            aria-selected={view === t.id}
            className={view === t.id ? "seg on" : "seg"}
            onClick={() => setView(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>

      <div className="explorer-record panel" key={view} role="tabpanel">
        {view === "zcash" && (
          <>
            <p className="record-head">
              Transaction <span className="mono">3968785d…fda54542</span> <span className="state-tag">Example</span>
            </p>
            <dl className="readings-list">
              <dt>Format</dt>
              <dd>v6, Ironwood shielded pool</dd>
              <dt>Shielded actions</dt>
              <dd>2</dd>
              <dt>Amount</dt>
              <dd className="hidden-value">Not visible</dd>
              <dt>Sender and recipient</dt>
              <dd className="hidden-value">Not visible</dd>
              <dt>Fee</dt>
              <dd>0.0001 ZEC</dd>
            </dl>
            <p className="note">Anyone sees that a shielded transaction happened, and nothing else.</p>
          </>
        )}
        {view === "lender" && (
          <>
            <p className="record-head">
              Escrow deposit, decrypted <span className="state-tag">Example</span>
            </p>
            <dl className="readings-list">
              <dt>Received</dt>
              <dd>20.0000 ZEC</dd>
              <dt>Pool</dt>
              <dd>Ironwood</dd>
              <dt>Spent</dt>
              <dd>No, still in the escrow</dd>
              <dt>Why it is visible</dt>
              <dd>The escrow's viewing key, shared by its three keyholders when the keys were made</dd>
            </dl>
            <p className="note">Visible to the borrower, the lender and the policy signer. Never movable by one of them.</p>
          </>
        )}
        {view === "base" && (
          <>
            <p className="record-head">
              LoanBook and USDC <span className={loan ? "state-tag live" : "state-tag"}>{loan ? "Live" : "Example"}</span>
            </p>
            <dl className="readings-list">
              <dt>Loan</dt>
              <dd className="mono">{loan ? short(loan.loanId, 6) : "0x6753a7…008dde"}</dd>
              <dt>Terms</dt>
              <dd>
                A hash only: <span className="mono">{loan ? short(loan.termsHash, 6) : "0xf168cf…ced2a9"}</span>
              </dd>
              <dt>USDC payout</dt>
              <dd>
                {loan
                  ? `${(loan.debt / 1e6).toLocaleString("en-US", { maximumFractionDigits: 0 })} USDC`
                  : "10,000 USDC"}{" "}
                from the vault <span className="mono">{loan ? short(loan.vault) : "0x8464…18bC"}</span>
              </dd>
              <dt>Health</dt>
              <dd>Reported by the attested policy signer</dd>
            </dl>
            <p className="note">
              The USDC leg is an ordinary transfer, so its size is public. Use a fresh Base address for each loan.
            </p>
          </>
        )}
      </div>
    </div>
  );
}
