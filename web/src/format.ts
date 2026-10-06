import type { Health } from "./chain";

export const short = (hex: string, n = 6) => (hex.length > 2 * n + 2 ? `${hex.slice(0, n + 2)}…${hex.slice(-n)}` : hex);

export const usdc = (v: bigint) =>
  (Number(v) / 1e6).toLocaleString("en-US", { style: "currency", currency: "USD", maximumFractionDigits: 0 });

export const time = (unix: number) =>
  unix ? new Date(unix * 1000).toLocaleString("en-GB", { dateStyle: "medium", timeStyle: "short" }) : "—";

export const HEALTH_LABEL: Record<Health, string> = {
  none: "Unknown",
  active: "Active",
  margin_call: "Margin call",
  liquidating: "Liquidating",
  closed: "Closed",
  liquidated: "Liquidated",
};
