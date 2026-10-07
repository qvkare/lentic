#!/usr/bin/env bash
# Two loans end to end, with real Zcash testnet transactions.
#
# For each loan the escrow key is made through the policy signer's relay (borrower and lender
# run as separate processes), both parties approve the terms and register them with the
# signer, the borrower deposits real testnet ZEC and the lender's vault funds the loan on Base.
#   loan A: the price drops past flood stage; the lender and the signer co-sign the liquidation.
#   loan B: the borrower repays on Base; the borrower and the signer co-sign the close.
# Every transaction lands in $OUT/record.jsonl.
#
#   Base:   BASE=local (default): a fresh anvil with the local deployment.
#           BASE=sepolia: Base Sepolia with contracts/deployments/84532.json and the role keys in
#           SEPOLIA_KEYS (scripts/deploy-sepolia.sh makes both).
#   Zcash:  testnet through lightwalletd; FUNDER is a `lentic-watch wallet` holding TAZ,
#           LENDER_PAYOUT a wallet that receives the lender's share.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${OUT:-$ROOT/dev/e2e/$(date +%Y%m%d-%H%M%S)}"
CHAIN_PORT="${CHAIN_PORT:-8556}"
API_PORT="${API_PORT:-8796}"
LWD="${LWD:-https://testnet.zec.rocks:443}"
FUNDER="${FUNDER:-$ROOT/dev/testnet/funder.wallet.json}"
LENDER_PAYOUT="${LENDER_PAYOUT:-$ROOT/dev/testnet/lender-payout.wallet.json}"
COLLATERAL_ZAT="${COLLATERAL_ZAT:-1000000}" # 0.01 TAZ per loan
PRINCIPAL="${PRINCIPAL:-6000000}"           # 6 USDC
FLOOD_PRICE="${FLOOD_PRICE:-750}"           # USD/ZEC for loan A: LTV 80 %, above the 75 % flood stage
SIGNER="$ROOT/target/debug/lentic-signer"
WATCH="$ROOT/target/debug/lentic-watch"
POLICY="$ROOT/target/debug/lentic-policy"
API="http://127.0.0.1:$API_PORT"
BASE="${BASE:-local}"
case "$BASE" in
  local)
    RPC="http://127.0.0.1:$CHAIN_PORT"
    # anvil's default accounts: #1 lender (vault owner), #2 borrower, #3 policy signer.
    LENDER_KEY=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
    BORROWER_KEY=0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a
    POLICY_KEY=0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6
    BASE_TX=""
    ;;
  sepolia)
    RPC="${BASE_RPC:-https://sepolia.base.org}"
    KEYS="${SEPOLIA_KEYS:-$ROOT/dev/sepolia/keys}"
    LENDER_KEY=$(cat "$KEYS/lender.key")
    BORROWER_KEY=$(cat "$KEYS/borrower.key")
    POLICY_KEY=$(cat "$KEYS/policy.key")
    BASE_TX="https://sepolia.basescan.org/tx/"
    ;;
  *) echo "BASE is local or sepolia" >&2; exit 1 ;;
esac

mkdir -p "$OUT/keys"
PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }
field() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
record() { echo "$1" >>"$OUT/record.jsonl"; echo "  $1"; }
until_ok() { # until_ok <seconds> <description> <command...>
  local limit=$1 what=$2; shift 2
  for _ in $(seq 1 "$limit"); do "$@" >/dev/null 2>&1 && return 0; sleep 1; done
  echo "timed out: $what" >&2; return 1
}

say "building"
(cd "$ROOT" && cargo build -q -p lentic-signer -p lentic-watch -p lentic-policy)

if [ "$BASE" = local ]; then
  say "Base: anvil on :$CHAIN_PORT, contracts"
  anvil --port "$CHAIN_PORT" --chain-id 31337 --block-time 1 --silent &
  PIDS+=($!)
  until_ok 30 "anvil" cast chain-id --rpc-url "$RPC"
  (cd "$ROOT/contracts" && forge script script/DeployLocal.s.sol:DeployLocal --rpc-url "$RPC" --broadcast --slow >/dev/null)
