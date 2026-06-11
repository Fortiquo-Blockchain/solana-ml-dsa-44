# CLAUDE.md — infinia (Solana validator clone)

Clone of the official Solana validator monorepo (Rust/Cargo workspace; crates & binaries are still branded `solana`, version `2.0.0`). **Built and run via WSL2 Ubuntu — native Windows is not supported for the validator.**

## Workflow — what runs where  ⚠️ read first
This repo is edited on Windows but builds/runs in WSL. Keep the three roles separate:

| Role | Launch from | Runs in | For |
|---|---|---|---|
| **Claude Code** (this agent) | a **Windows** terminal (PowerShell): `cd D:\Work\infinia\solana-ml-dsa-44` → `claude` | Windows | driving work; runs builds/git via `wsl …` |
| **Cursor** (editor + rust-analyzer) | a **WSL** shell: `cursor .` | Remote-WSL | reading/editing code |
| **Build / run / git** | a **WSL** shell (or Cursor's terminal) | WSL | `cargo`, `solana-test-validator`, `git` |

Same files underneath (`/mnt/d` = `D:\`), so edits stay in sync.

**Rules:**
- **Run `claude` only from a Windows terminal** — Cursor's built-in terminal is a WSL shell with no Claude installed (`claude: command not found`).
- **Use WSL git for this repo, never Windows git.** Repo-local `core.symlinks=true` + `core.autocrlf=input` are set; Windows git would re-break the symlinks (shows them "type changed") and churn line-endings. When this agent runs git here, it routes through `wsl` (e.g. `wsl -e bash -lc 'cd /mnt/d/Work/infinia/solana-ml-dsa-44 && git …'`).
- **Always open Cursor via Remote-WSL** (`cursor .` from WSL), never as a plain Windows folder — Windows can't read the 26 WSL-native symlinks (`*/build.rs`, a few `.sh`, `sdk/package.json`), so opening those files errors. `cargo` in WSL reads them fine; to read one from Windows use `wsl cat <file>`.

## Environment
- **Build/run host: WSL2 Ubuntu** (default user `gizmoclardin`). Do not build from native Windows/PowerShell.
- **Rust toolchain: pinned to 1.76.0** by `rust-toolchain.toml` (rustup honors it automatically inside the repo).
- Repo path from WSL: `/mnt/d/Work/infinia/solana-ml-dsa-44`.
- Build deps (already installed): `libssl-dev libudev-dev pkg-config zlib1g-dev llvm clang cmake protobuf-compiler libprotobuf-dev` (plus gcc/make).

## ⚠️ First-build fix: restore Windows-corrupted symlinks
This repo has **26 git symlinks** (mode `120000`). A Windows checkout (`core.symlinks=false`) turns them into plain text files, so `cargo build` fails immediately with:
`error: expected item, found `..`  --> frozen-abi/macro/build.rs:1:1`.
If you hit this (e.g. after a fresh clone on Windows), restore them from WSL:
```bash
cd /mnt/d/Work/infinia/solana-ml-dsa-44
git config core.symlinks true
git ls-files -s | grep '^120000' | awk '{print $4}' | while read f; do ln -sfn "$(cat "$f")" "$f"; done
```
Already applied in this checkout. Restoring symlinks made WSL `git status` show ~2042 files "modified" — that was **only CRLF line-endings** (`git diff --ignore-all-space` showed none). That noise is now silenced by `git config core.autocrlf input` (also already set), so WSL `git status` is clean apart from intentional changes.

## Build
```bash
cd /mnt/d/Work/infinia/solana-ml-dsa-44
cargo build --release --bin solana-test-validator --bin solana --bin solana-keygen
```
Binaries land in `target/release/`. First build ≈ **20 min**; incremental builds are fast.

## Run the local test validator
```bash
cd /mnt/d/Work/infinia/solana-ml-dsa-44
export PATH="$PWD/target/release:$PATH"
solana-test-validator --ledger ~/solana-test-ledger      # add --reset for a fresh genesis
```
- JSON-RPC: `http://127.0.0.1:8899`  •  WebSocket: `ws://127.0.0.1:8900`
- Ledger is kept on ext4 (`~/solana-test-ledger`) for speed — **not** on `/mnt/d` (slow drvfs I/O).
- Stop with **Ctrl-C**.

## Interact (second terminal)
```bash
export PATH="/mnt/d/Work/infinia/solana-ml-dsa-44/target/release:$PATH"
solana config set --url http://127.0.0.1:8899
solana-keygen new --no-bip39-passphrase     # once, if you have no wallet
solana airdrop 10
solana balance                              # 10 SOL
solana slot                                 # increases across calls => chain is producing
solana logs                                 # live transaction logs
```
Tip: add the `export PATH=...` line to `~/.bashrc` so the CLIs are always available.

## Rust in Cursor (rust-analyzer)
Open the repo via **Remote-WSL** so rust-analyzer uses the WSL toolchain (the symlinks and artifacts only work there):
1. From a WSL shell in the repo: `cursor .`  (or Cursor → Command Palette → "Reopen Folder in WSL").
2. Install the **rust-analyzer** extension *in WSL* (the Extensions panel offers "Install in WSL: Ubuntu").
3. `rust-src` is installed and `.vscode/settings.json` tunes rust-analyzer for this large workspace (separate target dir so it won't invalidate your `--release` build). First index takes a few minutes.

## Post-quantum signatures (ML-DSA-44) — fork goal
This fork is migrating Solana's signatures from Ed25519 to **ML-DSA-44** (NIST FIPS 204, post-quantum). The full strategy, the five signature surfaces, and the phased roadmap live in **`docs/ml-dsa-migration.md`**.

**Phase 0 — done & verified:** an on-chain **ML-DSA-44 precompile** (the post-quantum analogue of the ed25519 precompile) that lets transactions verify ML-DSA-44 signatures. Phase 0 itself changed nothing about *how* things are signed — see **Phase 1** below, which replaces transaction signing for user payments.

What it touches:
- **Crate:** `fips204 = "0.4.6"` (pure Rust, final FIPS 204, MSRV 1.70 → builds on our pinned 1.76). Always signs/verifies with an **empty context** to stay byte-compatible with the sibling JS sample (`../ml-dsa-44/`, `@noble/post-quantum`).
- **New files:** `sdk/src/ml_dsa_instruction.rs` (verify + `new_ml_dsa_instruction`, mirrors `ed25519_instruction.rs`; pubkey 1312 B, sig 2420 B) and `sdk/program/src/ml_dsa_program.rs` (program id `6CjqvizoLwkFdkutVsjsXjttQiNVfrZM18MdZw2VbNSs`). Registered in `sdk/src/precompiles.rs` with `feature: None`, so the bank auto-creates its account at `finish_init`.
- **⚠️ Packet ceiling raised:** `PACKET_DATA_SIZE` is **8192** (was `1280-40-8 = 1232`) so one ~3.9 KB ML-DSA transaction fits in a packet. This deliberately breaks live-cluster wire compatibility (self-hosted only). Ripple fixes that must move together if this value changes again: the derived `const_assert`s in `sdk/src/offchain_message.rs`, the shred-size asserts in `ledger/src/shred*.rs` + `core/src/repair/serve_repair.rs`, and the base58/base64 "golden" caps in `rpc/src/rpc.rs` (`MAX_BASE58_SIZE`/`MAX_BASE64_SIZE`).
- **Cost model prices the precompile** (EPIC 3-2): an ML-DSA precompile-verify instruction is now charged compute units, mirroring ed25519/secp256k1. `ML_DSA_VERIFY_COST = COMPUTE_UNIT_TO_US_RATIO * 177 = 5310 CU` in `cost-model/src/block_cost_limits.rs` (benchmarked at ~2.3× ed25519 — median of repeated same-host runs — via `programs/ml-dsa-tests/examples/bench_verify.rs`). `get_signature_details` counts the `ml_dsa_program` id (`sdk/program/src/message/sanitized.rs`), `cost_model.rs` multiplies the count by the constant, and the program id is registered `=> 0` in `BUILT_IN_INSTRUCTION_COSTS` so its verify cost flows only through the signature path. It also counts toward `total_signatures()` (the per-signature fee), exactly like the other precompiles.

Test it:
```bash
cargo test -p solana-sdk --features full ml_dsa     # unit: valid passes, tampered fails
cargo test -p solana-ml-dsa-program-tests           # integration via solana-program-test
# live, end-to-end over RPC (start the validator first, then):
cargo run --release -p solana-ml-dsa-program-tests --example submit_live
```
The live demo airdrops an ordinary Ed25519 fee payer, then submits a valid ML-DSA precompile tx (confirms) and a tampered one (rejected).

## Post-quantum transaction signing (Phase 1) — done & verified
Building on Phase 0, **user-payment transactions can now be signed with ML-DSA-44 instead of Ed25519** — the fee payer's signature *and* address are post-quantum. This **coexists** with the Ed25519 path (a `0x00` lead byte marks an ML-DSA transaction; everything else stays Ed25519), so the validator's own votes/gossip/shreds keep running and the node keeps producing blocks. It is **not** an exclusive cutover — that would require Phases 2–3.

- **Address:** `address = sha256(ml_dsa_public_key)` (32 bytes ⇒ accounts-db / PDAs / base58 unchanged). `MlDsaKeypair` + helper in `sdk/src/ml_dsa_keypair.rs`.
- **New tx format** (`sdk/src/ml_dsa_transaction.rs`): `[0x00][sig count][2420-B ML-DSA sigs][pubkey count][1312-B signer pubkeys][bincode(Message)]`. The 64-byte tx **id** is a synthetic `sha256(sigs)‖sha256(msg)` (the runtime keys on a 64-byte `Signature`; a 2420-B sig can't be one). Verification = ML-DSA sig **and** `sha256(pubkey)==account_keys[i]`.
- **Validator intercepts:** `perf/src/sigverify.rs` CPU-verifies `0x00` packets; `core/src/banking_stage/immutable_deserialized_packet.rs` bridges them for execution; `rpc/src/rpc.rs` `sendTransaction` runs the ML-DSA preflight and forwards the raw bytes. Ed25519/GPU paths untouched.
- **Scope:** one required signer (fee payer), legacy `Message` only (multi-sig / v0 / lookup-tables rejected at decode).
- **Caveats (Phase-1 PoC):** ML-DSA verification runs on the **CPU sigverify path** — if perf-libs/GPU is loaded, `0x00` packets are *dropped* (fail-closed) on the GPU path, so run with GPU sigverify disabled (the default for `solana-test-validator`). The RPC preflight for ML-DSA is **signature-only** (no `simulate`/health check), so a stale-blockhash/underfunded-but-signed tx is caught later at sigverify/banking, not at `sendTransaction`.

Test it:
```bash
cargo test -p solana-sdk ml_dsa_transaction --lib            # sign/verify round-trip
cargo test -p solana-perf sigverify                          # 0x00 routing + Ed25519 regression
cargo test -p solana-runtime test_ml_dsa_transfer_executes   # bank execution
bash programs/ml-dsa-tests/demo-transfer.sh                  # live: an ML-DSA transfer confirms, a forged one is rejected
```

## Post-quantum validator votes (Phase 2) — done & verified
Building on Phase 1, **the validator's own consensus votes can now be signed with ML-DSA-44 instead of Ed25519** — the third signing surface (and the first *consensus* one). It is **flag-gated and coexists**: default **off** = byte-for-byte unchanged Ed25519 voting (so the single-node validator never stalls); **on** = post-quantum votes. The node identity (gossip/shreds) stays Ed25519.

- **Single-signer fit:** a vote needs exactly one signer when the ML-DSA voter is *both* the fee payer *and* the authorized voter (`to_vote_instruction` makes the authorized voter the signer, not the vote account) ⇒ `num_required_signatures == 1`, which is exactly what `MlDsaTransaction::sign` accepts. No multi-signer extension needed.
- **Vote construction** (`core/src/replay_stage.rs`, `generate_vote_tx`): when `ml_dsa_voter` is set and it equals the vote account's authorized voter, build the vote instruction with the ML-DSA address as payer+authority, wrap it as a Phase 1 `0x00` `MlDsaTransaction`, and return a new `GenerateVoteTxResult::MlDsaTx(wire, blockhash)`. Tower save / `vote_signatures` (synthetic ids) are unchanged. Emits a per-vote `info!("Signed ML-DSA-44 … vote …")`.
- **Routing** (`core/src/voting_service.rs` + `gossip/src/cluster_info.rs`): new `VoteOp::{Push,Refresh}MlDsaVote{wire,…}` submit the raw bytes via `cluster_info.send_transaction_raw(&wire, None)` to the node's **own regular TPU** — **never** the vote-only port (Phase 1 sigverify rejects `0x00` there) nor gossip CRDS (typed to Ed25519 `Transaction`). From there it rides the Phase 1 regular-path sigverify → banking `new_ml_dsa` bridge → the vote instruction executes and updates the vote account.
- **Plumbing:** `ml_dsa_voter: Option<Arc<MlDsaKeypair>>` is carried on `ValidatorConfig` → `Tvu::new` → `ReplayStageConfig` (`None` everywhere else; `safe_clone_config` updated). `solana-test-validator` gets a **`--ml-dsa-vote <KEYFILE>`** flag (`validator/src/cli.rs` + `validator/src/bin/solana-test-validator.rs`) that loads or mints the keypair.
- **Genesis** (`test-validator/src/lib.rs`, new `solana-vote-program` dep): when `ml_dsa_voter` is set, the genesis vote account's `authorized_voter` is repointed to the ML-DSA address (node_pubkey stays the identity — `replay_stage` asserts that), the address is funded, and **`tpu_enable_udp` is forced on** (the regular TPU only ingests UDP when enabled; default is QUIC-only, which silently drops the raw-UDP vote packets).
- **⚠️ Caveats (Phase-2 PoC):** ML-DSA votes ride the **regular TPU over UDP** and do **not** propagate via gossip CRDS — fine single-node; a multi-node cluster would only observe them via block replay. Each vote pays a normal ~5000-lamport fee (genesis funds the address with 1M SOL); stock votes are feeless. Same Phase-1 CPU-sigverify caveat (GPU path drops `0x00`). One `authorized_voter` per epoch ⇒ no live Ed25519↔ML-DSA hot-swap; switching is a flag-gated restart.

Test it:
```bash
cargo build --release --bin solana-test-validator            # picks up the --ml-dsa-vote flag
bash programs/ml-dsa-tests/demo-vote.sh                       # live: PQ vote authority, chain confirms+finalizes on ML-DSA votes
# baseline regression: start solana-test-validator WITHOUT the flag => Ed25519 voting finalizes, zero ML-DSA activity
```
Verified live: with `--ml-dsa-vote`, the vote account's authorized voter is the post-quantum address and processed/confirmed/finalized slots + the vote account's `lastVote`/root all advance (the chain roots on ML-DSA votes); without the flag, Ed25519 voting is unchanged.

## Running the demos — combined script vs. two terminals
All three live demos (`programs/ml-dsa-tests/demo.sh` Phase 0, `demo-transfer.sh` Phase 1, `demo-vote.sh` Phase 2) print the full keys/signatures and the **raw JSON-RPC request + response at every chain interaction** (so nothing is faked). Run any of them **two ways** from a WSL shell (bash only):

**A) One combined script** — boots a throwaway validator, runs the demo, tears it down:
```bash
bash programs/ml-dsa-tests/demo.sh           # Phase 0 — precompile verifies a PQ signature
bash programs/ml-dsa-tests/demo-transfer.sh  # Phase 1 — a PQ-signed SOL transfer
bash programs/ml-dsa-tests/demo-vote.sh      # Phase 2 — the validator's own votes are PQ
```
Pass `--build` after editing Rust to force cargo (otherwise it skips cargo when the binaries exist; only changed crates recompile). `--build` on the Phase 1/2 scripts rebuilds `solana-test-validator` → recompiles `solana-core`; to iterate on just an example, run `cargo build --release --example <name>` and then the script **without** `--build`.

**B) Two terminals** — a long-running chain you re-run tests against.
Phase 0 / Phase 1 (plain validator + the example binary):
```bash
# Terminal 1 (the chain) — leave running:
export PATH="$PWD/target/release:$PATH"
solana-test-validator --reset --ledger ~/solana-test-ledger
# Terminal 2 (the test) — re-runnable; each run mints fresh wallets + airdrops:
./target/release/examples/submit_live        # Phase 0
./target/release/examples/ml_dsa_transfer    # Phase 1
```
Phase 2 (the votes happen *inside* the validator, so the flag is on the chain; observe from the other terminal):
```bash
# Terminal 1 (the chain, voting post-quantum) — leave running:
solana-test-validator --reset --ledger ~/solana-test-ledger-mldsa-vote --ml-dsa-vote /tmp/mldsa-vote.bin
# Terminal 2 (observe) — slots + the vote account's lastVote keep climbing:
solana slot --commitment finalized
curl -s http://127.0.0.1:8899 -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getVoteAccounts","params":[{"commitment":"processed"}]}'
```
Keep ledgers on ext4 (`~/...`), not `/mnt/d`, for speed. Phase 0/1 need only a plain validator; `--ml-dsa-vote` is Phase 2 only. The examples connect to `127.0.0.1:8899`.

## Don't
- Don't build from native Windows/PowerShell (symlinks are WSL-style; the validator is unsupported on Windows).
- Don't run the bash scripts (`multinode-demo/*`, `scripts/cargo-install-all.sh`) from PowerShell — they are bash-only.
