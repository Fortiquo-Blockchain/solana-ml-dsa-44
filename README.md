# infinia — Post-Quantum Solana Validator (ML-DSA-44)

A fork of the Solana validator (Anza/Agave lineage, Rust/Cargo workspace, crates still
branded `solana`, version `2.0.0`) that **migrates the validator's signatures from
Ed25519 to ML-DSA-44** — the post-quantum signature standardized by NIST in
**FIPS 204** (finalized Aug 2024).

> **In one line:** swap the chain's signature algorithm for a quantum-resistant one —
> feasible on a network we control, deliberately **not** compatible with live Solana.

**Status:** Phases 0–3 delivered & verified end-to-end on a local validator. Phase 4
(flip the node identity itself to an ML-DSA address) is deferred — see
[Migration phases](#migration-phases). The full strategy and the engineering details
live in [`docs/ml-dsa-migration.md`](docs/ml-dsa-migration.md) and
[`CLAUDE.md`](CLAUDE.md). What's still **pending** to take it further is catalogued in
[`docs/ml-dsa-remaining-work.md`](docs/ml-dsa-remaining-work.md).

---

## The goal

Replace the signature scheme the validator uses everywhere — today **Ed25519** — with
**ML-DSA-44**. Signing/verifying work the same way conceptually; **all the difficulty is
size**. The new keys and signatures are ~40× larger, and the system was built assuming
signatures are tiny:

|             | Today (Ed25519) | Target (ML-DSA-44) | Bigger by |
| ----------- | --------------: | -----------------: | --------: |
| Public key  |        32 bytes |    **1,312 bytes** |      ~41× |
| Signature   |        64 bytes |    **2,420 bytes** |      ~38× |

Two core assumptions break under big keys, and both are solvable only on a network we own:

- **Wall #1 — an account's address _is_ its public key.** A 1,312-byte key can't be a
  32-byte address. **Fix:** `address = sha256(public_key)` (32 bytes), carry the full key
  inside the transaction. Accounts-db / PDAs / base58 stay unchanged.
- **Wall #2 — a whole transaction must fit in one ~1,232-byte packet.** One ML-DSA
  signature + key is already ~3,732 bytes (~3× over). **Fix:** raise `PACKET_DATA_SIZE`
  to **8192** (we control the wire). This is what makes the fork **incompatible with live
  Solana** — we can't change that limit on everyone else's machines.

---

## The five signing surfaces

The network signs five separate things; each is an independent upgrade. Surfaces 1–4
share one signing engine, so once user payments work, votes/chatter/shreds largely come
along. Surface 5 is fully self-contained, which is why it's the safe warm-up.

| # | Surface | What it is | Status |
| - | ------- | ---------- | ------ |
| 5 | **App-level checks** (precompile) | An on-chain ML-DSA verify capability apps can call | ✅ Phase 0 |
| 1 | **User payments** | Wallets signing transactions — the headline use case | ✅ Phase 1 |
| 2 | **Validator votes** | How validators agree on the chain (a vote _is_ a transaction) | ✅ Phase 2a |
| 3 | **Node-to-node chatter** (gossip) | How validators find and trust each other | ✅ Phase 2b core |
| 4 | **Block broadcasting** (shreds/turbine) | The block producer signing what it publishes | ✅ Phase 3 |

Everything **coexists** with Ed25519 — the post-quantum path is additive/flag-gated, so a
single-node validator never stalls. The node's own network *identity* (QUIC/TLS, gossip,
repair) is still Ed25519; flipping it is Phase 4.

---

## Migration phases

| Phase | Surface | "Done" means | Key flag / artifact |
| ----- | ------- | ------------ | ------------------- |
| **0** | App-level precompile | ML-DSA verify instruction; valid passes, tampered rejected; live ~3.9 KB tx confirms | program id `6Cjqvizo…VbNSs`, `feature: None` |
| **1** | User payments | A fee payer signs a SOL transfer with ML-DSA (`address = sha256(pubkey)`); validator verifies, executes, confirms; forged one rejected | `0x00` lead-byte tx format, CPU sigverify |
| **2a** | Validator votes | The validator's own consensus votes are ML-DSA-signed; chain confirms + **finalizes** on them | `--ml-dsa-vote <KEYFILE>` |
| **2b** | Node chatter (gossip) | CRDS values carry an ML-DSA signature + `sha256(pubkey)==identity` binding; propagates + verifies across two live nodes | `CrdsSignature` enum |
| **3** | Block broadcasting (shreds) | Each FEC-set Merkle root is *also* ML-DSA-signed; advisory verify at turbine ingress (opt-in gating) | `--ml-dsa-shred <KEYFILE>` · `--ml-dsa-shred-strict` |
| **4** | Node identity flip | _Deferred._ Make `id()` the ML-DSA address | blocked by QUIC/TLS Ed25519 cert wall + fixed-width Ping/Pong/Prune sigs |