fi
CHAIN_ID=$(cast chain-id --rpc-url "$RPC")
DEP="$ROOT/contracts/deployments/$CHAIN_ID.json"
[ -f "$DEP" ] || { echo "no deployment for chain $CHAIN_ID ($DEP)"; exit 1; }
dep() { python3 -c "import json; print(json.load(open('$DEP'))['$1'])"; }
base_tx() { [ -n "$BASE_TX" ] && echo ",\"url\":\"$BASE_TX$1\"" || true; }
zec_tx() { echo ",\"url\":\"https://testnet.zecblock.com/tx/$1\""; }
VAULT=$(dep lenderVault)
USDC=$(dep usdc)
BOOK=$(dep loanBook)
echo "$LENDER_KEY" >"$OUT/keys/lender.key"
echo "$BORROWER_KEY" >"$OUT/keys/borrower.key"
LENDER_ADDR=$(cast wallet address "$LENDER_KEY")
BORROWER_ADDR=$(cast wallet address "$BORROWER_KEY")

say "policy signer on :$API_PORT (Base chain $CHAIN_ID, Zcash testnet, health reports on)"
POLICY_ETH_KEY="$POLICY_KEY" "$POLICY" serve --base-rpc "$RPC" --listen "127.0.0.1:$API_PORT" --dev-routes --report-health \
  --deployment "$DEP" --zcash-network testnet --lightwalletd "$LWD" \
  --loans-dir "$OUT/policy/loans" --state-dir "$OUT/policy/state" --round-secs 10 >"$OUT/policy.log" 2>&1 &
PIDS+=($!)
until_ok 90 "policy signer" curl -sf "$API/v1/price"

BORROWER_UA=$("$WATCH" wallet show --wallet "$FUNDER" | field "d['address']")
LENDER_UA=$("$WATCH" wallet show --wallet "$LENDER_PAYOUT" | field "d['address']")

