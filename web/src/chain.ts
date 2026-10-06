import { createPublicClient, defineChain, http, type Address, type Hex } from "viem";
import { base, baseSepolia } from "viem/chains";

import { deployments, lenderVaultAbi, loanBookAbi } from "./generated/contracts";

export const CHAIN_ID = Number(import.meta.env.VITE_CHAIN_ID ?? 31337);
export const RPC_URL = import.meta.env.VITE_BASE_RPC_URL ?? "http://127.0.0.1:8546";
export const POLICY_URL = import.meta.env.VITE_POLICY_URL ?? "http://127.0.0.1:8787";

const local = defineChain({
  id: 31337,
  name: "Local (anvil)",
  nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
  rpcUrls: { default: { http: [RPC_URL] } },
});

const chain = CHAIN_ID === 8453 ? base : CHAIN_ID === 84532 ? baseSepolia : local;
export const client = createPublicClient({ chain, transport: http(RPC_URL) });

export type Deployment = {
  loanBook: Address;
  attestationVerifier: Address;
  lenderVault?: Address;
  startBlock: bigint;
};

export function deployment(): Deployment | null {
  const d = deployments[String(CHAIN_ID)];
  if (!d) return null;
  return {
    loanBook: d.loanBook as Address,
    attestationVerifier: d.attestationVerifier as Address,
    lenderVault: d.lenderVault as Address | undefined,
    startBlock: BigInt(d.startBlock ?? 0),
  };
}

/** Must match LoanBook.Health / lentic_core::Health. */
export const HEALTH = ["none", "active", "margin_call", "liquidating", "closed", "liquidated"] as const;
export type Health = (typeof HEALTH)[number];

export type Loan = {
  loanId: Hex;
  termsHash: Hex;
  vault: Address;
  health: Health;
  openedAt: number;
  updatedAt: number;
  oracleRound: bigint;
  attestationHash: Hex;
};

export async function readLoan(loanBook: Address, loanId: Hex): Promise<Loan | null> {
  const [termsHash, vault, health, openedAt, updatedAt, oracleRound, attestationHash] = await client.readContract({
    address: loanBook,
    abi: loanBookAbi,
    functionName: "loans",
    args: [loanId],
  });
  if (health === 0) return null;
  return {
    loanId,
    termsHash,
    vault,
    health: HEALTH[health] ?? "none",
    openedAt: Number(openedAt),
    updatedAt: Number(updatedAt),
    oracleRound,
    attestationHash,
  };
}

export async function listLoans(d: Deployment): Promise<Loan[]> {
  const opened = await client.getContractEvents({
    address: d.loanBook,
    abi: loanBookAbi,
    eventName: "LoanOpened",
    fromBlock: d.startBlock,
  });
  const loans = await Promise.all(opened.map((e) => readLoan(d.loanBook, e.args.loanId as Hex)));
  return loans.filter((l): l is Loan => l !== null).reverse();
}

export type HealthEvent = { health: Health; oracleRound: bigint; signer: Address; block: bigint; tx: Hex };

export async function healthHistory(d: Deployment, loanId: Hex): Promise<HealthEvent[]> {
  const logs = await client.getContractEvents({
    address: d.loanBook,
    abi: loanBookAbi,
    eventName: "HealthUpdated",
    args: { loanId },
    fromBlock: d.startBlock,
  });
  return logs.map((l) => ({
    health: HEALTH[Number(l.args.health)] ?? "none",
    oracleRound: l.args.oracleRound ?? 0n,
    signer: l.args.signer as Address,
    block: l.blockNumber,
    tx: l.transactionHash,
  }));
}

export type VaultSummary = {
  address: Address;
  owner: Address;
  totalAssets: bigint;
  idle: bigint;
  outstanding: bigint;
  feeBps: number;
  name: string;
};

export async function vaultSummary(vault: Address): Promise<VaultSummary> {
  const read = <F extends "owner" | "totalAssets" | "idle" | "totalOutstanding" | "feeBps" | "name">(functionName: F) =>
    client.readContract({ address: vault, abi: lenderVaultAbi, functionName });
  const [owner, totalAssets, idle, outstanding, feeBps, name] = await Promise.all([
    read("owner"),
    read("totalAssets"),
    read("idle"),
    read("totalOutstanding"),
    read("feeBps"),
    read("name"),
  ]);
  return {
    address: vault,
    owner: owner as Address,
    totalAssets: totalAssets as bigint,
    idle: idle as bigint,
    outstanding: outstanding as bigint,
    feeBps: Number(feeBps),
    name: name as string,
  };
}
