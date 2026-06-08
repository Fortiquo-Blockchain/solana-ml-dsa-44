#!/usr/bin/env bash
# One-command live demo of the ML-DSA-44 post-quantum signature precompile.
#
# Boots a fresh local solana-test-validator, runs the narrated demo
# (size contrast -> accept a post-quantum signed tx -> re-fetch it from the
# chain -> reject a tampered one), then tears the validator down.
#
# Run it from a WSL shell (bash only):
#   bash programs/ml-dsa-tests/demo.sh
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO"
export PATH="$REPO/target/release:$PATH"

echo "Building the validator + demo (instant if already built) ..."
cargo build --release --bin solana-test-validator >/dev/null 2>&1
cargo build --release -p solana-ml-dsa-program-tests --example submit_live >/dev/null 2>&1

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

./target/release/examples/submit_live
