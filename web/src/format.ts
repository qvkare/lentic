import type { Health } from "./chain";

const NBSP = " ";

export const short = (hex: string, n = 4) => (hex.length > 2 * n + 2 ? `${hex.slice(0, n + 2)}…${hex.slice(-n)}` : hex);

const fixed = (v: number, digits: number) =>
  v.toLocaleString("en-US", { minimumFractionDigits: digits, maximumFractionDigits: digits });

export const pct = (bps: number) => `${fixed(bps / 100, 1)}${NBSP}%`;
/** Stage thresholds: whole percentages without a decimal. */
export const pctStage = (bps: number) => `${bps % 100 === 0 ? bps / 100 : fixed(bps / 100, 1)}${NBSP}%`;
export const usdc = (micro: number | bigint) => `${fixed(Number(micro) / 1e6, 2)}${NBSP}USDC`;
export const usdcWhole = (micro: number | bigint) => `${fixed(Number(micro) / 1e6, 0)}${NBSP}USDC`;
export const zec = (zat: number) => `${fixed(zat / 1e8, 4)}${NBSP}ZEC`;
export const usd = (v: number) => `${fixed(v, 2)}${NBSP}USD`;

export const clock = (unix: number) =>
  unix ? new Date(unix * 1000).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit", second: "2-digit" }) : "";

export const when = (unix: number) =>
  unix ? new Date(unix * 1000).toLocaleString("en-GB", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }) : "";

export type Stage = "below" | "action" | "flood";

export function stageOf(ltvBps: number, actionBps: number, floodBps: number): Stage {
  if (ltvBps >= floodBps) return "flood";
  if (ltvBps >= actionBps) return "action";
  return "below";
}

export const STAGE_SENTENCE: Record<Stage, string> = {
  below: "Below action stage.",
  action: "Action stage: margin call. Nothing moves; the borrower can add ZEC or repay.",
  flood: "Flood stage: liquidation. The lender and the policy signer may sign the escrow spend.",
};

/** How the policy signer's last report on Base reads. */
export const HEALTH_SENTENCE: Record<Health, string> = {
  none: "No report yet",
  active: "Below action stage",
  margin_call: "Action stage: margin call",
  liquidating: "Flood stage: liquidation",
  closed: "Closed",
  liquidated: "Liquidated",
};

export const isFlood = (h: Health) => h === "liquidating" || h === "liquidated";

/** USD per ZEC at which the loan reaches `stageBps`. */
export function stagePrice(debtMicro: number, collateralZat: number, stageBps: number): number | null {
  if (collateralZat <= 0) return null;
  return debtMicro / 1e6 / ((collateralZat / 1e8) * (stageBps / 10_000));
}

/** ZIP 317 fee of a liquidation (one escrow spend, two payments); mirrors lentic_zcash::fee. */
export const LIQUIDATION_FEE_ZAT = 10_000;

/**
 * Loan-to-value in basis points at `priceUsd`, rounded to 0.01 bps so that the exact stage price
 * lands on the stage (7499.9999… would otherwise miss flood stage at the flood price).
 */
export function ltvAt(debtMicro: number, collateralZat: number, priceUsd: number): number {
  const value = (collateralZat / 1e8) * priceUsd;
  if (value <= 0) return Number.POSITIVE_INFINITY;
  return Math.round((debtMicro / 1e6 / value) * 10_000 * 100) / 100;
}

/** The split the policy signer enforces (lentic_core::math::liquidation_split), for display. */
export function liquidationSplit(
  collateralZat: number,
  debtMicro: number,
  priceUsd: number,
  haircutBps: number,
  penaltyBps: number,
): { lenderZat: number; borrowerZat: number; feeZat: number } {
  const available = collateralZat - LIQUIDATION_FEE_ZAT;
  const owed = (debtMicro / 1e6) * (1 + penaltyBps / 10_000);
  const needed = Math.ceil((owed / (priceUsd * (1 - haircutBps / 10_000))) * 1e8);
  const lenderZat = Math.min(needed, available);
  return { lenderZat, borrowerZat: available - lenderZat, feeZat: LIQUIDATION_FEE_ZAT };
}
