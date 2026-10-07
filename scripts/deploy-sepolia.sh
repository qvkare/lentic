#!/usr/bin/env bash
# Deploys Lentic to Base Sepolia with the development attestor (contracts/script/DeployTestnet.s.sol)
# and gives the lender, the borrower and the policy signer a little ETH for gas.
#
#   SEPOLIA_KEYS (default dev/sepolia/keys) holds deployer.key, lender.key, borrower.key and
#   policy.key (hex, test keys only); only the deployer needs funding beforehand.
#   USDC: Circle's Base Sepolia USDC when the deployer holds some (it hands VAULT_DEPOSIT to the
#   lender and BORROWER_USDC to the borrower), otherwise a mock token the script mints.
#   Writes contracts/deployments/84532.json and refreshes the ABIs the services and web read.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RPC="${BASE_RPC:-https://sepolia.base.org}"
KEYS="${SEPOLIA_KEYS:-$ROOT/dev/sepolia/keys}"
GAS_ETH="${GAS_ETH:-0.0002}"            # ~30M gas each at Base Sepolia's 0.006 gwei
CIRCLE_USDC=0x036CbD53842c5426634e7929541eC2318f3dCF7e
VAULT_DEPOSIT="${VAULT_DEPOSIT:-15000000}" # 15 USDC
BORROWER_USDC="${BORROWER_USDC:-1000000}"  # 1 USDC, to cover interest on repayment

key() { cat "$KEYS/$1.key"; }
addr() { cast wallet address "$(key "$1")"; }
usdc_of() { cast call "$CIRCLE_USDC" "balanceOf(address)(uint256)" "$1" --rpc-url "$RPC" | awk '{print $1}'; }
# The public RPC is load-balanced: a send can read a stale nonce, so retry a few times.
send() {
  for attempt in 1 2 3 4; do
    cast send --rpc-url "$RPC" --private-key "$(key deployer)" "$@" >/dev/null 2>&1 && return 0
    sleep $((attempt * 3))
  done
  cast send --rpc-url "$RPC" --private-key "$(key deployer)" "$@" >/dev/null
}

[ "$(cast chain-id --rpc-url "$RPC")" = 84532 ] || { echo "$RPC is not Base Sepolia" >&2; exit 1; }
DEPLOYER=$(addr deployer)
echo "deployer $DEPLOYER: $(cast balance --ether "$DEPLOYER" --rpc-url "$RPC") ETH"

for role in lender borrower policy; do
  a=$(addr "$role")
  if [ "$(cast balance "$a" --rpc-url "$RPC")" -lt 100000000000000 ]; then
    send "$a" --value "${GAS_ETH}ether"
  fi
  echo "$role $a: $(cast balance --ether "$a" --rpc-url "$RPC") ETH"
done

USDC_ENV=()
missing() { local have; have=$(usdc_of "$(addr "$1")"); [ "$have" -lt "$2" ] && echo $(($2 - have)) || echo 0; }
need_lender=$(missing lender "$VAULT_DEPOSIT")
need_borrower=$(missing borrower "$BORROWER_USDC")
held=$(usdc_of "$DEPLOYER")
if [ "$held" -ge $((need_lender + need_borrower)) ]; then
  echo "using Circle's USDC (deployer holds $((held / 1000000)) USDC)"
  [ "$need_lender" -gt 0 ] && send "$CIRCLE_USDC" "transfer(address,uint256)" "$(addr lender)" "$need_lender"
  [ "$need_borrower" -gt 0 ] && send "$CIRCLE_USDC" "transfer(address,uint256)" "$(addr borrower)" "$need_borrower"
  echo "lender $(($(usdc_of "$(addr lender)") / 1000000)) USDC, borrower $(($(usdc_of "$(addr borrower)") / 1000000)) USDC"
  USDC_ENV=(USDC="$CIRCLE_USDC")
fi

(cd "$ROOT/contracts" && env ${USDC_ENV[@]+"${USDC_ENV[@]}"} DEPLOYER_KEY="$(key deployer)" LENDER_KEY="$(key lender)" \
  POLICY_SIGNER="$(addr policy)" BORROWER="$(addr borrower)" VAULT_DEPOSIT="$VAULT_DEPOSIT" \
  forge script script/DeployTestnet.s.sol:DeployTestnet --rpc-url "$RPC" --broadcast --slow)
(cd "$ROOT" && node scripts/export-abis.mjs)
cat "$ROOT/contracts/deployments/84532.json"
