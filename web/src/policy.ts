import type { Health } from "./chain";
import { POLICY_URL } from "./chain";

export type PolicyInfo = {
  signerAddress: string;
  attestor: "dev" | "dstack";
  measurement: string;
  registeredOnBase: boolean;
  zcashNetwork: string;
  devRoutes: boolean;
  loansHeld: number;
  roundSecs: number;
  reportsHealth: boolean;
};

export type PriceKind = "exchange_median" | "fixed" | "test_override";

export type PriceReading = {
  usd: number;
  kind: PriceKind;
  sources: { source: string; usd: number }[];
  at: number;
};

export type Reading = {
  round: number;
  priceUsd: number;
  priceKind: PriceKind;
  debt: number;
  collateralZat: number;
  ltvBps: number | null;
  health: Health;
};

export type Gauge = {
  loanId: string;
  actionStageBps: number;
  floodStageBps: number;
  haircutBps: number;
  penaltyBps: number;
  collateralSource: "terms" | "escrow";
  roundSecs: number;
  readings: Reading[];
  lastError: string | null;
};

export class PolicyError extends Error {}

async function call<T>(path: string, init?: RequestInit): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${POLICY_URL}${path}`, init);
  } catch {
    throw new PolicyError(`The policy signer is not answering at ${POLICY_URL}.`);
  }
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new PolicyError(body.error ?? `The policy signer answered ${res.status}.`);
  return body as T;
}

export const policyInfo = () => call<PolicyInfo>("/v1/info");
export const policyPrice = () => call<PriceReading>("/v1/price");
export const policyGauge = (loanId: string) => call<Gauge>(`/v1/loans/${loanId}/gauge`);

export const setTestPrice = (usd: number | null) =>
  call<PriceReading>("/v1/dev/price", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ usd }),
  });

export const PRICE_SOURCE: Record<PriceKind, string> = {
  exchange_median: "median of five exchanges",
  fixed: "fixed by configuration",
  test_override: "test price",
};
