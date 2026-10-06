#!/usr/bin/env bash
# Deploys the local stack to anvil and opens one demo loan whose termsHash commits to real
# terms (dev/terms.local.json + salt), so /v1/assess reports matchesOnChain = true.
#   RPC (default http://127.0.0.1:8546) must be an anvil with the default mnemonic.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RPC="${RPC:-http://127.0.0.1:8546}"
LENDER_KEY=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
SALT=0x4444444444444444444444444444444444444444444444444444444444444444
SIGNER="$ROOT/target/debug/lentic-signer"

cd "$ROOT/contracts"
forge build >/dev/null
forge script script/DeployLocal.s.sol:DeployLocal --rpc-url "$RPC" --broadcast --slow >/dev/null
DEP="$ROOT/contracts/deployments/31337.json"
cd "$ROOT"
node scripts/export-abis.mjs

json() { python3 -c "import json,sys; print(json.load(open('$DEP'))['$1'])"; }
VAULT=$(json lenderVault)
BORROWER=$(json borrower)

mkdir -p "$ROOT/dev"
TERMS="$ROOT/dev/terms.local.json"
NOW=$(cast block latest --rpc-url "$RPC" --field timestamp)
cat >"$TERMS" <<EOF
{
  "version": 1,
  "chainId": 31337,
  "lenderVault": "$VAULT",
  "borrower": "$BORROWER",
  "principal": 10000000000,
  "aprBps": 1200,
  "originationFeeBps": 100,
  "collateralZat": 2000000000,
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
EOF

[ -x "$SIGNER" ] || cargo build -q -p lentic-signer
TERMS_HASH=$("$SIGNER" terms-hash --terms "$TERMS" --salt "$SALT")
LOAN_ID=$(cast keccak "lentic-demo-loan-$NOW")
cast send --rpc-url "$RPC" --private-key "$LENDER_KEY" "$VAULT" \
  "fund(bytes32,bytes32,address,uint128)" "$LOAN_ID" "$TERMS_HASH" "$BORROWER" 10000000000 >/dev/null

cat >"$ROOT/dev/loan.local.json" <<EOF
{ "loanId": "$LOAN_ID", "termsHash": "$TERMS_HASH", "salt": "$SALT", "terms": "dev/terms.local.json" }
EOF
echo "deployed to $RPC; demo loan $LOAN_ID (termsHash $TERMS_HASH)"
