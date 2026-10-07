#!/usr/bin/env bash
# Deploys Lentic to Base Sepolia with the development attestor (contracts/script/DeployTestnet.s.sol)
# and gives the lender, the borrower and the policy signer a little ETH for gas.
#
#   SEPOLIA_KEYS (default dev/sepolia/keys) holds deployer.key, lender.key, borrower.key and
#   policy.key (hex, test keys only); only the deployer needs funding beforehand.
#   Writes contracts/deployments/84532.json and refreshes the ABIs the services and web read.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RPC="${BASE_RPC:-https://sepolia.base.org}"
KEYS="${SEPOLIA_KEYS:-$ROOT/dev/sepolia/keys}"
GAS_ETH="${GAS_ETH:-0.002}"

key() { cat "$KEYS/$1.key"; }
addr() { cast wallet address "$(key "$1")"; }

[ "$(cast chain-id --rpc-url "$RPC")" = 84532 ] || { echo "$RPC is not Base Sepolia" >&2; exit 1; }
DEPLOYER=$(addr deployer)
echo "deployer $DEPLOYER: $(cast balance --ether "$DEPLOYER" --rpc-url "$RPC") ETH"

for role in lender borrower policy; do
  a=$(addr "$role")
  if [ "$(cast balance "$a" --rpc-url "$RPC")" -lt 500000000000000 ]; then
    cast send --rpc-url "$RPC" --private-key "$(key deployer)" "$a" --value "${GAS_ETH}ether" >/dev/null
  fi
  echo "$role $a: $(cast balance --ether "$a" --rpc-url "$RPC") ETH"
done

(cd "$ROOT/contracts" && DEPLOYER_KEY="$(key deployer)" LENDER_KEY="$(key lender)" \
  POLICY_SIGNER="$(addr policy)" BORROWER="$(addr borrower)" \
  forge script script/DeployTestnet.s.sol:DeployTestnet --rpc-url "$RPC" --broadcast --slow)
(cd "$ROOT" && node scripts/export-abis.mjs)
cat "$ROOT/contracts/deployments/84532.json"