# open_loan <tag>: DKG through the signer, terms approved and registered, a real deposit, and
# the loan funded on Base. Sets LOAN_ID and leaves the files in $OUT/<tag>.
open_loan() {
  local tag=$1 dir="$OUT/$1"
  mkdir -p "$dir"
  say "loan $tag: escrow key through the signer's relay (borrower and lender as separate processes)"
  local session
  session=$("$SIGNER" dkg create --policy "$API" --borrower "$BORROWER_ADDR" --lender "$LENDER_ADDR" | field "d['session']")
  "$SIGNER" dkg join --policy "$API" --session "$session" --role borrower --base-key "$OUT/keys/borrower.key" \
    --out "$dir/borrower.share.json" >"$dir/borrower.dkg.json" 2>"$dir/borrower.dkg.log" &
  local b=$!
  "$SIGNER" dkg join --policy "$API" --session "$session" --role lender --base-key "$OUT/keys/lender.key" \
    --out "$dir/lender.share.json" >"$dir/lender.dkg.json" 2>"$dir/lender.dkg.log" &
  local l=$!
  wait "$b" "$l"
  local escrow fvk_hash
  escrow=$(field "d['escrowAddress']" <"$dir/lender.dkg.json")
  fvk_hash=$(field "d['escrowFvkHash']" <"$dir/lender.dkg.json")
  [ "$fvk_hash" = "$(field "d['escrowFvkHash']" <"$dir/borrower.dkg.json")" ] || { echo "parties disagree on the escrow key"; exit 1; }
  record "{\"loan\":\"$tag\",\"step\":\"dkg\",\"session\":\"$session\",\"escrowFvkHash\":\"$fvk_hash\",\"escrow\":\"$escrow\"}"

  say "loan $tag: terms approved by both, registered with the signer"
  local now salt hash bsig lsig
  now=$(cast block latest --rpc-url "$RPC" --field timestamp)
  salt=0x$(openssl rand -hex 32)
  cat >"$dir/terms.json" <<TERMS
{
  "version": 1, "chainId": $CHAIN_ID, "lenderVault": "$VAULT", "borrower": "$BORROWER_ADDR",
  "principal": $PRINCIPAL, "aprBps": 1200, "originationFeeBps": 100, "collateralZat": $COLLATERAL_ZAT,
  "marginLtvBps": 6000, "liquidationLtvBps": 7500, "haircutBps": 500, "penaltyBps": 300,
  "fundingDeadline": $((now + 3600)), "maturity": $((now + 30 * 86400)),
  "escrowFvkHash": "$fvk_hash",
  "borrowerReturnAddress": "$BORROWER_UA", "lenderZcashAddress": "$LENDER_UA"
}
TERMS
  # LoanBook binds the loan id to the vault: keccak256(abi.encode(vault, termsHash)).
  LOAN_ID=$("$SIGNER" terms-hash --terms "$dir/terms.json" --salt "$salt" --loan-id)
  bsig=$("$SIGNER" terms approve --terms "$dir/terms.json" --salt "$salt" --base-key "$OUT/keys/borrower.key" | field "d['signature']")
  lsig=$("$SIGNER" terms approve --terms "$dir/terms.json" --salt "$salt" --base-key "$OUT/keys/lender.key" | field "d['signature']")
  "$SIGNER" terms register --policy "$API" --terms "$dir/terms.json" --salt "$salt" \
    --session "$session" --borrower-signature "$bsig" --lender-signature "$lsig" >"$dir/registered.json"
  hash=$(field "d['termsHash']" <"$dir/registered.json")

  say "loan $tag: the borrower deposits $COLLATERAL_ZAT zat into the escrow (Zcash testnet)"
  local dep
  until dep=$("$WATCH" wallet send --wallet "$FUNDER" --to "$escrow" --zat "$COLLATERAL_ZAT" --server "$LWD" 2>"$dir/deposit.err"); do
    grep -q "confirmation" "$dir/deposit.err" || { cat "$dir/deposit.err"; exit 1; }
    sleep 10 # the wallet's change from the last deposit is not mined yet
  done
  local dep_tx
  dep_tx=$(echo "$dep" | field "d['txid']")
  record "{\"loan\":\"$tag\",\"step\":\"deposit\",\"chain\":\"zcash-testnet\",\"txid\":\"$dep_tx\",\"zat\":$COLLATERAL_ZAT$(zec_tx "$dep_tx")}"
  local seen=0
  for _ in $(seq 1 60); do
    seen=$(curl -sf "$API/v1/loans/$LOAN_ID/escrow" | field "d['scan']['unspentZat']" || echo 0)
    [ "$seen" -gt 0 ] 2>/dev/null && break
    sleep 5
  done
  [ "$seen" -gt 0 ] || { echo "the signer never saw the deposit"; exit 1; }
  echo "  the signer sees $seen zat in the escrow"

  say "loan $tag: the lender's vault funds it on Base"
  local fund
  fund=$(bsend "$LENDER_KEY" "$VAULT" "fund(bytes32,bytes32,address,uint128)" "$LOAN_ID" "$hash" "$BORROWER_ADDR" \
    "$PRINCIPAL" | field "d['transactionHash']")
  record "{\"loan\":\"$tag\",\"step\":\"fund\",\"chain\":\"base-$BASE\",\"tx\":\"$fund\",\"loanId\":\"$LOAN_ID\",\"termsHash\":\"$hash\"$(base_tx "$fund")}"
}

# cast send with retries: a load-balanced public RPC can hand out a stale nonce.
bsend() { # bsend <private key> <cast send args...>; prints the receipt JSON
  local key=$1 out attempt
  shift
  for attempt in 1 2 3 4; do
    out=$(cast send --json --rpc-url "$RPC" --private-key "$key" "$@" 2>/dev/null) && { echo "$out"; return 0; }
    sleep $((attempt * 3))
  done
  cast send --json --rpc-url "$RPC" --private-key "$key" "$@"
}

health() { curl -sf "$API/v1/loans/$1" | field "d['health']"; }
wait_health() { # wait_health <loan> <health>
  for _ in $(seq 1 120); do [ "$(health "$1" 2>/dev/null)" = "$2" ] && return 0; sleep 5; done
  echo "loan $1 never reached $2" >&2; return 1
}

# ---------------------------------------------------------------- loan A: liquidation
open_loan A
LOAN_A=$LOAN_ID
say "loan A: ZEC/USD drops to $FLOOD_PRICE (test price); the signer reports flood stage"
curl -sf -X POST "$API/v1/dev/price" -H 'content-type: application/json' -d "{\"usd\": $FLOOD_PRICE}" >/dev/null
wait_health "$LOAN_A" liquidating
say "loan A: lender + policy signer co-sign the liquidation"
"$SIGNER" cosign --policy "$API" --loan-id "$LOAN_A" --kind liquidation --share "$OUT/A/lender.share.json" \
  --server "$LWD" --out-dir "$OUT/A/cosign" | tee "$OUT/A/liquidation.json"
