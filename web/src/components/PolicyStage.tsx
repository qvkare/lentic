import { useEffect, useMemo, useState } from "react";

import { useReducedMotion, useWidth } from "../hooks";
import { POLICIES, type PolicyId } from "./policies";
import { LaneStage, useInView, usePlayer } from "./Stage";

/** One escrow, four policies: a tab per policy over the same two-lane stage. */
export function PolicyStage({ price, priceLabel }: { price: number; priceLabel: string }) {
  const [policyId, setPolicyId] = useState<PolicyId>("loan");
  const [branchIndex, setBranchIndex] = useState(0);
  const policy = POLICIES.find((p) => p.id === policyId)!;
  const branch = policy.branches[Math.min(branchIndex, policy.branches.length - 1)];
  const frames = useMemo(() => branch.frames({ price }), [branch, price]);
  const holds = useMemo(() => frames.map((f) => f.hold ?? 2200), [frames]);

  const reduced = useReducedMotion();
  const player = usePlayer(frames.length, holds, reduced);
  const view = useInView<HTMLDivElement>(0.5);
  const box = useWidth<HTMLDivElement>();

  // Play when the stage first comes into view; pause when it leaves; resume on return.
  useEffect(() => {
    if (reduced) return;
    if (view.inView && !player.played) player.play();
    else if (view.inView && !player.finished) player.resume();
    else if (!view.inView) player.pause();
  }, [view.inView, reduced]);

  // A new policy or ending starts from the first frame (once the stage has been seen).
  useEffect(() => {
    if (!reduced && player.played) player.play();
  }, [policyId, branchIndex]);

  const frame = frames[Math.min(player.index, frames.length - 1)];
  const names = [policy.parties.a, policy.parties.b, "Policy signer"];

  return (
    <div className="policy-stage">
      <div className="policy-tabs" role="tablist" aria-label="Policies">
        {POLICIES.map((p) => (
          <button
            key={p.id}
            role="tab"
            aria-selected={p.id === policyId}
            aria-controls="policy-panel"
            className="policy-tab"
            onClick={() => {
              setPolicyId(p.id);
              setBranchIndex(0);
            }}
          >
            <span>{p.name}</span>
            <span className={p.live ? "state-tag live" : "state-tag"}>{p.live ? "Live" : "Simulation"}</span>
          </button>
        ))}
      </div>

      <div className="policy-panel" id="policy-panel" role="tabpanel">
        <div className="policy-rules">
          <h3>
            {policy.name} <span className={policy.live ? "state-tag live" : "state-tag"}>{policy.live ? "Live" : "Simulation"}</span>
          </h3>
          <p className="policy-summary">{policy.summary}</p>
          <dl className="readings-list">
            <dt>Keys</dt>
            <dd>
              {policy.parties.a}, {policy.parties.b.toLowerCase()} and the policy signer
            </dd>
            <dt>The signer watches</dt>
            <dd>{policy.watches}</dd>
          </dl>
          <table className="ledger rules">
            <thead>
              <tr>
                {names.map((n) => (
                  <th key={n} scope="col" className="key-col">
                    {n}
                  </th>
                ))}
                <th scope="col">Can</th>
              </tr>
            </thead>
            <tbody>
              {policy.rules.map((r) => (
                <tr key={r.can}>
                  {r.keys.map((k, i) => (
                    <td key={names[i]} className="key-col" data-label={names[i]}>
                      <span className={k ? "key-mark acted" : "key-mark"} role="img" aria-label={k ? "signs" : "not needed"} />
                    </td>
                  ))}
                  <th scope="row" data-label="Can">
                    {r.can}
                  </th>
                </tr>
              ))}
              <tr>
                <td className="key-col" data-label={names[0]}>
                  <span className="key-mark" role="img" aria-label="one key" />
                </td>
                <td className="key-col" data-label={names[1]}>
                  <span className="key-mark" role="img" aria-label="one key" />
                </td>
                <td className="key-col" data-label={names[2]}>
                  <span className="key-mark" role="img" aria-label="one key" />
                </td>
                <th scope="row" data-label="Can">
                  Any one key alone: nothing
                </th>
              </tr>
            </tbody>
          </table>
          {!policy.live && (
            <p className="note">
              Simulation. The escrow, its keys and the policy signer exist today; these rules are not written yet.
            </p>
          )}
        </div>

        <div className="policy-sim" ref={view.ref}>
          <div className="sim-head">
            {policy.branches.length > 1 && (
              <div className="branch-buttons" role="group" aria-label="Endings">
                {policy.branches.map((b, i) => (
                  <button
                    key={b.id}
                    type="button"
                    className={i === branchIndex ? "seg on" : "seg"}
                    aria-pressed={i === branchIndex}
                    onClick={() => setBranchIndex(i)}
                  >
                    {b.label}
                  </button>
                ))}
              </div>
            )}
            <span className="sim-step">
              Step {Math.min(player.index, frames.length - 1) + 1} of {frames.length}
            </span>
          </div>
          <div className="sim-canvas" ref={box.ref}>
            {box.width > 0 && (
              <LaneStage
                frame={frame}
                prev={player.index > 0 ? frames[player.index - 1] : undefined}
                width={box.width}
                parties={policy.parties}
              />
            )}
          </div>
          <p className={frame.liquidation ? "sim-caption flood-text" : "sim-caption"} aria-live="polite">
            {frame.caption}
          </p>
          {reduced && (
            <ol className="sim-steps">
              {frames.map((f) => (
                <li key={f.caption}>{f.caption}</li>
              ))}
            </ol>
          )}
          <div className="sim-foot">
            {!reduced && (
              <button type="button" className="btn secondary" onClick={() => player.play()}>
                Replay
              </button>
            )}
            <span className="note">
              {policy.id === "recovery" ? "Simulated run, time compressed." : `Simulated run. ZEC/USD ${priceLabel}.`}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
