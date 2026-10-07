import { EXPLORER_TX, type KeyTurn, type Party } from "../chain";
import { isFlood, when } from "../format";

const PARTIES: { id: Party; label: string }[] = [
  { id: "borrower", label: "Borrower" },
  { id: "lender", label: "Lender" },
  { id: "policy", label: "Policy signer" },
];

function KeyMark({ acted, party }: { acted: boolean; party: string }) {
  return (
    <span className={acted ? "key-mark acted" : "key-mark"} role="img" aria-label={`${party}: ${acted ? "acted" : "not needed"}`} />
  );
}

/** Every recorded event, with which of the three keys acted. */
export function KeyTurnLedger({ turns }: { turns: KeyTurn[] }) {
  if (turns.length === 0) {
    return <p className="quiet">Events appear here as the lender's vault and the policy signer record them on Base.</p>;
  }
  return (
    <table className="ledger key-turns">
      <thead>
        <tr>
          <th scope="col">Event</th>
          {PARTIES.map((p) => (
            <th scope="col" key={p.id} className="key-col">
              {p.label}
            </th>
          ))}
          <th scope="col">Detail</th>
          <th scope="col" className="num">
            Time
          </th>
        </tr>
      </thead>
      <tbody>
        {turns.map((t) => (
          <tr key={t.key} className={t.health && isFlood(t.health) ? "flood-row" : undefined}>
            <th scope="row" data-label="Event">
              {t.event}
            </th>
            {PARTIES.map((p) => (
              <td key={p.id} className="key-col" data-label={p.label}>
                <KeyMark acted={t.acted.includes(p.id)} party={p.label} />
              </td>
            ))}
            <td data-label="Detail">{t.detail}</td>
            <td className="num" data-label="Time">
              {EXPLORER_TX ? <a href={`${EXPLORER_TX}${t.tx}`}>{when(t.time)}</a> : when(t.time)}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

const VISIBILITY: [string, string, string, string][] = [
  ["Collateral amount and escrow address", "No", "Yes, with the viewing key", "Yes"],
  ["Loan terms", "A hash in LoanBook", "Yes", "Yes"],
  ["Health stage", "Yes, in LoanBook", "Yes", "Yes"],
  ["USDC payout, interest and repayments", "Yes, ordinary USDC transfers", "Yes", "Yes"],
  ["Liquidation split", "A hash in LoanBook", "Yes", "Yes"],
];

/** What each audience can see. Shielded does not mean invisible to the parties. */
export function VisibilityTable() {
  return (
    <table className="ledger visibility">
      <thead>
        <tr>
          <th scope="col">What</th>
          <th scope="col">Anyone</th>
          <th scope="col">Borrower and lender</th>
          <th scope="col">Policy signer</th>
        </tr>
      </thead>
      <tbody>
        {VISIBILITY.map(([what, anyone, parties, policy]) => (
          <tr key={what}>
            <th scope="row" data-label="What">
              {what}
            </th>
            <td data-label="Anyone">{anyone}</td>
            <td data-label="Borrower and lender">{parties}</td>
            <td data-label="Policy signer">{policy}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
