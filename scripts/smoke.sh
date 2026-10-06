#!/usr/bin/env bash
# End-to-end infrastructure check on throwaway ports: anvil → contracts → demo loan →
# policy signer → its API (attestation registered, loan listed, terms match on chain,
# health update lands on chain) → offline Zcash liquidation demo. Cleans up after itself.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHAIN_PORT="${SMOKE_CHAIN_PORT:-8599}"
API_PORT="${SMOKE_API_PORT:-8799}"
RPC="http://127.0.0.1:$CHAIN_PORT"
API="http://127.0.0.1:$API_PORT"
PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

pass() { printf '  \033[32m✓\033[0m %s\n' "$1"; }
fail() { printf '  \033[31m✗\033[0m %s\n' "$1"; exit 1; }
wait_for() { for _ in $(seq 1 60); do curl -sf "$1" >/dev/null 2>&1 && return 0; sleep 1; done; return 1; }

for port in "$CHAIN_PORT" "$API_PORT"; do
  if lsof -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
    fail "port $port is already in use (stale smoke run?); set SMOKE_CHAIN_PORT / SMOKE_API_PORT"
  fi
done

echo "building…"
(cd "$ROOT" && cargo build -q -p lentic-policy -p lentic-signer)

echo "chain on :$CHAIN_PORT"
anvil --port "$CHAIN_PORT" --chain-id 31337 --silent &
PIDS+=($!)
for _ in $(seq 1 30); do cast chain-id --rpc-url "$RPC" >/dev/null 2>&1 && break; sleep 0.5; done

RPC="$RPC" bash "$ROOT/scripts/dev-deploy.sh" >/dev/null
LOAN_ID=$(python3 -c "import json; print(json.load(open('$ROOT/dev/loan.local.json'))['loanId'])")
pass "contracts deployed, demo loan funded"

echo "policy signer on :$API_PORT"
# Started directly (no subshell) so that $! is the server itself and cleanup really stops it.
"$ROOT/target/debug/lentic-policy" serve --base-rpc "$RPC" --listen "127.0.0.1:$API_PORT" --dev-routes \
  --deployment "$ROOT/contracts/deployments/31337.json" --lightwalletd https://testnet.zec.rocks:443 \
  >"$ROOT/dev/smoke-policy.log" 2>&1 &
PIDS+=($!)
wait_for "$API/healthz" || fail "policy signer did not start (dev/smoke-policy.log)"

get() { curl -sf --max-time 60 "$API$1"; }
field() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

[ "$(get /v1/info | field "d['registeredOnBase']")" = "True" ] && pass "policy signer attested and registered on Base" || fail "not registered"
[ "$(get /v1/loans | field "len(d)")" -ge 1 ] && pass "LoanBook lists the loan" || fail "loan missing"

TERMS=$(cat "$ROOT/dev/terms.local.json")
ASSESS=$(curl -sf --max-time 60 -X POST "$API/v1/assess" -H 'content-type: application/json' \
  -d "{\"terms\":$TERMS,\"salt\":\"0x4444444444444444444444444444444444444444444444444444444444444444\",\"loanId\":\"$LOAN_ID\",\"collateralZat\":2000000000,\"priceUsdE8\":80000000000,\"elapsedSecs\":2592000}")
[ "$(echo "$ASSESS" | field "d['matchesOnChain']")" = "True" ] && pass "private terms match the on-chain termsHash" || fail "terms mismatch: $ASSESS"
[ "$(echo "$ASSESS" | field "d['health']")" = "margin_call" ] && pass "LTV assessed (margin call at \$800)" || fail "unexpected health: $ASSESS"

curl -sf --max-time 60 -X POST "$API/v1/loans/$LOAN_ID/health" -H 'content-type: application/json' -d '{"health":"margin_call","oracleRound":1}' >/dev/null
[ "$(get "/v1/loans/$LOAN_ID" | field "d['health']")" = "margin_call" ] && pass "health update written to LoanBook by the signer key" || fail "health not updated"

QUOTE=$(get /v1/attestation | field "d['quote']")
cast send --rpc-url "$RPC" --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  "$(python3 -c "import json; print(json.load(open('$ROOT/contracts/deployments/31337.json'))['attestationVerifier'])")" \
  "register(bytes)" "$QUOTE" >/dev/null && pass "service-generated quote accepted by AttestationVerifier" || fail "quote rejected"

if [ "${SMOKE_ZCASH_NETWORK:-1}" = "1" ]; then
  [ "$(get /v1/zcash | field "d['servers'][0]['ok']")" = "True" ] && pass "lightwalletd reachable (testnet)" || echo "  ! lightwalletd unreachable (network?) — skipped"
fi

"$ROOT/target/debug/lentic-signer" demo >/dev/null && pass "offline Ironwood liquidation: FROST 2-of-3 → anchor refresh → proof → valid tx" || fail "zcash demo failed"
echo "smoke ok"
