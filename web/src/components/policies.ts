import { liquidationSplit, ltvAt, stagePrice } from "../format";
import type { Frame, Parties, Token } from "./Stage";

/**
 * The escrow's policies. The keys and the policy signer are the same in every one; only the
 * rules differ. Loans are built; the others are simulations of rules not written yet.
 */
export type PolicyId = "loan" | "credit" | "otc" | "recovery";

export type Rule = { keys: [boolean, boolean, boolean]; can: string };

export type Branch = { id: string; label: string; frames: (ctx: Ctx) => Frame[] };

export type Policy = {
  id: PolicyId;
  name: string;
  live: boolean;
  summary: string;
  parties: Parties;
  watches: string;
  rules: Rule[];
  branches: Branch[];
};

/** Live ZEC/USD from the policy signer, or a marked example. */
export type Ctx = { price: number };

const fmt = (v: number, digits = 0) =>
  v.toLocaleString("en-US", { minimumFractionDigits: digits, maximumFractionDigits: digits });
const usdcLabel = (v: number) => `${fmt(Math.round(v / 10) * 10)} USDC`;
const zecLabel = (zat: number) => `${fmt(zat / 1e8, 2)} ZEC`;
const usd = (v: number) => `${fmt(v, 2)} USD`;

const ACTION = 6000;
const FLOOD = 7500;

const tok = (id: string, kind: Token["kind"], label: string, at: Token["at"], hidden = false): Token => ({
  id,
  kind,
  label,
  at,
  hidden,
});

// ---------------------------------------------------------------- loan (built)

const LOAN_ZAT = 20e8;
const LOAN_DEBT = 10_000e6;

function loanOpening(ctx: Ctx): Frame[] {
  const ltv = ltvAt(LOAN_DEBT, LOAN_ZAT, ctx.price);
  return [
    {
      caption: "The borrower holds 20 ZEC. The lender's vault on Base holds USDC.",
      clock: "Day 0",
      tokens: [tok("zec", "zec", "20 ZEC", "aZ"), tok("usdc", "usdc", "10,000 USDC", "bBase")],
      keys: [false, false, false],
      basin: { label: "" },
      hold: 1800,
    },
    {
      caption: "Borrower, lender and policy signer make the escrow's three keys. The 20 ZEC moves in, still shielded.",
      clock: "Day 0",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("usdc", "usdc", "10,000 USDC", "bBase")],
      keys: [true, true, true],
      basin: { label: "", gauge: { ltvBps: 0, actionBps: ACTION, floodBps: FLOOD } },
      hold: 2400,
    },
    {
      caption: "The lender's vault pays 10,000 USDC to a fresh Base address of the borrower.",
      clock: "Day 0",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("usdc", "usdc", "10,000 USDC", "aBase")],
      keys: [false, false, false],
      basin: { label: "", gauge: { ltvBps: ltv, actionBps: ACTION, floodBps: FLOOD } },
      hold: 2200,
    },
  ];
}

function loanRepaid(ctx: Ctx): Frame[] {
  const ltv = ltvAt(LOAN_DEBT, LOAN_ZAT, ctx.price);
  const gauge = { ltvBps: ltv, actionBps: ACTION, floodBps: FLOOD };
  return [
    ...loanOpening(ctx),
    {
      caption: `Every round the policy signer reads ZEC/USD (${usd(ctx.price)}) and keeps the loan-to-value at ${fmt(ltv / 100, 1)} %.`,
      clock: "Day 1 to 30",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("usdc", "usdc", "10,000 USDC", "aBase", true)],
      keys: [false, false, false],
      watch: "reads the price",
      basin: { label: "", gauge },
      hold: 2400,
    },
    {
      caption: "The borrower repays 10,098.63 USDC to the vault: principal and 30 days at 12 %.",
      clock: "Day 30",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("repay", "usdc", "10,098.63 USDC", "bBase")],
      keys: [false, false, false],
      watch: "sees the repayment",
      basin: { label: "", gauge: { ...gauge, ltvBps: 0 } },
      hold: 2400,
    },
    {
      caption: "Borrower and policy signer sign the return. The lender's key is not needed.",
      clock: "Day 30",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("repay", "usdc", "10,098.63 USDC", "bBase")],
      keys: [true, false, true],
      basin: { label: "", gauge: { ...gauge, ltvBps: 0 } },
      hold: 2000,
    },
    {
      caption: "The 20 ZEC goes back to the borrower's shielded address. It never left the pool.",
      clock: "Day 30",
      tokens: [tok("zec", "zec", "20 ZEC", "aZ"), tok("repay", "usdc", "10,098.63 USDC", "bBase")],
      keys: [false, false, false],
      basin: { label: "" },
      hold: 2600,
    },
  ];
}

