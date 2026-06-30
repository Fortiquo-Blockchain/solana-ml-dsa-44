# ML-DSA Validator — Step-by-Step Runbook

**Audience:** Anyone running or testing the post-quantum fork  
**Last updated:** July 2026

This guide shows **two ways** to test each phase:

| Way | Best for |
|-----|----------|
| **A — Script** | Fast smoke test; one command boots validator, runs test, tears down |
| **B — Manual CLI** | Learning how it works; keep the chain running and re-run tests |

> **Note on Phase 4:** Phases **0–3** are implemented and testable today. **Phase 4** (full PQ node identity / TLS cutover) is **not built yet** — see [What Phase 4 would be](#phase-4--not-available-yet) at the end.

---

## Before you start (one-time setup)

### What you need

| Requirement | Why |
|-------------|-----|
| **Linux or macOS shell** | Bash required for scripts; validator does not run on native Windows |
| **Rust 1.76.0** | Pinned by `rust-toolchain.toml`; install via [rustup](https://rustup.rs) |
| **Build tools** | `clang`, `cmake`, `pkg-config`, `openssl`, `protobuf` |
| **~20 GB disk** | First `cargo build --release` is large |
| **Two terminal tabs** | Manual method: one for the chain, one for tests |

### Step 0 — Clone and enter the repo

```bash
cd /path/to/solana-ml-dsa-44
```

**What this does:** Moves you into the validator source tree. All commands below assume you are here.

---

### Step 1 — Build the binaries (first time only, ~15–20 min)

```bash
cargo build --release \
  --bin solana-test-validator \
  --bin solana \
  --bin solana-keygen
```

**What this does:** Compiles the local test validator and CLI tools. Output lands in `target/release/`.

---

### Step 2 — Put tools on your PATH (every new terminal)

```bash
export PATH="/path/to/solana-ml-dsa-44/target/release:$PATH"
```

**What this does:** Lets you type `solana`, `solana-keygen`, and `solana-test-validator` without a full path. Add this line to `~/.bashrc` or `~/.zshrc` to make it permanent.

---

### Step 3 — Point the CLI at your local chain (manual method only)

```bash
solana config set --url http://127.0.0.1:8899
```

**What this does:** Tells `solana` commands to talk to your **local** validator, not mainnet.

---

## Phase overview

| Phase | What you are testing | Validator flags needed |
|-------|----------------------|------------------------|
| **0** | On-chain ML-DSA signature **verify** (precompile) | None (plain validator) |
| **1** | **User payment** signed with ML-DSA-44 | None (plain validator) |
| **2** | Validator **consensus votes** signed with ML-DSA-44 | `--ml-dsa-vote <KEYFILE>` |
| **2b** | **Gossip** CRDS values carry PQ signatures | None (automated test, not live CLI demo) |
| **3** | **Block shreds** carry PQ attestation | `--ml-dsa-shred <KEYFILE>` |
| **4** | Full PQ node identity | **Not implemented** |

**Endpoints once the validator is running:**

| Service | URL |
|---------|-----|
| JSON-RPC | `http://127.0.0.1:8899` |
| WebSocket | `ws://127.0.0.1:8900` |

**Stop any validator:** press **Ctrl-C** in its terminal, or `kill` the background process.

---

## Phase 0 — On-chain ML-DSA verify (precompile)

**What success looks like:** A post-quantum signed transaction is **accepted** on-chain; a tampered signature is **rejected**.

---

### Way A — Script (one command)

```bash
bash programs/ml-dsa-tests/demo.sh
```

**What this does:**

1. Builds `solana-test-validator` + `submit_live` example (if missing)
2. Deletes old ledger `~/solana-test-ledger` and starts a **fresh** validator
3. Waits until RPC responds healthy
4. Runs the Phase 0 example (valid PQ tx → confirm; bad sig → reject)
5. Stops the validator when done

**After editing Rust code**, force a rebuild:

```bash
bash programs/ml-dsa-tests/demo.sh --build
```

---

### Way B — Manual CLI (two terminals)

#### Terminal 1 — Start the chain

```bash
cd /path/to/solana-ml-dsa-44
export PATH="$PWD/target/release:$PATH"
solana-test-validator --reset --ledger ~/solana-test-ledger
```

**What this does:**

- `--reset` — wipes the ledger and creates a new genesis (clean slate)
- `--ledger ~/solana-test-ledger` — stores chain data on local disk (fast)
- Starts producing blocks; RPC listens on port **8899**

Leave this terminal running.

---

#### Terminal 2 — Build and run the Phase 0 test

```bash
cd /path/to/solana-ml-dsa-44
export PATH="$PWD/target/release:$PATH"
cargo build --release -p solana-ml-dsa-program-tests --example submit_live
./target/release/examples/submit_live
```

**What this does:**

- Builds the `submit_live` example (only needed once, or after code changes)
- Connects to `127.0.0.1:8899`
- Submits a valid ML-DSA precompile transaction → should **confirm**
- Submits a tampered transaction → should be **rejected**
- Prints keys, signatures, and raw JSON-RPC request/response for each step

You can re-run `./target/release/examples/submit_live` as many times as you like while Terminal 1 is still up.

---

#### Optional — Check the chain is alive

```bash
curl -s http://127.0.0.1:8899 -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}'
```

**Expected:** `"result":"ok"`

```bash
solana slot
```

**Expected:** Slot number increases on each call.

---

## Phase 1 — Post-quantum user payments

**What success looks like:** A SOL transfer whose **fee payer** signs with ML-DSA-44 confirms on-chain; a forged transfer is rejected.

---

### Way A — Script

```bash
bash programs/ml-dsa-tests/demo-transfer.sh
```

**What this does:** Same pattern as Phase 0, but runs `ml_dsa_transfer` instead — airdrop → PQ transfer → balance check → reject tampered tx.

```bash
bash programs/ml-dsa-tests/demo-transfer.sh --build   # after Rust edits
```

---

### Way B — Manual CLI (two terminals)

#### Terminal 1 — Start the chain

```bash
cd /path/to/solana-ml-dsa-44
export PATH="$PWD/target/release:$PATH"
solana-test-validator --reset --ledger ~/solana-test-ledger
```

Same as Phase 0 — plain validator, no extra flags.

---

#### Terminal 2 — Run the Phase 1 test

```bash
cd /path/to/solana-ml-dsa-44
export PATH="$PWD/target/release:$PATH"
solana config set --url http://127.0.0.1:8899
cargo build --release -p solana-ml-dsa-program-tests --example ml_dsa_transfer
./target/release/examples/ml_dsa_transfer
```

**What this does:**

- Mints a fresh ML-DSA keypair
- Airdrops SOL to its address (`sha256(public_key)`)
- Sends a PQ-signed transfer → should **confirm**
- Attempts a forged transfer → should **reject**
- Prints before/after balances from the chain

---

#### Optional — Generate your own ML-DSA keypair

```bash
solana-keygen new --scheme mldsa44 -o /tmp/my-mldsa.bin --no-bip39-passphrase
```

**What this does:** Creates a post-quantum keypair file compatible with this fork.

---

## Phase 2 — Post-quantum validator votes

**What success looks like:** The validator signs its own **consensus votes** with ML-DSA-44; slots and `lastVote` keep climbing; the chain **finalizes**.

> **Important:** PQ voting happens **inside** the validator. The flag goes on the **chain** terminal, not the test terminal.

---

### Way A — Script

```bash
bash programs/ml-dsa-tests/demo-vote.sh
```

**What this does:**

1. Starts validator with `--ml-dsa-vote /tmp/mldsa-vote-keypair.bin`
2. Repoints genesis vote account to the PQ voter address
3. Polls `getSlot` and `getVoteAccounts` to prove finalized slots and `lastVote` advance
4. Stops the validator when done

```bash
bash programs/ml-dsa-tests/demo-vote.sh --build
```

---

### Way B — Manual CLI (two terminals)

#### Terminal 1 — Start the chain **with PQ voting**

```bash
cd /path/to/solana-ml-dsa-44
export PATH="$PWD/target/release:$PATH"
solana-test-validator --reset \
  --ledger ~/solana-test-ledger-mldsa-vote \
  --ml-dsa-vote /tmp/mldsa-vote.bin
```

**What each flag does:**

| Flag | Purpose |
|------|---------|
| `--reset` | Fresh genesis |
| `--ledger ~/solana-test-ledger-mldsa-vote` | Separate ledger from Phase 0/1 (avoids conflicts) |
| `--ml-dsa-vote /tmp/mldsa-vote.bin` | Load or **create** an ML-DSA vote keypair; validator signs votes with it |

Watch startup log for lines like `voter address: ...` and `Phase 2: ML-DSA-44 votes enabled`.

Leave this terminal running.

---

#### Terminal 2 — Observe consensus

```bash
export PATH="/path/to/solana-ml-dsa-44/target/release:$PATH"
solana config set --url http://127.0.0.1:8899
```

**Check slot is advancing:**

```bash
solana slot --commitment finalized
# Wait 10 seconds, run again — number should increase
```

**What this does:** Confirms the chain is producing and **finalizing** blocks on PQ votes.

---

**Check vote account authority is PQ:**

```bash
solana vote-account ~/solana-test-ledger-mldsa-vote/vote-account-keypair.json
```

**What this does:** Shows the vote account. The **Vote Authority** should be the ML-DSA address (not the Ed25519 node identity).

---

**Check lastVote via RPC:**

```bash
curl -s http://127.0.0.1:8899 -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getVoteAccounts","params":[{"commitment":"processed"}]}'
```

**What this does:** Returns vote accounts. `lastVote` should increase over time.

---

#### Baseline — prove Ed25519 voting still works (no flag)

Start the validator **without** `--ml-dsa-vote`:

```bash
solana-test-validator --reset --ledger ~/solana-test-ledger
solana slot --commitment finalized   # should still advance
```

**What this does:** Confirms default Ed25519 voting is unchanged when the PQ flag is off.

---

## Phase 2b — Post-quantum gossip (optional)

**What success looks like:** ML-DSA-signed gossip CRDS values verify and propagate between two live nodes.

There is **no single CLI demo script** for Phase 2b. Use the automated test:

```bash
cd /path/to/solana-ml-dsa-44
cargo test -p solana-gossip --test gossip crds_value
cargo test -p solana-gossip --test gossip ml_dsa_crds_value_propagates_between_live_nodes
```

**What this does:**

- First test: unit tests for PQ signature verify + address binding on CRDS values
- Second test: spins up two gossip nodes; proves an ML-DSA CRDS value propagates A→B

**Note:** A node does **not** yet sign its own gossip identity with PQ (deferred to Phase 4).

---

## Phase 3 — Post-quantum block shreds

**What success looks like:** The validator produces blocks normally; produced shreds carry a valid ML-DSA-44 attestation (verified offline).

> **Important:** A single node's own shreds skip turbine verify, so the demo **stops the validator** and checks shreds from the blockstore directly.

---

### Way A — Script

```bash
bash programs/ml-dsa-tests/demo-shred.sh
```

**What this does:**

1. Starts validator with `--ml-dsa-shred /tmp/mldsa-shred-keypair.bin`
2. Waits for ~20 slots of blocks
3. Stops the validator
4. Runs `verify_ml_dsa_shreds` against the ledger offline

```bash
bash programs/ml-dsa-tests/demo-shred.sh --build
```

---

### Way B — Manual CLI (two terminals)

#### Terminal 1 — Start the chain **with PQ shred signing**

```bash
cd /path/to/solana-ml-dsa-44
export PATH="$PWD/target/release:$PATH"
solana-test-validator --reset \
  --ledger ~/solana-test-ledger-mldsa-shred \
  --ml-dsa-shred /tmp/mldsa-shred.bin
```

**What each flag does:**

| Flag | Purpose |
|------|---------|
| `--ml-dsa-shred /tmp/mldsa-shred.bin` | Sign each FEC-set Merkle root with ML-DSA-44 in addition to Ed25519 |

Optional strict mode (drops invalid PQ shreds at turbine ingress):

```bash
solana-test-validator --reset \
  --ledger ~/solana-test-ledger-mldsa-shred \
  --ml-dsa-shred /tmp/mldsa-shred.bin \
  --ml-dsa-shred-strict
```

Wait until slots advance (~30 seconds), then **stop the validator** with **Ctrl-C**.

---

#### Terminal 2 — Verify shreds offline

```bash
cd /path/to/solana-ml-dsa-44
export PATH="$PWD/target/release:$PATH"
cargo build --release -p solana-ml-dsa-program-tests --example verify_ml_dsa_shreds

IDENTITY=$(solana-keygen pubkey ~/solana-test-ledger-mldsa-shred/validator-keypair.json)
./target/release/examples/verify_ml_dsa_shreds \
  ~/solana-test-ledger-mldsa-shred \
  "$IDENTITY"
```

**What this does:**

- Reads the blockstore directly from the ledger directory
- For each ML-DSA shred: verifies Ed25519 Merkle sig + PQ commitment binding + ML-DSA signature
- Exit code **0** = PASS

---

## Phase 4 — Not available yet

**Planned scope (not built):**

- Flip **node identity** from Ed25519 to ML-DSA address
- Redesign **QUIC/TLS** certificates (today they require Ed25519 keys)
- Node signs its **own gossip identity** with PQ
- Optional: PQ-only shred liveness (today Ed25519 remains load-bearing)

**There are no CLI commands or flags to test Phase 4 today.** Track progress in [`ml-dsa-migration-status.md`](./ml-dsa-migration-status.md).

---

## Quick reference — all script commands

```bash
# One-time build
cargo build --release --bin solana-test-validator --bin solana --bin solana-keygen
export PATH="$PWD/target/release:$PATH"

# Phase 0 — precompile verify
bash programs/ml-dsa-tests/demo.sh

# Phase 1 — PQ user payment
bash programs/ml-dsa-tests/demo-transfer.sh

# Phase 2 — PQ validator votes
bash programs/ml-dsa-tests/demo-vote.sh

# Phase 3 — PQ block shreds
bash programs/ml-dsa-tests/demo-shred.sh
```

Add `--build` after any Rust code change.

---

## Quick reference — manual two-terminal pattern

| Phase | Terminal 1 (chain) | Terminal 2 (test / observe) |
|-------|------------------|-------------------------------|
| **0** | `solana-test-validator --reset --ledger ~/solana-test-ledger` | `./target/release/examples/submit_live` |
| **1** | same as Phase 0 | `./target/release/examples/ml_dsa_transfer` |
| **2** | `solana-test-validator --reset --ledger ~/solana-test-ledger-mldsa-vote --ml-dsa-vote /tmp/mldsa-vote.bin` | `solana slot`, `solana vote-account ...`, `getVoteAccounts` |
| **3** | `solana-test-validator --reset --ledger ~/solana-test-ledger-mldsa-shred --ml-dsa-shred /tmp/mldsa-shred.bin` → wait → Ctrl-C | `./target/release/examples/verify_ml_dsa_shreds <ledger> <identity>` |

---

## Optional — unit tests (no validator needed for most)

```bash
# Phase 0 — SDK + precompile
cargo test -p solana-sdk --features full ml_dsa
cargo test -p solana-ml-dsa-program-tests
cargo test -p solana-ml-dsa-program-tests --test fips204_vectors

# Phase 1 — sigverify + bank execution
cargo test -p solana-perf sigverify
cargo test -p solana-runtime test_ml_dsa_transfer_executes

# Phase 2b — gossip
cargo test -p solana-gossip --test gossip crds_value

# Phase 3 — shreds
cargo test -p solana-ledger --lib shred::merkle
cargo test -p solana-ledger --lib sigverify_shreds
```

**What this does:** Runs automated tests without starting a live validator (faster for developers).

---

## Troubleshooting

| Problem | What to try |
|---------|-------------|
| `solana-test-validator: command not found` | Run `export PATH="$PWD/target/release:$PATH"` |
| `Address already in use` (port 8899) | Another validator is running; stop it or use a different machine |
| Validator won't start | Check ledger path is on local disk (`~/...`), not a slow network mount |
| Demo times out waiting for RPC | Read log: `/tmp/mldsa_demo_validator.log` or the ledger's `validator.log` |
| `error: expected item, found '..'` in build.rs | Git symlinks broken — see [`../CLAUDE.md`](../CLAUDE.md) symlink fix |
| Phase 3 verify fails | Ensure validator ran long enough (~20 slots) before Ctrl-C |
| GPU sigverify drops PQ txs | Use default `solana-test-validator` (GPU off); don't enable perf-libs GPU path |

---

## Related documents

| Document | Purpose |
|----------|---------|
| [`ml-dsa-migration-status.md`](./ml-dsa-migration-status.md) | Management summary: done vs remaining |
| [`ml-dsa-explorer-roadmap.md`](./ml-dsa-explorer-roadmap.md) | Block explorer plan |
| [`ml-dsa-migration.md`](./ml-dsa-migration.md) | Full technical migration strategy |
| [`../CLAUDE.md`](../CLAUDE.md) | Engineering build/environment notes |
