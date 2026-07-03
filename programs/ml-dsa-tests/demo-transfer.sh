#!/usr/bin/env bash
# One-command live demo of Phase 1: a SOL transfer whose fee payer signs with
# ML-DSA-44 (post-quantum) and whose address is sha256(public key).
#
# Boots a fresh local solana-test-validator, runs the demo (airdrop -> ML-DSA
# transfer confirmed on-chain -> tampered transfer rejected), then tears it down.
#
# Run it from a WSL shell (bash only):
#   bash programs/ml-dsa-tests/demo-transfer.sh
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO"
export PATH="$REPO/target/release:$PATH"

# Freshness: cargo itself is the only reliable judge (it compares source fingerprints
# against the built artifacts and recompiles only what changed). For speed this script
# SKIPS cargo when the binaries exist -- which does NOT detect stale code -- so after
# editing Rust, pass --build to force cargo to re-check:
#   bash programs/ml-dsa-tests/demo-transfer.sh --build
# NOTE: one cargo invocation builds both targets; two separate ones thrash shared-crate
# feature sets and recompile everything each run.
build_targets() { cargo build --release --bin solana-test-validator --example ml_dsa_transfer; }
if [ "${1:-}" = "--build" ]; then
    echo "Building (cargo re-checks freshness; only changed crates recompile) ..."
    build_targets
elif [ ! -x target/release/solana-test-validator ] || [ ! -x target/release/examples/ml_dsa_transfer ]; then
    echo "Binaries missing -- first-time build (~10-15 min; progress below) ..."
    build_targets
else
    echo "Binaries present -- skipping cargo (re-run with --build after Rust code changes)."
fi

echo "Starting a fresh local validator ..."
rm -rf ~/solana-test-ledger
solana-test-validator --reset --ledger ~/solana-test-ledger >/tmp/mldsa_demo_validator.log 2>&1 &
VPID=$!
cleanup() { kill "$VPID" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

ready=0
for _ in $(seq 1 120); do
    if curl -s http://127.0.0.1:8899 -X POST -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}' 2>/dev/null | grep -q '"result":"ok"'; then
        ready=1
        break
    fi
    sleep 1
done
if [ "$ready" != "1" ]; then
    echo "Validator did not become ready; last log lines:"
    tail -n 30 /tmp/mldsa_demo_validator.log
    exit 1
fi

./target/release/examples/ml_dsa_transfer
