import { POLICY_URL } from "./chain";

export type PolicyInfo = {
  signerAddress: string;
  configHash: string;
  attestor: "dev" | "dstack";
  measurement: string;
  registeredOnBase: boolean;
  baseChainId: number;
  zcashNetwork: string;
  version: string;
};

export async function policyInfo(): Promise<PolicyInfo> {
  const res = await fetch(`${POLICY_URL}/v1/info`);
  if (!res.ok) throw new Error(`policy signer: HTTP ${res.status}`);
  return res.json();
}
