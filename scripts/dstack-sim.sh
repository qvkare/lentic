#!/usr/bin/env bash
# The policy signer's production attestation path (`--attestor dstack`) against dstack's
# guest-agent simulator, on throwaway ports: the signer takes its Base key from dstack GetKey
# and its TDX quote from GetQuote, as in a CVM; AttestationVerifier pins the quote's
# measurement; the quote registers the key; the key then writes to LoanBook.
#
# What is simulated is the hardware and the KMS. The simulator replays a TDX quote captured from
# a real dstack CVM and patches REPORTDATA per request, so the quote's signature no longer
# verifies and DCAP is mocked (MockDcapAttestation); keys derive from fixture app keys instead of
# the dstack KMS. Everything on Lentic's side is the code that runs in the CVM.
#
# Needs Docker: the simulator image is built from deploy/dstack/simulator (dstack v0.6.0) on
# first use, which takes a few minutes.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHAIN_PORT="${SIM_CHAIN_PORT:-8598}"
API_PORT="${SIM_API_PORT:-8798}"
DSTACK_PORT="${SIM_DSTACK_PORT:-8698}"
IMAGE="${DSTACK_SIM_IMAGE:-lentic-dstack-simulator:v0.6.0}"
RPC="http://127.0.0.1:$CHAIN_PORT"
API="http://127.0.0.1:$API_PORT"
# Read by the dstack SDK in place of /var/run/dstack.sock.
export DSTACK_SIMULATOR_ENDPOINT="http://127.0.0.1:$DSTACK_PORT"
DEPLOYMENT_FILE=deployments/local/dstack-sim.json
DEP="$ROOT/contracts/$DEPLOYMENT_FILE"
DEPLOYER_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
# The production config id (deploy/dstack/docker-compose.yml).
CONFIG_ID=lentic-policy-v1
POLICY="$ROOT/target/debug/lentic-policy"
CONTAINER="lentic-dstack-sim-$$"
PIDS=()
DEV_DIR=$(mktemp -d)
cleanup() {
  for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null && wait "$p" 2>/dev/null || true; done
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
  rm -rf "$DEV_DIR" "$DEP"
}
trap cleanup EXIT

pass() { printf '  \033[32m✓\033[0m %s\n' "$1"; }
fail() { printf '  \033[31m✗\033[0m %s\n' "$1"; exit 1; }
wait_for() { for _ in $(seq 1 60); do curl -sf "$1" >/dev/null 2>&1 && return 0; sleep 1; done; return 1; }
get() { curl -sf --max-time 60 "$API$1"; }
field() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
json() { python3 -c "import json; print(json.load(open('$DEP'))['$1'])"; }

for port in "$CHAIN_PORT" "$API_PORT" "$DSTACK_PORT"; do
  if lsof -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
    fail "port $port is already in use; set SIM_CHAIN_PORT / SIM_API_PORT / SIM_DSTACK_PORT"
  fi
done

echo "building…"
(cd "$ROOT" && cargo build -q -p lentic-policy -p lentic-signer)
if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "building the dstack simulator image (first run only)…"
  docker build -q -t "$IMAGE" "$ROOT/deploy/dstack/simulator" >/dev/null
fi

echo "dstack simulator on :$DSTACK_PORT"
docker run -d --name "$CONTAINER" -p "127.0.0.1:$DSTACK_PORT:8090" "$IMAGE" >/dev/null
up=0
for _ in $(seq 1 30); do
  curl -sf -X POST -H 'content-type: application/json' -d '{}' "$DSTACK_SIMULATOR_ENDPOINT/Info" >/dev/null 2>&1 && { up=1; break; }
  sleep 1
done
[ "$up" = 1 ] || { docker logs "$CONTAINER" 2>&1 | tail -20; fail "simulator did not start"; }

MEASUREMENT=$("$POLICY" measurement --attestor dstack)
pass "GetQuote: measurement keccak256(MRTD ‖ RTMR0..3) = $MEASUREMENT"

echo "chain on :$CHAIN_PORT"
anvil --port "$CHAIN_PORT" --chain-id 31337 --silent &
PIDS+=($!)
for _ in $(seq 1 30); do cast chain-id --rpc-url "$RPC" >/dev/null 2>&1 && break; sleep 0.5; done

mkdir -p "$(dirname "$DEP")"
RPC="$RPC" DEV_DIR="$DEV_DIR" DEPLOYMENT_FILE="$DEPLOYMENT_FILE" POLICY_MEASUREMENT="$MEASUREMENT" \
  bash "$ROOT/scripts/dev-deploy.sh" >/dev/null 2>&1
VERIFIER=$(json attestationVerifier)
LOAN_ID=$(python3 -c "import json; print(json.load(open('$DEV_DIR/loan.local.json'))['loanId'])")
[ "$(cast call --rpc-url "$RPC" "$VERIFIER" 'expectedMeasurement()(bytes32)')" = "$MEASUREMENT" ] \
  && pass "AttestationVerifier pins that measurement (mock DCAP), demo loan funded" || fail "verifier not pinned"