function loanLiquidated(ctx: Ctx): Frame[] {
  const flood = stagePrice(LOAN_DEBT, LOAN_ZAT, FLOOD) ?? ctx.price;
  const action = stagePrice(LOAN_DEBT, LOAN_ZAT, ACTION) ?? ctx.price;
  const split = liquidationSplit(LOAN_ZAT, LOAN_DEBT, flood, 500, 300);
  const gauge = (ltvBps: number) => ({ ltvBps, actionBps: ACTION, floodBps: FLOOD });
  return [
    ...loanOpening(ctx),
    {
      caption: `ZEC falls to ${usd(action)}. The water reaches action stage: a margin call, and nothing moves.`,
      clock: "Day 12",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("usdc", "usdc", "10,000 USDC", "aBase", true)],
      keys: [false, false, false],
      watch: "reads the price",
      basin: { label: "", gauge: gauge(ACTION) },
      hold: 2400,
    },
    {
      caption: `ZEC falls to ${usd(flood)}. Flood stage: the loan can be liquidated.`,
      clock: "Day 14",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("usdc", "usdc", "10,000 USDC", "aBase", true)],
      keys: [false, false, false],
      watch: "reads the price",
      basin: { label: "", gauge: gauge(FLOOD) },
      liquidation: true,
      hold: 2200,
    },
    {
      caption: "Lender and policy signer sign the spend the policy signer computed. The borrower's key is not needed.",
      clock: "Day 14",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("usdc", "usdc", "10,000 USDC", "aBase", true)],
      keys: [false, true, true],
      basin: { label: "", gauge: gauge(FLOOD) },
      liquidation: true,
      hold: 2000,
    },
    {
      caption: `${zecLabel(split.lenderZat)} goes to the lender for the debt and penalty; ${zecLabel(split.borrowerZat)} goes back to the borrower. All of it stays shielded.`,
      clock: "Day 14",
      tokens: [
        tok("zec", "zec", zecLabel(split.lenderZat), "bZ"),
        tok("rest", "zec", zecLabel(split.borrowerZat), "aZ"),
        tok("usdc", "usdc", "10,000 USDC", "aBase", true),
      ],
      keys: [false, false, false],
      basin: { label: "" },
      liquidation: true,
      hold: 2800,
    },
  ];
}

// ---------------------------------------------------------------- credit line (simulation)

