import { useEffect, useState } from "react";

import {
  STAGE_SENTENCE,
  clock,
  liquidationSplit,
  ltvAt,
  pct,
  pctStage,
  stageOf,
  stagePrice,
  usd,
  usdc,
  zec,
} from "../format";
import { useReducedMotion, useWidth } from "../hooks";
import { PRICE_SOURCE, type PriceKind } from "../policy";
import { StaffGauge } from "./StaffGauge";

/** A loan as the drill needs it: its debt, collateral and terms, plus the live price. */
export type DrillLoan = {
  loanId: string | null;
  debt: number;
  collateralZat: number;
  actionBps: number;
  floodBps: number;
  haircutBps: number;
  penaltyBps: number;
  livePrice: number;
  liveAt: number;
  priceKind: PriceKind;
  /** True when the policy signer could not be reached and a marked example is shown. */
  example: boolean;
  /** On-chain facts of the loan, for the privacy views. */
  termsHash?: string;
  vault?: string;
};

type Phase = "live" | "falling" | "action" | "flood" | "keys" | "split";

type State = {
  phase: Phase;
  price: number;
  round: number;
  lender: boolean;
  policy: boolean;
};

const STEP_MS = 160;

/**
 * The home page's one orchestrated moment (web/DESIGN.md, Motion): the live station, then
 * ZEC/USD lowered in simulated rounds to the action stage and the flood stage, the two keys
 * that liquidate, and the split of the collateral at the flood price.
 */
