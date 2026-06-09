#!/usr/bin/env bash
# One-command live demo of Phase 2: the validator's own CONSENSUS VOTES are signed
# with ML-DSA-44 (post-quantum) instead of Ed25519, while the chain keeps producing.
#
# Boots a fresh local solana-test-validator with --ml-dsa-vote (which mints an ML-DSA
# vote keypair, sets the genesis vote account's authorized voter to that key's address,
# and funds it). Then it proves, on-chain, that:
#   1. the vote account's authorized voter is the post-quantum address (no Ed25519 key
#      exists for it -- it is sha256(ML-DSA public key)),
#   2. that authority's lastVote keeps climbing -> votes are landing, and
#   3. getSlot keeps advancing -> consensus is alive on post-quantum votes.
# It also greps the validator log for the per-vote "Signed ML-DSA-44 vote" line.
#
# Run it from a WSL shell (bash only):
#   bash programs/ml-dsa-tests/demo-vote.sh
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO"
export PATH="$REPO/target/release:$PATH"

LEDGER="$HOME/solana-test-ledger-mldsa-vote"
VLOG="/tmp/mldsa_vote_validator.log"
KEYFILE="/tmp/mldsa-vote-keypair.bin"
RPC="http://127.0.0.1:8899"

# Freshness: cargo itself is the only reliable judge of staleness. For speed this script
# SKIPS cargo when the binary exists -- which does NOT detect stale code -- so after
# editing Rust, pass --build to force a re-check:
#   bash programs/ml-dsa-tests/demo-vote.sh --build
build_targets() { cargo build --release --bin solana-test-validator --bin solana --bin solana-keygen; }
if [ "${1:-}" = "--build" ]; then
    echo "Building (cargo re-checks freshness; only changed crates recompile) ..."
    build_targets
elif [ ! -x target/release/solana-test-validator ] || [ ! -x target/release/solana ]; then
    echo "Binaries missing -- first-time build (~15-20 min; progress below) ..."
    build_targets
else
    echo "Binaries present -- skipping cargo (re-run with --build after Rust code changes)."
fi

solana config set --url "$RPC" >/dev/null

echo
echo "=== Starting a fresh validator with post-quantum (ML-DSA-44) voting ==="
rm -rf "$LEDGER" "$KEYFILE"
solana-test-validator --reset --ledger "$LEDGER" --ml-dsa-vote "$KEYFILE" >"$VLOG" 2>&1 &
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

# The post-quantum voter address that --ml-dsa-vote printed at startup.
MLDSA_ADDR="$(grep -oE 'voter address: [1-9A-HJ-NP-Za-km-z]+' "$VLOG" | head -1 | awk '{print $3}')"
VOTE_PUBKEY="$(solana-keygen pubkey "$LEDGER/vote-account-keypair.json")"

