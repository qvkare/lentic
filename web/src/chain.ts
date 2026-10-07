import { createPublicClient, defineChain, http, type Address, type Hex } from "viem";
import { base, baseSepolia } from "viem/chains";

import { deployments, lenderVaultAbi, loanBookAbi } from "./generated/contracts";

// `pnpm web` reads the local stack by default. A production build reads only what its env names, so a site
// published without a deployment never calls a visitor's localhost: it shows the simulations alone.
const env = import.meta.env;
export const CHAIN_ID = Number(env.VITE_CHAIN_ID || (env.DEV ? 31337 : 84532));
export const RPC_URL: string | null = env.VITE_BASE_RPC_URL || (env.DEV ? "http://127.0.0.1:8546" : null);
export const POLICY_URL: string | null = env.VITE_POLICY_URL || (env.DEV ? "http://127.0.0.1:8787" : null);

const local = defineChain({
  id: 31337,
  name: "Local (anvil)",
  nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
  rpcUrls: { default: { http: [RPC_URL ?? "http://127.0.0.1:8546"] } },
});

const chain = CHAIN_ID === 8453 ? base : CHAIN_ID === 84532 ? baseSepolia : local;
export const client = createPublicClient({ chain, transport: http(RPC_URL ?? undefined) });

export type Deployment = {
  loanBook: Address;
  attestationVerifier: Address;
  lenderVault?: Address;
  startBlock: bigint;
};

export function deployment(): Deployment | null {
  const d = deployments[String(CHAIN_ID)];
  if (!d || !RPC_URL) return null;
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

/** Loans opened by the configured lender's vault (every loan, when no vault is configured). */
export async function listLoans(d: Deployment): Promise<Loan[]> {
  const opened = await client.getContractEvents({
    address: d.loanBook,
    abi: loanBookAbi,
    eventName: "LoanOpened",
    args: d.lenderVault ? { vault: d.lenderVault } : undefined,
    fromBlock: d.startBlock,
  });
  const loans = await Promise.all(opened.map((e) => readLoan(d.loanBook, e.args.loanId as Hex)));
  return loans.filter((l): l is Loan => l !== null).reverse();
}

export type VaultSummary = {
  address: Address;
  totalAssets: bigint;
  idle: bigint;
  outstanding: bigint;
  feeBps: number;
  name: string;
};

export async function vaultSummary(vault: Address): Promise<VaultSummary> {
  const read = <F extends "totalAssets" | "idle" | "totalOutstanding" | "feeBps" | "name">(functionName: F) =>
    client.readContract({ address: vault, abi: lenderVaultAbi, functionName });
  const [totalAssets, idle, outstanding, feeBps, name] = await Promise.all([
    read("totalAssets"),
    read("idle"),
    read("totalOutstanding"),
    read("feeBps"),
    read("name"),
  ]);
  return {
    address: vault,
    totalAssets: totalAssets as bigint,
    idle: idle as bigint,
    outstanding: outstanding as bigint,
    feeBps: Number(feeBps),
    name: name as string,
  };
}

export type Party = "borrower" | "lender" | "policy";

/** One row of the key-turn ledger: what happened and which keys acted. */
export type KeyTurn = {
  key: string;
  event: string;
  detail: string;
  acted: Party[];
  health?: Health;
  block: bigint;
  time: number;
};

const blockTimes = new Map<bigint, number>();
async function blockTime(block: bigint): Promise<number> {
  const hit = blockTimes.get(block);
  if (hit !== undefined) return hit;
  const b = await client.getBlock({ blockNumber: block });
  blockTimes.set(block, Number(b.timestamp));
  return Number(b.timestamp);
}

const STAGE_REPORT: Record<Health, string> = {
  none: "Reported",
  active: "Reported below action stage",
  margin_call: "Reported action stage",
  liquidating: "Reported flood stage",
  closed: "Reported closed",
  liquidated: "Liquidation co-signed",
};

/** Base events for a loan, oldest first, mapped to the keys that acted. */
export async function keyTurns(d: Deployment, loan: Loan): Promise<KeyTurn[]> {
  const from = d.startBlock;
  const [opened, reports, funded, repaid, writtenOff] = await Promise.all([
    client.getContractEvents({ address: d.loanBook, abi: loanBookAbi, eventName: "LoanOpened", args: { loanId: loan.loanId }, fromBlock: from }),
    client.getContractEvents({ address: d.loanBook, abi: loanBookAbi, eventName: "HealthUpdated", args: { loanId: loan.loanId }, fromBlock: from }),
    client.getContractEvents({ address: loan.vault, abi: lenderVaultAbi, eventName: "LoanFunded", args: { loanId: loan.loanId }, fromBlock: from }),
    client.getContractEvents({ address: loan.vault, abi: lenderVaultAbi, eventName: "LoanRepaid", args: { loanId: loan.loanId }, fromBlock: from }),
    client.getContractEvents({ address: loan.vault, abi: lenderVaultAbi, eventName: "LoanWrittenOff", args: { loanId: loan.loanId }, fromBlock: from }),
  ]);

  const rows: Omit<KeyTurn, "time">[] = [];
  const principal = funded[0]?.args.principal ?? 0n;
  for (const e of opened) {
    rows.push({
      key: `${e.transactionHash}-${e.logIndex}`,
      event: "Opened and paid out",
      detail: principal ? `${formatUsdc(principal)} from the lender's vault` : "Lender's vault",
      acted: ["lender"],
      block: e.blockNumber,
    });
  }
  for (const e of reports) {
    const health = HEALTH[Number(e.args.health)] ?? "none";
    const byVault = (e.args.signer as string).toLowerCase() === loan.vault.toLowerCase();
    rows.push({
      key: `${e.transactionHash}-${e.logIndex}`,
      event: byVault ? "Closed by borrower and lender" : STAGE_REPORT[health],
      detail: byVault ? "Recorded by the lender's vault" : `Price reading of ${readingTime(e.args.oracleRound)}`,
      acted: byVault ? ["borrower", "lender"] : health === "liquidated" ? ["lender", "policy"] : ["policy"],
      health,
      block: e.blockNumber,
    });
  }
  for (const e of repaid) {
    const amount = (e.args.principalPart ?? 0n) + (e.args.interestPart ?? 0n);
    rows.push({
      key: `${e.transactionHash}-${e.logIndex}`,
      event: "Repayment",
      detail: `${formatUsdc(amount)} into the vault`,
      acted: ["borrower"],
      block: e.blockNumber,
    });
  }
  for (const e of writtenOff) {
    rows.push({
      key: `${e.transactionHash}-${e.logIndex}`,
      event: "Written off",
      detail: `${formatUsdc(e.args.principalLost ?? 0n)} left the vault's books`,
      acted: ["lender"],
      block: e.blockNumber,
    });
  }
  rows.sort((a, b) => (a.block < b.block ? -1 : a.block > b.block ? 1 : 0));
  return Promise.all(rows.map(async (r) => ({ ...r, time: await blockTime(r.block) })));
}

/** Oracle rounds are the price reading's unix time. */
function readingTime(round: bigint | undefined): string {
  if (!round) return "";
  return new Date(Number(round) * 1000).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function formatUsdc(micro: bigint): string {
  return `${(Number(micro) / 1e6).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })} USDC`;
}