function creditLine(ctx: Ctx): Frame[] {
  const zat = 30e8;
  const p0 = ctx.price;
  const value0 = 30 * p0;
  const d1 = Math.round((value0 * 0.3) / 100) * 100;
  const p1 = p0 * 1.15;
  const d2 = Math.round((30 * p1 * 0.48 - d1) / 100) * 100;
  const p2 = p1 * 0.88; // LTV lands between the 50 % draw limit and the 60 % action stage
  const repay = Math.round(((d1 + d2) - 30 * p2 * 0.44) / 100) * 100;
  const g = (debt: number, price: number) => ({
    ltvBps: ltvAt(debt * 1e6, zat, price),
    limitBps: 5000,
    actionBps: ACTION,
    floodBps: FLOOD,
  });
  const room = (debt: number, price: number) => Math.max(0, 30 * price * 0.5 - debt);
  const base = [tok("zec", "zec", "30 ZEC", "escrow")];
  return [
    {
      caption: "The borrower keeps 30 ZEC in the escrow and opens a credit line: draw any time while the water is under 50 %.",
      clock: "Day 0",
      tokens: [...base, tok("d1", "usdc", usdcLabel(d1), "bBase", true), tok("d2", "usdc", usdcLabel(d2), "bBase", true), tok("r", "usdc", usdcLabel(repay), "aBase", true)],
      keys: [true, true, true],
      basin: { label: "", gauge: g(0, p0) },
      hold: 2400,
    },
    {
      caption: `Draw ${usdcLabel(d1)}. The policy signer checks there is room under 50 %; the lender's vault pays.`,
      clock: "Day 2",
      tokens: [...base, tok("d1", "usdc", usdcLabel(d1), "aBase"), tok("d2", "usdc", usdcLabel(d2), "bBase", true), tok("r", "usdc", usdcLabel(repay), "aBase", true)],
      keys: [false, false, false],
      watch: "checks the room",
      basin: { label: "", gauge: g(d1, p0) },
      hold: 2400,
    },
    {
      caption: `ZEC rises to ${usd(p1)}. The water falls; ${usdcLabel(room(d1, p1))} more can be drawn.`,
      clock: "Day 20",
      tokens: [...base, tok("d1", "usdc", usdcLabel(d1), "aBase"), tok("d2", "usdc", usdcLabel(d2), "bBase", true), tok("r", "usdc", usdcLabel(repay), "aBase", true)],
      keys: [false, false, false],
      watch: "reads the price",
      basin: { label: "", gauge: g(d1, p1) },
      hold: 2400,
    },
    {
      caption: `Draw ${usdcLabel(d2)} more, up to the limit.`,
      clock: "Day 21",
      tokens: [...base, tok("d1", "usdc", usdcLabel(d1), "aBase"), tok("d2", "usdc", usdcLabel(d2), "aBase"), tok("r", "usdc", usdcLabel(repay), "aBase", true)],
      keys: [false, false, false],
      watch: "checks the room",
      basin: { label: "", gauge: g(d1 + d2, p1) },
      hold: 2200,
    },
    {
      caption: `ZEC falls to ${usd(p2)}. Above the draw limit: no new draws, and nothing moves until the action stage.`,
      clock: "Day 40",
      tokens: [...base, tok("d1", "usdc", usdcLabel(d1), "aBase"), tok("d2", "usdc", usdcLabel(d2), "aBase"), tok("r", "usdc", usdcLabel(repay), "aBase", true)],
      keys: [false, false, false],
      watch: "reads the price",
      basin: { label: "", gauge: g(d1 + d2, p2) },
      hold: 2600,
    },
    {
      caption: `A partial repayment of ${usdcLabel(repay)} brings the water back under the limit.`,
      clock: "Day 41",
      tokens: [...base, tok("d1", "usdc", usdcLabel(d1), "aBase", true), tok("d2", "usdc", usdcLabel(d2), "aBase", true), tok("r", "usdc", usdcLabel(repay), "bBase")],
      keys: [false, false, false],
      watch: "sees the repayment",
      basin: { label: "", gauge: g(d1 + d2 - repay, p2) },
      hold: 2600,
    },
  ];
}

// ---------------------------------------------------------------- OTC settlement (simulation)

function otcStart(usdcAmount: string): Frame[] {
  return [
    {
      caption: `Seller and buyer agree: 20 ZEC for ${usdcAmount}. They make an escrow with the policy signer.`,
      clock: "Hour 0",
      tokens: [tok("zec", "zec", "20 ZEC", "aZ"), tok("usdc", "usdc", usdcAmount, "bBase")],
      keys: [true, true, true],
      basin: { label: "" },
      hold: 2400,
    },
    {
      caption: "The seller moves the 20 ZEC into the escrow. The buyer sees it there with the viewing key.",
      clock: "Hour 1",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("usdc", "usdc", usdcAmount, "bBase")],
      keys: [true, false, false],
      basin: { label: "" },
      hold: 2200,
    },
  ];
}