LIQ_TX=$(field "d['txid']" <"$OUT/A/liquidation.json")
record "{\"loan\":\"A\",\"step\":\"liquidation\",\"chain\":\"zcash-testnet\",\"txid\":\"$LIQ_TX\",\"attestationHash\":\"$(field "d['attestationHash']" <"$OUT/A/liquidation.json")\"$(zec_tx "$LIQ_TX")}"
curl -sf -X POST "$API/v1/dev/price" -H 'content-type: application/json' -d '{"usd": null}' >/dev/null
wait_health "$LOAN_A" liquidated
BOOK_A=$(curl -sf "$API/v1/loans/$LOAN_A/escrow" | field "d['intent']['reported']")
record "{\"loan\":\"A\",\"step\":\"loanbook\",\"chain\":\"base-$BASE\",\"health\":\"liquidated\",\"tx\":\"$BOOK_A\"$(base_tx "$BOOK_A")}"
say "loan A: the lender holds the ZEC now; the unrepaid principal leaves the vault's books"
WRITE_OFF=$(bsend "$LENDER_KEY" "$VAULT" "writeOff(bytes32)" "$LOAN_A" | field "d['transactionHash']")
record "{\"loan\":\"A\",\"step\":\"write-off\",\"chain\":\"base-$BASE\",\"tx\":\"$WRITE_OFF\"$(base_tx "$WRITE_OFF")}"

# ---------------------------------------------------------------- loan B: repaid and closed
open_loan B
LOAN_B=$LOAN_ID
say "loan B: the borrower repays principal and interest on Base"
for _ in $(seq 1 30); do
  OWED=$(curl -sf "$API/v1/loans/$LOAN_B/gauge" | field "(lambda o: o['principal'] + o['interest'])(d['state']['owed'])" 2>/dev/null || echo "")
  [ -n "$OWED" ] && [ "$OWED" -gt 0 ] && break
  sleep 2
done
PAY=$((OWED + 10000)) # interest keeps accruing by the second; a cent on top covers it
bsend "$BORROWER_KEY" "$USDC" "approve(address,uint256)" "$VAULT" "$PAY" >/dev/null
REPAY=$(bsend "$BORROWER_KEY" "$VAULT" "repay(bytes32,uint256)" "$LOAN_B" "$PAY" | field "d['transactionHash']")
record "{\"loan\":\"B\",\"step\":\"repay\",\"chain\":\"base-$BASE\",\"tx\":\"$REPAY\",\"micro_usdc\":$PAY$(base_tx "$REPAY")}"
say "loan B: borrower + policy signer co-sign the close"
for _ in $(seq 1 30); do
  "$SIGNER" cosign --policy "$API" --loan-id "$LOAN_B" --kind close --share "$OUT/B/borrower.share.json" \
    --server "$LWD" --out-dir "$OUT/B/cosign" >"$OUT/B/close.json" 2>"$OUT/B/close.err" && break
  grep -q "still owed" "$OUT/B/close.err" || { cat "$OUT/B/close.err"; exit 1; }
  sleep 5 # the signer has not read the repayment yet
done
cat "$OUT/B/close.json"
CLOSE_TX=$(field "d['txid']" <"$OUT/B/close.json")
record "{\"loan\":\"B\",\"step\":\"close\",\"chain\":\"zcash-testnet\",\"txid\":\"$CLOSE_TX\",\"attestationHash\":\"$(field "d['attestationHash']" <"$OUT/B/close.json")\"$(zec_tx "$CLOSE_TX")}"
wait_health "$LOAN_B" closed
BOOK_B=$(curl -sf "$API/v1/loans/$LOAN_B/escrow" | field "d['intent']['reported']")
record "{\"loan\":\"B\",\"step\":\"loanbook\",\"chain\":\"base-$BASE\",\"health\":\"closed\",\"tx\":\"$BOOK_B\"$(base_tx "$BOOK_B")}"

say "done: $OUT/record.jsonl"
cat "$OUT/record.jsonl"
