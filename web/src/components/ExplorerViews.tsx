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
              Transaction{" "}
              <a
                className="mono"
                href="https://testnet.zecblock.com/tx/8f77eee426e01da6b2187c3d5a12c0da9a5d3a2dc03b082f11e6178474b20ab5"
              >
                8f77eee4…74b20ab5
              </a>{" "}
              <span className="state-tag live">Testnet</span>
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
            <p className="note">
              A liquidation on Zcash testnet. Anyone sees that a shielded transaction happened, and nothing else.
            </p>
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
              LoanBook and USDC <span className="state-tag live">{loan ? "Live" : "Base Sepolia"}</span>
            </p>
            <dl className="readings-list">
              <dt>Loan</dt>
              <dd className="mono">{loan ? short(loan.loanId, 6) : "0xa3c82d…3e02cc"}</dd>
              <dt>Terms</dt>
              <dd>
                A hash only: <span className="mono">{loan ? short(loan.termsHash, 6) : "0xffd17b…fb5408"}</span>
              </dd>
              <dt>USDC payout</dt>
              <dd>
                {loan
                  ? `${(loan.debt / 1e6).toLocaleString("en-US", { maximumFractionDigits: 0 })} USDC`
                  : "6 USDC"}{" "}
                from the vault <span className="mono">{loan ? short(loan.vault) : "0xfE5c…29f0"}</span>
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