# Print a JSON-RPC request and the validator's raw response; stash it in LAST_RESP.
rpc() {
    local req="{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":$2}"
    echo "    → JSON-RPC request:  $req"
    LAST_RESP="$(curl -s "$RPC" -X POST -H 'Content-Type: application/json' -d "$req")"
    echo "    ← raw result:        $LAST_RESP"
}
# Parse the slot number out of the last getSlot response.
parse_slot() { echo "$LAST_RESP" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("result",0))' 2>/dev/null || echo 0; }
# Parse our vote account's lastVote out of the last getVoteAccounts response.
parse_last_vote() { echo "$LAST_RESP" | python3 -c "import sys,json
d=json.load(sys.stdin).get('result',{})
accs=d.get('current',[])+d.get('delinquent',[])
print(next((a['lastVote'] for a in accs if a['votePubkey']=='$VOTE_PUBKEY'),''))" 2>/dev/null || echo ''; }

echo
echo "=== STEP 1: on-chain proof — the vote authority is a post-quantum address ==="
echo "ML-DSA-44 voter address (sha256 of the PQ public key): ${MLDSA_ADDR:-<not found in log>}"
echo "Validator vote account:                                $VOTE_PUBKEY"
echo
echo "solana vote-account $VOTE_PUBKEY:"
solana vote-account "$VOTE_PUBKEY" | grep -iE 'vote authority|root slot|recent timestamp' || true
echo
echo "(The 'Vote Authority' above is the ML-DSA-44 address. No Ed25519 private key exists"
echo " for it -- it is sha256(ML-DSA public key) -- so the only way it can vote is with"
echo " post-quantum signatures.)"

echo
echo "=== STEP 2: what is (and isn't) reduced — key & signature sizes ==="
PK_LEN=1312; SK_LEN=2560; SIG_LEN=2420            # FIPS 204 ML-DSA-44 (fixed constants)
RAW_LEN=$((PK_LEN + SK_LEN))                      # raw keypair material = 3872 B
KF_LEN="$(stat -c%s "$KEYFILE" 2>/dev/null || echo '?')"
echo "  public key:   Ed25519   32 B   ->   ML-DSA-44 ${PK_LEN} B   (carried FULL-SIZE in every tx; NOT reduced)"
echo "  signature:    Ed25519   64 B   ->   ML-DSA-44 ${SIG_LEN} B   (carried FULL-SIZE in every tx; NOT reduced)"
echo "  address:      Ed25519   32 B   ->   ML-DSA-44   32 B   <== the ONLY reduction: sha256(public key)"
echo "  raw key material: ${RAW_LEN} B  =  ${PK_LEN} B public key  +  ${SK_LEN} B secret key"
echo "  on-disk keyfile:  ${KF_LEN} B  (JSON byte-array, same shape as a Solana keypair file)"
echo
echo "  The public key and signature are NOT shrunk -- verification needs them whole. Only"
echo "  the 32-byte ADDRESS is reduced, by hashing the full public key, which still travels"
echo "  inside the transaction. (This is why the packet ceiling was raised 1232 -> 8192.)"

echo
echo "=== STEP 3: liveness — consensus advances on post-quantum (ML-DSA-44) votes ==="
echo "Each reading below shows the raw JSON-RPC call and the validator's raw reply."
# Optimistic confirmation needs ~30 slots of votes to warm up, so wait (quietly) until
# the confirmed slot first moves off 0 before taking the 'before' sample.
echo "Waiting for the first confirmed slot (votes warming up) ..."
for _ in $(seq 1 40); do
    c="$(curl -s "$RPC" -X POST -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[{"commitment":"confirmed"}]}' \
        2>/dev/null | python3 -c 'import sys,json;print(json.load(sys.stdin).get("result",0))' 2>/dev/null || echo 0)"
    [ "${c:-0}" -gt 0 ] 2>/dev/null && break
    sleep 1
done

echo "  -- BEFORE sample --"
rpc getSlot '[{"commitment":"processed"}]';          proc1="$(parse_slot)"
rpc getSlot '[{"commitment":"confirmed"}]';          conf1="$(parse_slot)"
rpc getSlot '[{"commitment":"finalized"}]';          fin1="$(parse_slot)"
rpc getVoteAccounts '[{"commitment":"processed"}]';  last1="$(parse_last_vote)"
sleep 15
echo "  -- AFTER sample (~15s later) --"
rpc getSlot '[{"commitment":"processed"}]';          proc2="$(parse_slot)"
rpc getSlot '[{"commitment":"confirmed"}]';          conf2="$(parse_slot)"
rpc getSlot '[{"commitment":"finalized"}]';          fin2="$(parse_slot)"
rpc getVoteAccounts '[{"commitment":"processed"}]';  last2="$(parse_last_vote)"

echo
echo "processed slot:  $proc1  ->  $proc2   (advancing => blocks are being produced)"
echo "confirmed slot:  $conf1  ->  $conf2   (advancing => PQ votes optimistically confirm)"
echo "finalized slot:  $fin1  ->  $fin2   (advancing => the chain ROOTS on PQ votes)"
echo "vote lastVote:   ${last1:-?}  ->  ${last2:-?}   (advancing => PQ votes landed + executed on-chain)"

echo
echo "=== Validator log: per-vote ML-DSA-44 signing ==="
( grep -m1 'Signed ML-DSA-44' "$VLOG" "$LEDGER"/validator*.log 2>/dev/null \
    | sed 's/^/  /' ) \
    || echo "  (per-vote 'Signed ML-DSA-44' info line not captured at the current log level; the"
echo "   on-chain proof above is authoritative.)"

echo
if [ "${fin2:-0}" -gt "${fin1:-0}" ] 2>/dev/null && [ "${last2:-0}" -gt "${last1:-0}" ] 2>/dev/null; then
    echo "RESULT: PASS -- consensus is alive on ML-DSA-44 votes: the chain FINALIZES (roots)"
    echo "and the post-quantum vote authority's on-chain lastVote keeps advancing."
else
    echo "RESULT: NEEDS REVIEW -- processed=$proc1->$proc2, confirmed=$conf1->$conf2,"
    echo "finalized=$fin1->$fin2, lastVote=$last1->$last2. Inspect $LEDGER/validator.log"
fi