function otcPaid(ctx: Ctx): Frame[] {
  const amount = usdcLabel(20 * ctx.price);
  return [
    ...otcStart(amount),
    {
      caption: `The buyer pays ${amount} to the seller's Base address.`,
      clock: "Hour 3",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("usdc", "usdc", amount, "aBase")],
      keys: [false, false, false],
      watch: "sees the payment",
      basin: { label: "" },
      hold: 2400,
    },
    {
      caption: "With the payment seen on Base, the buyer and the policy signer sign the release.",
      clock: "Hour 3",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("usdc", "usdc", amount, "aBase")],
      keys: [false, true, true],
      basin: { label: "" },
      hold: 2000,
    },
    {
      caption: "The ZEC moves to the buyer, shielded. Neither side had to trust the other, and nothing was bridged.",
      clock: "Hour 3",
      tokens: [tok("zec", "zec", "20 ZEC", "bZ"), tok("usdc", "usdc", amount, "aBase")],
      keys: [false, false, false],
      basin: { label: "" },
      hold: 2800,
    },
  ];
}

function otcExpired(ctx: Ctx): Frame[] {
  const amount = usdcLabel(20 * ctx.price);
  return [
    ...otcStart(amount),
    {
      caption: "Twenty-four hours pass and no payment arrives.",
      clock: "Hour 24, deadline",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("usdc", "usdc", amount, "bBase")],
      keys: [false, false, false],
      watch: "no payment",
      basin: { label: "" },
      hold: 2400,
    },
    {
      caption: "After the deadline the seller and the policy signer sign the refund.",
      clock: "Hour 24",
      tokens: [tok("zec", "zec", "20 ZEC", "escrow"), tok("usdc", "usdc", amount, "bBase")],
      keys: [true, false, true],
      basin: { label: "" },
      hold: 2000,
    },
    {
      caption: "The ZEC goes back to the seller, shielded. The buyer kept the USDC it never sent.",
      clock: "Hour 24",
      tokens: [tok("zec", "zec", "20 ZEC", "aZ"), tok("usdc", "usdc", amount, "bBase")],
      keys: [false, false, false],
      basin: { label: "" },
      hold: 2800,
    },
  ];
}

// ---------------------------------------------------------------- recovery (simulation)

function recoveryStart(): Frame[] {
  return [
    {
      caption: "The owner keeps 50 ZEC in a recovery escrow with an heir and the policy signer.",
      clock: "Day 0",
      tokens: [tok("zec", "zec", "50 ZEC", "aZ"), tok("ping", "msg", "check-in", "aBase", true)],
      keys: [true, true, true],
      basin: { label: "" },
      hold: 2200,
    },
    {
      caption: "The ZEC sits in the escrow, shielded. Day to day nothing changes for the owner.",
      clock: "Day 0",
      tokens: [tok("zec", "zec", "50 ZEC", "escrow"), tok("ping", "msg", "check-in", "aBase", true)],
      keys: [true, false, false],
      basin: { label: "" },
      hold: 2000,
    },
    {
      caption: "Each month the owner signs a check-in. The policy signer records it.",
      clock: "Day 30",
      tokens: [tok("zec", "zec", "50 ZEC", "escrow"), tok("ping", "msg", "check-in", "aBase")],
      keys: [false, false, false],
      watch: "records the check-in",
      basin: { label: "" },
      hold: 2000,
    },
  ];
}

function recoverySilent(): Frame[] {
  return [
    ...recoveryStart(),
    {
      caption: "Then the check-ins stop. After 180 days of silence the recovery opens.",
      clock: "Day 210",
      tokens: [tok("zec", "zec", "50 ZEC", "escrow"), tok("ping", "msg", "check-in", "aBase", true)],
      keys: [false, false, false],
      watch: "counts the silence",
      basin: { label: "" },
      hold: 2600,
    },
    {
      caption: "The heir and the policy signer sign the recovery.",
      clock: "Day 210",
      tokens: [tok("zec", "zec", "50 ZEC", "escrow"), tok("ping", "msg", "check-in", "aBase", true)],
      keys: [false, true, true],
      basin: { label: "" },
      hold: 2000,
    },
    {
      caption: "The 50 ZEC moves to the heir's shielded address. No seed phrase changed hands.",
      clock: "Day 210",
      tokens: [tok("zec", "zec", "50 ZEC", "bZ"), tok("ping", "msg", "check-in", "aBase", true)],
      keys: [false, false, false],
      basin: { label: "" },
      hold: 2800,
    },
  ];
}