What's **realistic**: a self-hosted demo network proving quantum-resistant user payments
(and more) end to end. What's **not**: interoperating with live Solana, or matching
today's throughput (bigger signatures, no hardware fast-path — ML-DSA verify ≈ **2.3×**
ed25519, now priced as `ML_DSA_VERIFY_COST = 5310 CU`).

---

## Environment & prerequisites

Standard Cargo workspace. **Rust is pinned to 1.76.0** by `rust-toolchain.toml` on every
platform (rustup honors it automatically inside the repo). Pick your host:

| Host | Build/run natively? | Extra steps |
| ---- | ------------------- | ----------- |
| **Linux** (Ubuntu/Debian) | ✅ Yes | apt deps only |
| **macOS** (Apple Silicon or Intel) | ✅ Yes | brew deps only |
| **Windows** | ❌ No — build via **WSL2** | WSL deps + symlink fix (below) |

### Linux (Ubuntu/Debian) — native

```bash
sudo apt-get update
sudo apt-get install libssl-dev libudev-dev pkg-config zlib1g-dev llvm clang cmake make libprotobuf-dev protobuf-compiler
```

On Fedora: `sudo dnf install openssl-devel systemd-devel pkg-config zlib-devel llvm clang cmake make protobuf-devel protobuf-compiler perl-core`.
Then build & run directly — none of the WSL/symlink notes below apply.

### macOS — native

```bash
xcode-select --install                       # clang, make, git
brew install protobuf cmake openssl pkg-config
```

Then build & run directly. (Symlinks and line-endings are fine on macOS; the WSL section
below is Windows-only.)

> **Note:** the macOS dep list above is the standard Solana/Agave set and has **not yet
> been verified on a clean macOS build in this fork** — all delivery so far was on
> WSL2/Ubuntu. If `cargo build` errors on a missing native lib, that brew line is the first
> thing to revisit (likely add `openssl`/`pkg-config` env hints or extra packages). Please
> update this note once a macOS build is confirmed.

### Windows — via WSL2 (native Windows is **not** supported)

