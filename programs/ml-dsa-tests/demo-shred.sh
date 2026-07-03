#!/usr/bin/env bash
# One-command live demo of Phase 3: the validator's own BLOCK SHREDS are signed
# with ML-DSA-44 (post-quantum) on top of Ed25519, while the chain keeps producing.
#
# Boots a fresh solana-test-validator with a known --identity and --ml-dsa-shred
# (which mints an ML-DSA shred keypair). After the chain produces blocks, it stops
# the validator and opens the blockstore offline to prove the produced shreds:
#   1. carry a post-quantum ML-DSA-44 trailer (variant nibble 0xC0/0xD0/0xE0/0xF0), and
#   2. verify under the production three-step check: Ed25519 root vs the leader +
#      commitment binding (ml_dsa_address(pubkey)==in-root commitment) + ML-DSA-44 sig.
# A single node's OWN shreds bypass turbine sigverify (that path verifies peer shreds),
# so we verify offline with the verify_ml_dsa_shreds example.
#
# Run from a WSL shell (bash only):
#   bash programs/ml-dsa-tests/demo-shred.sh            # skips cargo if binaries exist
#   bash programs/ml-dsa-tests/demo-shred.sh --build    # force a rebuild after Rust changes
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO"
export PATH="$REPO/target/release:$PATH"

LEDGER="$HOME/solana-test-ledger-mldsa-shred"
VLOG="/tmp/mldsa_shred_validator.log"
SHRED_KEY="/tmp/mldsa-shred-keypair.bin"
RPC="http://127.0.0.1:8899"

build_targets() { cargo build --release --bin solana-test-validator --bin solana --bin solana-keygen; }
if [ "${1:-}" = "--build" ]; then
    echo "Building (cargo re-checks freshness; only changed crates recompile) ..."
    build_targets
elif [ ! -x target/release/solana-test-validator ] || [ ! -x target/release/solana-keygen ]; then
    echo "Binaries missing -- first-time build (~15-20 min; progress below) ..."
    build_targets
else
    echo "Binaries present -- skipping cargo (re-run with --build after Rust code changes)."
fi
echo "Building the offline ML-DSA shred verifier example (only recompiles if stale) ..."
cargo build --release -p solana-ml-dsa-program-tests --example verify_ml_dsa_shreds

solana config set --url "$RPC" >/dev/null

echo
echo "=== Starting a fresh validator with post-quantum (ML-DSA-44) shred signing ==="
rm -rf "$LEDGER" "$SHRED_KEY"
solana-test-validator --reset --ledger "$LEDGER" --ml-dsa-shred "$SHRED_KEY" >"$VLOG" 2>&1 &
VPID=$!
cleanup() { kill "$VPID" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

echo "Waiting for the validator RPC to come up ..."
ready=0
for _ in $(seq 1 180); do
    if curl -s "$RPC" -X POST -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' 2>/dev/null | grep -q '"result":"ok"'; then
        ready=1
        break
    fi
    sleep 1
done
if [ "$ready" != "1" ]; then
    echo "Validator did not become ready; last log lines:"
    tail -n 40 "$VLOG"
    exit 1
fi

# solana-test-validator auto-generates its identity (the bootstrap leader) and
# writes it here; that identity is the slot leader for every slot on this node.
IDENTITY_PUBKEY="$(solana-keygen pubkey "$LEDGER/validator-keypair.json")"
echo "Node identity (Ed25519, the slot leader for every slot): $IDENTITY_PUBKEY"

echo
echo "=== STEP 1: the flag is active (validator startup log) ==="
grep -m1 'Post-quantum shred signing enabled' "$VLOG" | sed 's/^/  /' \
    || echo "  (startup line not captured; continuing)"
SHRED_ADDR="$(grep -oE 'shred-signer address: [1-9A-HJ-NP-Za-km-z]+' "$VLOG" | head -1 | awk '{print $3}')"
echo "  ML-DSA-44 shred-signer address (sha256 of the PQ public key): ${SHRED_ADDR:-<not found>}"

echo
echo "=== STEP 2: liveness — the chain produces blocks (Ed25519 path intact) ==="
get_slot() { curl -s "$RPC" -X POST -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[{"commitment":"confirmed"}]}' \
    | python3 -c 'import sys,json;print(json.load(sys.stdin).get("result",0))' 2>/dev/null || echo 0; }
s1="$(get_slot)"
echo "  confirmed slot: $s1"
target=$(( ${s1:-0} + 20 ))
echo "  waiting for ~20 more slots of blocks to accumulate ..."
for _ in $(seq 1 90); do
    s="$(get_slot)"
    [ "${s:-0}" -ge "$target" ] 2>/dev/null && break
    sleep 1
done
s2="$(get_slot)"
echo "  confirmed slot: $s2   (advanced => blocks produced while ml_dsa-signing)"

echo
echo "=== STEP 3: stop the validator and verify its produced shreds offline ==="
echo "  (Opens the blockstore directly and runs the production three-step ML-DSA"
echo "   verify on every shred the leader broadcast.)"
kill "$VPID" 2>/dev/null || true
# Wait for the validator to exit and release the blockstore (rocksdb) lock.
for _ in $(seq 1 30); do
    kill -0 "$VPID" 2>/dev/null || break
    sleep 1
done
sleep 2

set +e
./target/release/examples/verify_ml_dsa_shreds "$LEDGER" "$IDENTITY_PUBKEY"
RC=$?
set -e

echo
if [ "$RC" -eq 0 ]; then
    echo "RESULT: PASS -- the validator produced blocks (Ed25519 liveness intact) and its"
    echo "shreds carry valid post-quantum ML-DSA-44 signatures bound to the leader identity."
else
    echo "RESULT: NEEDS REVIEW -- offline verifier exited $RC. Inspect $VLOG and the output above."
    exit 1
fi
