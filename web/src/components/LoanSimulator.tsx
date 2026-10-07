import { useState } from "react";

import { STAGE_SENTENCE, liquidationSplit, ltvAt, pct, stageOf, stagePrice, usd, zec } from "../format";
import { useWidth } from "../hooks";
import { StaffGauge } from "./StaffGauge";

export type SimTerms = { actionBps: number; floodBps: number; haircutBps: number; penaltyBps: number };

/** Demo terms the local lender offers; printed under the simulator. */
const APR = 0.12;
const ORIGINATION = 0.01;
const OPEN_LIMIT = 0.5;

const usdc = (v: number) => `${v.toLocaleString("en-US", { maximumFractionDigits: 0 })} USDC`;

function Slider(props: {
  id: string;
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  display: string;
  onChange: (v: number) => void;
}) {
  return (
    <div className="slider">
      <div className="slider-head">
        <label htmlFor={props.id}>{props.label}</label>
        <output htmlFor={props.id} className="slider-value">
          {props.display}
        </output>
      </div>
      <input
        id={props.id}
        type="range"
        min={props.min}
        max={props.max}
        step={props.step}
        value={props.value}
        aria-valuetext={props.display}
        onChange={(e) => props.onChange(Number(e.target.value))}
      />
    </div>
  );
}

/** A borrower's view of a loan before it exists: what to lock, what to draw, where the floods are. */
export function LoanSimulator({ price, priceLabel, terms }: { price: number; priceLabel: string; terms: SimTerms }) {
  const [zecAmount, setZecAmount] = useState(20);
  const [borrow, setBorrow] = useState(10_000);
  const [shock, setShock] = useState(0);
  const box = useWidth<HTMLDivElement>();

  const value = zecAmount * price;
  const maxBorrow = Math.max(100, Math.floor((value * OPEN_LIMIT) / 100) * 100);
  const b = Math.min(borrow, maxBorrow);
  const zat = zecAmount * 1e8;
  const debt = b * 1e6;
  const shocked = price * (1 + shock / 100);
  const ltvNow = ltvAt(debt, zat, price);
  const ltv = ltvAt(debt, zat, shocked);
  const stage = stageOf(ltv, terms.actionBps, terms.floodBps);
  const floodPrice = stagePrice(debt, zat, terms.floodBps) ?? 0;
  const actionPrice = stagePrice(debt, zat, terms.actionBps) ?? 0;
  const drop = (p: number) => `${((1 - p / price) * 100).toFixed(1)} % below today`;
  const split = liquidationSplit(zat, debt, shocked, terms.haircutBps, terms.penaltyBps);

  return (
    <div className="simulator">
      <div className="sim-controls">
        <Slider
          id="sim-zec"
          label="ZEC you lock in the escrow"
          value={zecAmount}
          min={1}
          max={200}
          step={1}
          display={`${zecAmount} ZEC`}
          onChange={setZecAmount}
        />
        <Slider
          id="sim-borrow"
          label="USDC you borrow"
          value={b}
          min={0}
          max={maxBorrow}
          step={100}
          display={usdc(b)}
          onChange={setBorrow}
        />
        <p className="note">
          Up to 50 % of the collateral's value at opening: {usdc(maxBorrow)} today.
        </p>
        <Slider
          id="sim-shock"
          label="ZEC/USD moves by"
          value={shock}
          min={-70}
          max={50}
          step={1}
          display={`${shock > 0 ? "+" : ""}${shock} %: ${usd(shocked)}`}
          onChange={setShock}
        />
        <p className="note">Today's price: {priceLabel}.</p>
      </div>

      <div className="sim-gauge" ref={box.ref}>
        {box.width > 0 && (
          <StaffGauge
            ltvBps={Math.min(ltv, 10_001)}
            actionBps={terms.actionBps}
            floodBps={terms.floodBps}
            width={box.width}
            height={340}
          />
        )}
      </div>

      <div className="sim-readout">
        <p className="ltv-label">Loan-to-value</p>
        <p className={stage === "flood" ? "ltv flood-text" : "ltv"}>{pct(ltv)}</p>
        <p className={stage === "flood" ? "reading-stage flood-text" : "reading-stage"}>{STAGE_SENTENCE[stage]}</p>
        <dl className="readings-list">
          <dt>At opening</dt>
          <dd>{pct(ltvNow)}</dd>
          <dt>Margin call at</dt>
          <dd>
            {usd(actionPrice)}
            <span className="sub">{drop(actionPrice)}</span>
          </dd>
          <dt>Liquidation at</dt>
          <dd>
            {usd(floodPrice)}
            <span className="sub">{drop(floodPrice)}</span>
          </dd>
          <dt>Interest</dt>
          <dd>
            {usdc((b * APR) / 12)} a month
            <span className="sub">12 % APR, plus {usdc(b * ORIGINATION)} at opening</span>
          </dd>
          {stage === "flood" && (
            <>
              <dt className="flood-text">If liquidated here</dt>
              <dd className="flood-text">
                {zec(split.lenderZat)} to the lender
                <span className="sub">{zec(Math.max(split.borrowerZat, 0))} back to you, shielded</span>
              </dd>
            </>
          )}
        </dl>
      </div>
      <p className="note sim-terms">
        Simulation with the local lender's demo terms: margin call at {terms.actionBps / 100} %, liquidation at{" "}
        {terms.floodBps / 100} %, a {terms.haircutBps / 100} % haircut and a {terms.penaltyBps / 100} % penalty. Opening a
        real loan today takes the signer CLI and a lender.
      </p>
    </div>
  );
}