function recoveryAlive(): Frame[] {
  return [
    ...recoveryStart(),
    {
      caption: "The owner checks in on day 170. The 180-day clock starts again.",
      clock: "Day 170",
      tokens: [tok("zec", "zec", "50 ZEC", "escrow"), tok("ping", "msg", "check-in", "aBase")],
      keys: [false, false, false],
      watch: "records the check-in",
      basin: { label: "" },
      hold: 2400,
    },
    {
      caption: "The heir and the policy signer can move nothing. The ZEC stays where the owner left it.",
      clock: "Day 210",
      tokens: [tok("zec", "zec", "50 ZEC", "escrow"), tok("ping", "msg", "check-in", "aBase", true)],
      keys: [false, false, false],
      basin: { label: "" },
      hold: 2800,
    },
  ];
}

export const POLICIES: Policy[] = [
  {
    id: "loan",
    name: "Loan",
    live: true,
    summary: "USDC from a lender's vault on Base against ZEC that stays in the escrow. Built and running.",
    parties: { a: "Borrower", b: "Lender" },
    watches: "ZEC/USD every round, and repayments on Base",
    rules: [
      { keys: [true, true, false], can: "Anything they agree on, without the policy signer" },
      { keys: [false, true, true], can: "Liquidate at flood stage, with the split the signer computes" },
      { keys: [true, false, true], can: "Return the ZEC once the repayment is seen on Base" },
    ],
    branches: [
      { id: "repaid", label: "Repaid", frames: loanRepaid },
      { id: "liquidated", label: "Price falls", frames: loanLiquidated },
    ],
  },
  {
    id: "credit",
    name: "Credit line",
    live: false,
    summary: "Draw and repay USDC as the price moves, against the same escrowed ZEC.",
    parties: { a: "Borrower", b: "Lender" },
    watches: "ZEC/USD every round; a draw needs room under the 50 % limit",
    rules: [
      { keys: [true, true, false], can: "Anything they agree on, without the policy signer" },
      { keys: [false, true, true], can: "Liquidate at flood stage, as in a loan" },
      { keys: [true, false, true], can: "Return the ZEC once the line is repaid and closed" },
    ],
    branches: [{ id: "line", label: "A month of draws", frames: creditLine }],
  },
  {
    id: "otc",
    name: "OTC settlement",
    live: false,
    summary: "Trade ZEC for USDC between two parties who do not trust each other, with no custodian and no bridge.",
    parties: { a: "Seller", b: "Buyer" },
    watches: "The USDC payment on Base, and the deadline",
    rules: [
      { keys: [true, true, false], can: "Anything they agree on, without the policy signer" },
      { keys: [false, true, true], can: "Release the ZEC to the buyer once the payment is seen" },
      { keys: [true, false, true], can: "Refund the seller after the deadline" },
    ],
    branches: [
      { id: "paid", label: "Buyer pays", frames: otcPaid },
      { id: "expired", label: "Deadline passes", frames: otcExpired },
    ],
  },
  {
    id: "recovery",
    name: "Recovery",
    live: false,
    summary: "An heir can recover the ZEC after a long silence, without ever holding a seed phrase.",
    parties: { a: "Owner", b: "Heir" },
    watches: "The owner's monthly check-ins, and 180 days of silence",
    rules: [
      { keys: [true, true, false], can: "Anything they agree on, without the policy signer" },
      { keys: [true, false, true], can: "Move the ZEC or name a new heir" },
      { keys: [false, true, true], can: "Recover the ZEC after 180 days without a check-in" },
    ],
    branches: [
      { id: "silent", label: "Owner goes silent", frames: () => recoverySilent() },
      { id: "alive", label: "Owner checks in", frames: () => recoveryAlive() },
    ],
  },
];