echo "policy signer (--attestor dstack) on :$API_PORT"
"$POLICY" serve --attestor dstack --config-id "$CONFIG_ID" --base-rpc "$RPC" --listen "127.0.0.1:$API_PORT" \
  --deployment "$DEP" --dev-routes --loans-dir "$DEV_DIR/loans" --state-dir "$DEV_DIR/policy" \
  --lightwalletd https://testnet.zec.rocks:443 --price-usd 800 >"$DEV_DIR/policy.log" 2>&1 &
PIDS+=($!)
wait_for "$API/healthz" || { cat "$DEV_DIR/policy.log"; fail "policy signer did not start"; }

INFO=$(get /v1/info)
SIGNER=$(cast to-check-sum-address "$(echo "$INFO" | field "d['signerAddress']")")
CONFIG_HASH=$(echo "$INFO" | field "d['configHash']")
[ "$(echo "$INFO" | field "d['attestor']")" = "dstack" ] && [ "$(echo "$INFO" | field "d['registeredOnBase']")" = "False" ] \
  && pass "GetKey: Base key $SIGNER derived by dstack, not registered yet" || fail "unexpected info: $INFO"
[ "$CONFIG_HASH" = "$(cast keccak "$CONFIG_ID")" ] || fail "config hash $CONFIG_HASH"

# Gas for the key's own transactions (a CVM operator funds the address the same way).
cast rpc --rpc-url "$RPC" anvil_setBalance "$SIGNER" 0xde0b6b3a7640000 >/dev/null
HEALTH='{"health":"margin_call","oracleRound":1}'
CODE=$(curl -s -o "$DEV_DIR/refused.json" -w '%{http_code}' --max-time 60 -X POST "$API/v1/loans/$LOAN_ID/health" \
  -H 'content-type: application/json' -d "$HEALTH")
[ "$CODE" != 200 ] && grep -qiE "$(cast sig 'NotPolicySigner(address)' | cut -c3-)|NotPolicySigner" "$DEV_DIR/refused.json" \
  && pass "LoanBook refuses the key before registration (NotPolicySigner)" || fail "unregistered write: $CODE $(cat "$DEV_DIR/refused.json")"

QUOTE=$(get /v1/attestation | field "d['quote']")
QUOTE_INFO=$(python3 - "$QUOTE" "$SIGNER" "$CONFIG_HASH" <<'PY'
import sys
quote, signer, config = (bytes.fromhex(a[2:]) for a in sys.argv[1:4])
assert int.from_bytes(quote[0:2], "little") == 4, "quote version"
assert int.from_bytes(quote[4:8], "little") == 0x81, "TEE type is not TDX"
report_data = quote[568:632]
assert report_data[:20] == signer and report_data[20:32] == bytes(12) and report_data[32:] == config, "REPORTDATA"
print(f"TDX v4 quote, {len(quote)} bytes")
PY
) && pass "$QUOTE_INFO, REPORTDATA = signer ‖ 0 ‖ keccak256(\"$CONFIG_ID\")" || fail "quote does not bind the key"

# The development attestor's registers, for the same key: other code, so a different measurement.
DEV_QUOTE=$(python3 - "$SIGNER" "$CONFIG_HASH" <<'PY'
import sys
quote = bytearray(48 + 584)
quote[0] = 4
quote[184:232] = b"\x11" * 48
for i in range(192):
    quote[376 + i] = 0x20 + i // 48
quote[568:588] = bytes.fromhex(sys.argv[1][2:])
quote[600:632] = bytes.fromhex(sys.argv[2][2:])
print("0x" + quote.hex())
PY
)
if cast call --rpc-url "$RPC" "$VERIFIER" "register(bytes)" "$DEV_QUOTE" >/dev/null 2>"$DEV_DIR/dev-quote.err"; then
  fail "a development quote registered"
fi
grep -qiE "$(cast sig 'MeasurementMismatch(bytes32)' | cut -c3-)|MeasurementMismatch" "$DEV_DIR/dev-quote.err" \
  && pass "a development-attestor quote for the same key is rejected (MeasurementMismatch)" \
  || fail "unexpected revert: $(cat "$DEV_DIR/dev-quote.err")"

cast send --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" "$VERIFIER" "register(bytes)" "$QUOTE" >/dev/null
[ "$(cast call --rpc-url "$RPC" "$VERIFIER" 'isPolicySigner(address)(bool)' "$SIGNER")" = "true" ] \
  && [ "$(get /v1/info | field "d['registeredOnBase']")" = "True" ] \
  && pass "dstack quote registered: isPolicySigner($SIGNER) = true" || fail "registration failed"

CODE=$(curl -s -o "$DEV_DIR/health.json" -w '%{http_code}' --max-time 60 -X POST "$API/v1/loans/$LOAN_ID/health" \
  -H 'content-type: application/json' -d "$HEALTH")
[ "$CODE" = 200 ] || { tail -5 "$DEV_DIR/policy.log"; fail "health write: $CODE $(cat "$DEV_DIR/health.json")"; }
TX=$(field "d['tx']" <"$DEV_DIR/health.json")
FROM=$(cast tx --rpc-url "$RPC" "$TX" from)
[ "$(get "/v1/loans/$LOAN_ID" | field "d['health']")" = "margin_call" ] && [ "$(cast to-check-sum-address "$FROM")" = "$SIGNER" ] \
  && pass "health update written to LoanBook by the dstack-derived key ($TX)" || fail "health not updated"

echo "dstack simulator ok"
