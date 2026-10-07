import { useState } from "react";

import { POLICIES } from "./policies";

const ROLES = [
  { name: "Holder", who: "borrower, seller, owner" },
  { name: "Counterparty", who: "lender, buyer, heir" },
  { name: "Policy signer", who: "attested, run by Lentic" },
];

/** Pick keys and see what they can do in every policy. */
export function KeyPicker() {
  const [picked, setPicked] = useState<[boolean, boolean, boolean]>([false, true, true]);
  const count = picked.filter(Boolean).length;
  const toggle = (i: number) => setPicked((p) => p.map((v, j) => (j === i ? !v : v)) as [boolean, boolean, boolean]);

  return (
    <div className="key-picker">
      <div className="key-buttons" role="group" aria-label="Keys">
        {ROLES.map((r, i) => (
          <button
            key={r.name}
            type="button"
            className={picked[i] ? "key-button on" : "key-button"}
            aria-pressed={picked[i]}
            onClick={() => toggle(i)}
          >
            <span className={picked[i] ? "key-mark acted" : "key-mark"} aria-hidden />
            <span className="key-name">{r.name}</span>
            <span className="key-who">{r.who}</span>
          </button>
        ))}
      </div>

      <div className="key-result" key={picked.join()}>
        {count === 0 && <p className="key-verdict">Pick two keys.</p>}
        {count === 1 && (
          <p className="key-verdict">One key alone moves nothing, in any policy. That includes the policy signer Lentic runs.</p>
        )}
        {count === 3 && <p className="key-verdict">All three can do anything; any two are already enough.</p>}
        {count === 2 && (
          <table className="ledger">
            <thead>
              <tr>
                <th scope="col">Policy</th>
                <th scope="col">These keys belong to</th>
                <th scope="col">Together they can</th>
              </tr>
            </thead>
            <tbody>
              {POLICIES.map((p) => {
                const rule = p.rules.find((r) => r.keys.every((k, i) => k === picked[i]));
                const names = [p.parties.a, p.parties.b, "the policy signer"].filter((_, i) => picked[i]);
                return (
                  <tr key={p.id}>
                    <th scope="row" data-label="Policy">
                      {p.name}
                      {!p.live && <span className="state-tag">Simulation</span>}
                    </th>
                    <td data-label="Keys of">{names.join(" and ")}</td>
                    <td data-label="Can">{rule?.can ?? "Nothing"}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