export function FloodDrill({ loan, onReplay, still = false }: { loan: DrillLoan; onReplay: () => void; still?: boolean }) {
  const reduced = useReducedMotion();
  const box = useWidth<HTMLDivElement>();
  const actionPrice = stagePrice(loan.debt, loan.collateralZat, loan.actionBps) ?? loan.livePrice;
  const floodPrice = stagePrice(loan.debt, loan.collateralZat, loan.floodBps) ?? loan.livePrice;
  const split = liquidationSplit(loan.collateralZat, loan.debt, floodPrice, loan.haircutBps, loan.penaltyBps);
  const startLtv = ltvAt(loan.debt, loan.collateralZat, loan.livePrice);

  const initial: State = { phase: "live", price: loan.livePrice, round: 0, lender: false, policy: false };
  // `still` opens on the last state without playing (the "#/?drill=end" link, screenshots).
  const last: State = { phase: "split", price: Math.min(loan.livePrice, floodPrice), round: 0, lender: true, policy: true };
  const [s, setS] = useState<State>(still ? last : initial);

  useEffect(() => {
    if (reduced || still) return;
    setS(initial);
    const timers: number[] = [];
    let t = 0;
    let round = 0;
    const at = (wait: number, fn: () => void) => {
      t += wait;
      timers.push(window.setTimeout(fn, t));
    };
    const fall = (from: number, to: number, steps: number) => {
      for (let i = 1; i <= steps; i++) {
        const price = from + ((to - from) * i) / steps;
        const r = ++round;
        at(STEP_MS, () => setS((p) => ({ ...p, phase: "falling", price, round: r })));
      }
    };

    at(1800, () => undefined);
    let price = loan.livePrice;
    if (startLtv < loan.actionBps) {
      fall(price, actionPrice, 10);
      price = actionPrice;
      at(0, () => setS((p) => ({ ...p, phase: "action" })));
      at(2200, () => undefined);
    }
    if (ltvAt(loan.debt, loan.collateralZat, price) < loan.floodBps) {
      fall(price, floodPrice, 8);
    }
    at(0, () => setS((p) => ({ ...p, phase: "flood", price: Math.min(p.price, floodPrice) })));
    at(1400, () => setS((p) => ({ ...p, phase: "keys", lender: true })));
    at(350, () => setS((p) => ({ ...p, policy: true })));
    at(2000, () => setS((p) => ({ ...p, phase: "split" })));
    return () => timers.forEach((id) => window.clearTimeout(id));
    // The drill restarts only when the loan snapshot changes (Replay fetches a new one).
  }, [reduced, still, loan]);

  const ltv = ltvAt(loan.debt, loan.collateralZat, s.price);
  const flooded = ltv >= loan.floodBps;
  const caption = captionFor(s.phase, loan, actionPrice, floodPrice, split);

  const mode =
    s.phase === "live"
      ? loan.example
        ? "Example reading"
        : `Live reading, ${PRICE_SOURCE[loan.priceKind]} at ${clock(loan.liveAt)}`
      : s.round > 0
        ? `Simulated round ${s.round}, this loan's terms`
        : "Simulated, this loan's terms";

  if (reduced) {
    return (
      <figure className="drill" aria-label="Flood drill">
        <div className="drill-gauge" ref={box.ref}>
          {box.width > 0 && (
            <StaffGauge
              ltvBps={Math.min(startLtv, 10_001)}
              actionBps={loan.actionBps}
              floodBps={loan.floodBps}
              width={box.width}
              height={box.width > 300 ? 320 : 380}
            />
          )}
        </div>
        <div className="drill-read">
          <table className="ledger drill-static">
            <thead>
              <tr>
                <th scope="col">Reading</th>
                <th scope="col" className="num">
                  ZEC/USD
                </th>
                <th scope="col" className="num">
                  Loan-to-value
                </th>
                <th scope="col">What happens</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <th scope="row" data-label="Reading">
                  {loan.example ? "Example" : "Live"}
                </th>
                <td className="num" data-label="ZEC/USD">
                  {usd(loan.livePrice)}
                </td>
                <td className="num" data-label="Loan-to-value">
                  {pct(startLtv)}
                </td>
                <td data-label="What happens">{STAGE_SENTENCE[stageOf(startLtv, loan.actionBps, loan.floodBps)]}</td>
              </tr>
              <tr>
                <th scope="row" data-label="Reading">
                  Simulated
                </th>
                <td className="num" data-label="ZEC/USD">
                  {usd(actionPrice)}
                </td>
                <td className="num" data-label="Loan-to-value">
                  {pctStage(loan.actionBps)}
                </td>
                <td data-label="What happens">{STAGE_SENTENCE.action}</td>
              </tr>
              <tr className="flood-row">
                <th scope="row" data-label="Reading">
                  Simulated
                </th>
                <td className="num" data-label="ZEC/USD">
                  {usd(floodPrice)}
                </td>
                <td className="num" data-label="Loan-to-value">
                  {pctStage(loan.floodBps)}
                </td>
                <td data-label="What happens">
                  The lender and the policy signer sign the escrow spend: {zec(split.lenderZat)} to the lender,{" "}
                  {zec(split.borrowerZat)} back to the borrower.
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </figure>
    );
  }

  return (
    <figure className="drill" aria-label="Flood drill">
      <div className="drill-gauge" ref={box.ref}>
        {box.width > 0 && (
          <StaffGauge
            ltvBps={Math.min(ltv, 10_001)}
            actionBps={loan.actionBps}
            floodBps={loan.floodBps}
            width={box.width}
            height={box.width > 300 ? 320 : 380}
          />
        )}
      </div>

      <div className="drill-read">
        <p className="drill-mode">{mode}</p>
        <dl className="drill-figures">
          <div>
            <dt>ZEC/USD</dt>
            <dd>
              {s.price.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
              <span className="unit-sm">USD</span>
            </dd>
          </div>
          <div>
            <dt>Loan-to-value</dt>
            <dd className={flooded ? "flood-text" : undefined}>{pct(ltv)}</dd>
          </div>
        </dl>
        <p className={flooded ? "drill-caption flood-text" : "drill-caption"} aria-live="polite">
          {caption}
        </p>

        <ul className="drill-keys" aria-label="Keys">
          <li>
            <span className="key-anim" aria-hidden />
            Borrower
          </li>
          <li>
            <span className={s.lender ? "key-anim on" : "key-anim"} aria-hidden />
            Lender
          </li>
          <li>
            <span className={s.policy ? "key-anim on" : "key-anim"} aria-hidden />
            Policy signer
          </li>
        </ul>

        <div className={s.phase === "split" ? "drill-split shown" : "drill-split"} aria-hidden={s.phase !== "split"}>
          <div className="split-bar">
            <span className="split-lender" style={{ flexGrow: split.lenderZat }} />
            <span className="split-borrower" style={{ flexGrow: Math.max(split.borrowerZat, 0) }} />
          </div>
          <div className="split-legend">
            <span className="flood-text">{zec(split.lenderZat)} to the lender</span>
            <span>{zec(Math.max(split.borrowerZat, 0))} back to the borrower</span>
          </div>
        </div>

        <div className="drill-actions">
          <button className="btn secondary" type="button" onClick={onReplay}>
            Replay
          </button>
          {loan.loanId && <a href={`#/station/${loan.loanId}`}>Open this station</a>}
        </div>
      </div>

      <figcaption className="caption">
        {loan.example
          ? "Example loan: "
          : "A live loan held by the policy signer: "}
        {usdc(loan.debt)} against {zec(loan.collateralZat)}. Prices after the first reading are simulated.
      </figcaption>
    </figure>
  );
}

function captionFor(
  phase: Phase,
  loan: DrillLoan,
  actionPrice: number,
  floodPrice: number,
  split: { lenderZat: number; borrowerZat: number },
): string {
  switch (phase) {
    case "live":
      return `${STAGE_SENTENCE[stageOf(ltvAt(loan.debt, loan.collateralZat, loan.livePrice), loan.actionBps, loan.floodBps)]} Now the price falls.`;
    case "falling":
      return "Each round the policy signer reads the price again. The water rises.";
    case "action":
      return `At ${usd(actionPrice)} the water reaches action stage: a margin call. Nothing moves; the borrower can add ZEC or repay.`;
    case "flood":
      return `At ${usd(floodPrice)} it reaches flood stage. The loan can now be liquidated.`;
    case "keys":
      return "The lender and the policy signer sign the escrow spend. The borrower's key is not needed, and Lentic holds only one.";
    case "split":
      return `${zec(split.lenderZat)} covers the debt and the penalty at a haircut price; ${zec(split.borrowerZat)} goes back to the borrower, still shielded.`;
  }
}