The validator's symlinks and toolchain only work under WSL, so on Windows you **edit on
Windows but build/run in WSL2 Ubuntu** (same files underneath: `/mnt/d` = `D:\`).

| Role | Launch from | Runs in |
| ---- | ----------- | ------- |
| Editing / Claude Code | Windows terminal (PowerShell) | Windows |
| Cursor + rust-analyzer | a WSL shell (`cursor .`) | Remote-WSL |
| **Build / run / git** | a WSL shell | **WSL2 Ubuntu** |

1. Install the **Ubuntu build deps** (the apt list above) inside WSL.
2. **Use WSL git for this repo, never Windows git** — `core.symlinks=true` +
   `core.autocrlf=input` are set repo-locally; Windows git re-breaks the 26 symlinks and
   churns line-endings.
3. **First-build symlink fix.** A Windows checkout (`core.symlinks=false`) turns the
   repo's **26 git symlinks** (mode `120000`, e.g. `*/build.rs`, a few `.sh`,
   `sdk/package.json`) into plain text files, so `cargo build` fails immediately with
   `error: expected item, found '..' --> frozen-abi/macro/build.rs:1:1`. Restore them from
   WSL (already applied in this checkout):

   ```bash
   git config core.symlinks true
   git ls-files -s | grep '^120000' | awk '{print $4}' | while read f; do ln -sfn "$(cat "$f")" "$f"; done
   git config core.autocrlf input    # silences the ~2042 CRLF-only "modified" files
   ```

---

## Building

From the repo root **in a WSL shell**:

```bash
cargo build --release --bin solana-test-validator --bin solana --bin solana-keygen
```

Binaries land in `target/release/`. First build ≈ **20 min**; incremental builds are fast.

---

## Running locally

There are **two ways** to run/observe the chain locally.

### A) One combined demo script (throwaway validator)

Each script boots a fresh validator, runs a narrated demo (prints real keys/signatures and
the raw JSON-RPC request + response at every chain interaction — nothing is staged), then
tears it down. Add `--build` after editing Rust to force a recompile.

```bash
bash programs/ml-dsa-tests/demo.sh           # Phase 0 — precompile verifies a PQ signature
bash programs/ml-dsa-tests/demo-transfer.sh  # Phase 1 — a PQ-signed SOL transfer
bash programs/ml-dsa-tests/demo-vote.sh      # Phase 2 — the validator's own votes are PQ
bash programs/ml-dsa-tests/demo-shred.sh     # Phase 3 — the validator's block shreds are PQ-signed
```

### B) Two terminals (long-running chain + re-runnable tests)

Keep one validator up; fire examples at it (RPC `127.0.0.1:8899`, WS `:8900`).

```bash
# Terminal 1 — the chain, leave it running:
export PATH="$PWD/target/release:$PATH"
solana-test-validator --reset --ledger ~/solana-test-ledger
```

```bash
# Terminal 2 — re-runnable; each run mints fresh wallets + airdrops:
export PATH="$PWD/target/release:$PATH"
./target/release/examples/submit_live        # Phase 0
./target/release/examples/ml_dsa_transfer    # Phase 1
```

For **Phase 2** the post-quantum voting happens *inside* the validator, so the flag goes
on the chain and the second terminal just observes:

```bash
# Terminal 1 — the chain, voting post-quantum:
solana-test-validator --reset --ledger ~/solana-test-ledger-mldsa-vote --ml-dsa-vote /tmp/mldsa-vote.bin
# Terminal 2 — slots + the vote account's lastVote keep climbing:
solana slot --commitment finalized
```

> **Tip:** any local path works for `--ledger`. **On WSL specifically**, keep ledgers on
> ext4 (`~/...`), not `/mnt/d` (slow drvfs I/O). The Phase-1 CPU-sigverify caveat applies
> on every host: run with GPU sigverify disabled (the default for `solana-test-validator`),
> or `0x00` packets are dropped fail-closed on the GPU path.

---

## Testing

The conformance suite's JS half needs Node: on Linux/macOS run `node …` directly; on a
Windows host there's no Node inside WSL, so run that one line from a Windows terminal.

```bash
# Phase 0 — precompile + conformance
cargo test -p solana-sdk --features full ml_dsa                   # valid passes, tampered fails
cargo test -p solana-ml-dsa-program-tests                         # integration via solana-program-test
cargo test -p solana-ml-dsa-program-tests --test fips204_vectors  # NIST FIPS 204 KAT
node programs/ml-dsa-tests/cross-impl/check_vectors.mjs           # @noble cross-impl (needs Node; on a Windows host run from Windows)

# Phase 1 — user payments
cargo test -p solana-sdk ml_dsa_transaction --lib                 # sign/verify round-trip
cargo test -p solana-perf sigverify                               # 0x00 routing + Ed25519 regression
cargo test -p solana-runtime test_ml_dsa_transfer_executes        # bank execution

# Phase 2b — gossip
cargo test -p solana-gossip --test gossip crds_value              # CrdsSignature verify/binding + Ed25519 regression
cargo test -p solana-gossip --test gossip ml_dsa_crds_value_propagates_between_live_nodes

# Phase 3 — shreds / turbine
cargo test -p solana-ledger --lib shred::merkle                   # signing round-trip, resigned set, partial recovery
cargo test -p solana-ledger --lib sigverify_shreds               # verify: honest / wrong leader / tampered / forged trailer / strict-drop
```

The conformance suite pins our `MlDsaKeypair::from_seed` keygen and the `fips204`
sign/verify core to the **official NIST ACVP FIPS 204 vectors** and proves the Rust
validator and the sibling `@noble/post-quantum` JS wallet produce **byte-identical** keys
and signatures (empty context). Signing is hedged (randomized); byte-KATs use the
deterministic path.

> **Known pre-existing red tests on this fork (not regressions):** 10 `solana-ledger`
> `shred` tests + 3 `crds_gossip_pull` bloom tests hardcode the old 1,232 packet size and
> fail because the fork sets `PACKET_DATA_SIZE = 8192`. Confirmed identical with the
> ML-DSA work stashed.

---

## What changed in the codebase (orientation map)

| Area | Change |
| ---- | ------ |
| `fips204 = "0.4.6"` | Pure-Rust FIPS 204 (MSRV 1.70 → builds on pinned 1.76); always empty context |
| `sdk/src/ml_dsa_keypair.rs` | `MlDsaKeypair`; `address = sha256(public_key)` |
| `sdk/src/ml_dsa_instruction.rs` · `sdk/program/src/ml_dsa_program.rs` | Precompile verify + program id, registered in `precompiles.rs` |
| `sdk/src/ml_dsa_transaction.rs` | `0x00`-lead-byte wire tx format; synthetic 64-byte id = `sha256(sigs)‖sha256(msg)` |
| `PACKET_DATA_SIZE = 8192` | Raised from 1,232; ripple-fixes in `offchain_message.rs`, `shred*.rs`, `serve_repair.rs`, `rpc.rs` base58/base64 caps |
| `cost-model/src/block_cost_limits.rs` | `ML_DSA_VERIFY_COST = 5310 CU` (~2.3× ed25519, benchmarked) |
| `perf/src/sigverify.rs` · `core/src/banking_stage/…` · `rpc/src/rpc.rs` | CPU sigverify + banking bridge + `sendTransaction` preflight for `0x00` packets |
| `core/src/replay_stage.rs` · `voting_service.rs` · `gossip/src/cluster_info.rs` | ML-DSA vote construction + routing (`--ml-dsa-vote`) |
| `gossip/src/crds_value.rs` | `CrdsSignature::{Ed25519, MlDsa{pubkey, signature}}` enum |
| `ledger/src/shred.rs` · `shred/merkle.rs` · `sigverify_shreds.rs` | ML-DSA shred trailer + commitment + verify (`--ml-dsa-shred[-strict]`) |

Deeper writeups per phase live in [`CLAUDE.md`](CLAUDE.md); the strategy, the size
analysis, the two walls, and the roadmap live in
[`docs/ml-dsa-migration.md`](docs/ml-dsa-migration.md).

---

## Don't

- Don't run the bash scripts (`multinode-demo/*`, `programs/ml-dsa-tests/*.sh`) from a
  non-POSIX shell — they are bash-only (use bash on Linux/macOS, or a WSL shell on Windows).

**Windows / WSL hosts only:**

- Don't build or run from native Windows/PowerShell (symlinks are WSL-style; the validator
  is unsupported on native Windows).
- Don't use Windows git on this repo (it re-breaks the symlinks and churns line-endings).
- Don't keep ledgers on `/mnt/d` — use ext4 (`~/...`) for speed.

---

<details>
<summary><b>Appendix — upstream Solana build reference & legal disclaimer</b></summary>

The original Solana validator. For canonical build/test docs see
[Agave](https://github.com/anza-xyz/agave). The instructions below are the upstream
generic flow (this fork pins Rust to 1.76 and builds under WSL — see
[Environment](#environment--prerequisites) above).

### Building (upstream)

```bash
# 1. Install rustc, cargo and rustfmt.
curl https://sh.rustup.rs -sSf | sh
source $HOME/.cargo/env
rustup component add rustfmt
rustup update

# On Ubuntu, install build deps:
sudo apt-get update
sudo apt-get install libssl-dev libudev-dev pkg-config zlib1g-dev llvm clang cmake make libprotobuf-dev protobuf-compiler

# 3. Build.
./cargo build
```

### Testing (upstream)

```bash
./cargo test
```

### Benchmarking (upstream)

`cargo bench` needs the nightly toolchain:

```bash
rustup install nightly
cargo +nightly bench
```

### Disclaimer

All claims, content, designs, algorithms, estimates, roadmaps, specifications, and
performance measurements described in this project are done with the Solana Labs, Inc.
(“SL”) good faith efforts. It is up to the reader to check and validate their accuracy and
truthfulness. Furthermore, nothing in this project constitutes a solicitation for
investment.

Any content produced by SL or developer resources that SL provides are for educational and
inspirational purposes only. SL does not encourage, induce or sanction the deployment,
integration or use of any such applications (including the code comprising the Solana
blockchain protocol) in violation of applicable laws or regulations and hereby prohibits
any such deployment, integration or use. This includes the use of any such applications by
the reader (a) in violation of export control or sanctions laws of the United States or any
other applicable jurisdiction, (b) if the reader is located in or ordinarily resident in a
country or territory subject to comprehensive sanctions administered by the U.S. Office of
Foreign Assets Control (OFAC), or (c) if the reader is or is working on behalf of a
Specially Designated National (SDN) or a person subject to similar blocking or denied party
prohibitions.

</details>
