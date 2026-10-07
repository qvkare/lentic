#!/usr/bin/env bash
# Deploys the local stack to anvil and opens two demo loans whose termsHash commits to real
# terms. The policy signer's copy of each loan (loanId, salt, terms) goes to dev/loans/, which
# `lentic-policy serve` reads by default.
#   RPC (default http://127.0.0.1:8546) must be an anvil with the default mnemonic.
#   DEV_DIR (default dev/) receives the terms, the loans and loan.local.json.
#   DEPLOYMENT_FILE (default deployments/31337.json, under contracts/) receives the addresses and
#   POLICY_MEASUREMENT pins another policy-signer measurement (scripts/dstack-sim.sh).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RPC="${RPC:-http://127.0.0.1:8546}"
LENDER_KEY=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
SALT=0x4444444444444444444444444444444444444444444444444444444444444444
SIGNER="$ROOT/target/debug/lentic-signer"
DEV="${DEV_DIR:-$ROOT/dev}"

cd "$ROOT/contracts"
forge build >/dev/null
forge script script/DeployLocal.s.sol:DeployLocal --rpc-url "$RPC" --broadcast --slow >/dev/null
DEP="$ROOT/contracts/${DEPLOYMENT_FILE:-deployments/31337.json}"
cd "$ROOT"
# Only the shared local deployment goes into the web app's generated addresses.
if [ -z "${DEPLOYMENT_FILE:-}" ]; then node scripts/export-abis.mjs; fi

json() { python3 -c "import json,sys; print(json.load(open('$DEP'))['$1'])"; }
VAULT=$(json lenderVault)
BORROWER=$(json borrower)

mkdir -p "$DEV/loans"
rm -f "$DEV"/loans/*.json
NOW=$(cast block latest --rpc-url "$RPC" --field timestamp)
[ -x "$SIGNER" ] || cargo build -q -p lentic-signer

# open_loan <tag> <principal, µUSDC> <collateral, zat>: writes the terms, commits their hash in
# LoanBook through the vault's payout, and stores the signer's copy in dev/loans/<loanId>.json.
open_loan() {
  local tag=$1 principal=$2 collateral=$3
  local terms_file="$DEV/terms.$tag.json"
  cat >"$terms_file" <<TERMS
{
  "version": 1,
  "chainId": 31337,
  "lenderVault": "$VAULT",
  "borrower": "$BORROWER",
  "principal": $principal,
  "aprBps": 1200,
  "originationFeeBps": 100,
  "collateralZat": $collateral,
  "marginLtvBps": 6000,
  "liquidationLtvBps": 7500,
  "haircutBps": 500,
  "penaltyBps": 300,
  "fundingDeadline": $((NOW + 86400)),
  "maturity": $((NOW + 365 * 86400)),
  "escrowFvkHash": "0x3333333333333333333333333333333333333333333333333333333333333333",
  "borrowerReturnAddress": "u1borrower-return-placeholder",
  "lenderZcashAddress": "u1lender-payout-placeholder"
}
TERMS
  local terms_hash loan_id
  terms_hash=$("$SIGNER" terms-hash --terms "$terms_file" --salt "$SALT")
  # LoanBook binds the id to the vault: keccak256(abi.encode(vault, termsHash)).
  loan_id=$("$SIGNER" terms-hash --terms "$terms_file" --salt "$SALT" --loan-id)
  cast send --rpc-url "$RPC" --private-key "$LENDER_KEY" "$VAULT" \
    "fund(bytes32,bytes32,address,uint128)" "$loan_id" "$terms_hash" "$BORROWER" "$principal" >/dev/null
  printf '{ "loanId": "%s", "salt": "%s", "terms": %s }\n' "$loan_id" "$SALT" "$(cat "$terms_file")" \
    >"$DEV/loans/$loan_id.json"
  echo "$loan_id"
}

# 10,000 USDC against 20 ZEC, and 4,500 USDC against 5 ZEC (near the action stage at ~1,370 USD/ZEC).
LOAN_ID=$(open_loan steady 10000000000 2000000000)
open_loan tight 4500000000 500000000 >/dev/null
cp "$DEV/terms.steady.json" "$DEV/terms.local.json"
cat >"$DEV/loan.local.json" <<LOAN
{ "loanId": "$LOAN_ID", "salt": "$SALT", "terms": "dev/terms.local.json" }
LOAN
echo "deployed to $RPC; the policy signer holds $(find "$DEV/loans" -name '*.json' | wc -l | tr -d ' ') loans ($DEV/loans)"
