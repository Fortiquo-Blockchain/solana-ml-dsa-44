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
This fork is migrating Solana's signatures from Ed25519 to **ML-DSA-44** (NIST FIPS 204, post-quantum). The full strategy, the five signature surfaces, and the phased roadmap live in **`docs/ml-dsa-migration.md`**. What is still **pending** to take the chain further lives in **`docs/ml-dsa-remaining-work.md`** — a living doc: when an item lands, prune it from that doc rather than annotating it done.

**Phase 0 — done & verified:** an on-chain **ML-DSA-44 precompile** (the post-quantum analogue of the ed25519 precompile) that lets transactions verify ML-DSA-44 signatures. Phase 0 itself changed nothing about *how* things are signed — see **Phase 1** below, which replaces transaction signing for user payments.

What it touches:
- **Crate:** `fips204 = "0.4.6"` (pure Rust, final FIPS 204, MSRV 1.70 → builds on our pinned 1.76). Always signs/verifies with an **empty context** to stay byte-compatible with the sibling JS sample (`../ml-dsa-44/`, `@noble/post-quantum`).
- **Conformance suite (NIST gold + cross-impl):** `programs/ml-dsa-tests/tests/fips204_vectors.rs` pins our `MlDsaKeypair::from_seed` keygen and the `fips204` sign/verify core to the **official NIST ACVP FIPS 204 vectors** (vendored inside `fips204` 0.4.6's `.crate`; excerpt committed at `tests/vectors/ml_dsa_44_kat.json`, regenerate with `cross-impl/gen_fixture.py`). The JS half `cross-impl/check_vectors.mjs` proves `@noble/post-quantum` produces byte-identical keys/signatures. Signing is **hedged** (randomized) so byte-KATs use the deterministic path (`_internal_sign` rnd=0 / `{extraEntropy:false}`); keygen-from-seed is deterministic. Run: `cargo test -p solana-ml-dsa-program-tests --test fips204_vectors` (WSL) + `node programs/ml-dsa-tests/cross-impl/check_vectors.mjs` (Windows; no Node in WSL). Closes the `docs/ml-dsa-migration.md` §8 reference-mismatch risk.
- **New files:** `sdk/src/ml_dsa_instruction.rs` (verify + `new_ml_dsa_instruction`, mirrors `ed25519_instruction.rs`; pubkey 1312 B, sig 2420 B) and `sdk/program/src/ml_dsa_program.rs` (program id `6CjqvizoLwkFdkutVsjsXjttQiNVfrZM18MdZw2VbNSs`). Registered in `sdk/src/precompiles.rs` with `feature: None`, so the bank auto-creates its account at `finish_init`.
- **⚠️ Packet ceiling raised:** `PACKET_DATA_SIZE` is **8192** (was `1280-40-8 = 1232`) so one ~3.9 KB ML-DSA transaction fits in a packet. This deliberately breaks live-cluster wire compatibility (self-hosted only). Ripple fixes that must move together if this value changes again: the derived `const_assert`s in `sdk/src/offchain_message.rs`, the shred-size asserts in `ledger/src/shred*.rs` + `core/src/repair/serve_repair.rs`, and the base58/base64 "golden" caps in `rpc/src/rpc.rs` (`MAX_BASE58_SIZE`/`MAX_BASE64_SIZE`).
- **Cost model prices the precompile** (EPIC 3-2): an ML-DSA precompile-verify instruction is now charged compute units, mirroring ed25519/secp256k1. `ML_DSA_VERIFY_COST = COMPUTE_UNIT_TO_US_RATIO * 177 = 5310 CU` in `cost-model/src/block_cost_limits.rs` (benchmarked at ~2.3× ed25519 — median of repeated same-host runs — via `programs/ml-dsa-tests/examples/bench_verify.rs`). `get_signature_details` counts the `ml_dsa_program` id (`sdk/program/src/message/sanitized.rs`), `cost_model.rs` multiplies the count by the constant, and the program id is registered `=> 0` in `BUILT_IN_INSTRUCTION_COSTS` so its verify cost flows only through the signature path. It also counts toward `total_signatures()` (the per-signature fee), exactly like the other precompiles.

Test it:
```bash
cargo test -p solana-sdk --features full ml_dsa     # unit: valid passes, tampered fails
cargo test -p solana-ml-dsa-program-tests           # integration via solana-program-test
cargo test -p solana-ml-dsa-program-tests --test fips204_vectors  # NIST FIPS 204 KAT + @noble cross-impl
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
- **Caveats (Phase-1 PoC):** ML-DSA verification runs on the **CPU sigverify path** — if perf-libs/GPU is loaded, `0x00` packets are *dropped* (fail-closed) on the GPU path, so run with GPU sigverify disabled (the default for `solana-test-validator`). The RPC preflight for ML-DSA now runs the same **health check + `simulate_transaction`** as the Ed25519 path (the ML-DSA signature + address-binding check stands in for `verify_transaction`, since the synthetic 64-byte id is not an Ed25519 signature), so a stale-blockhash/underfunded tx is rejected at `sendTransaction` — parity with Ed25519. `rpc/src/rpc.rs` `test_rpc_send_transaction_preflight` covers it.

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

## Post-quantum node-to-node gossip (Phase 2b core) — done & verified
Building on the SDK ML-DSA types, **gossip CRDS values can now carry a post-quantum ML-DSA-44 signature** — the node-to-node chatter surface (the fourth signing surface touched). The core wire/crypto change and a live cross-node verification are done; making a node sign its *own* gossip identity with ML-DSA is **deferred to Phase 3** (it needs the node identity to equal the ML-DSA address, which is entangled with shred signing). **Coexists**: Ed25519 is the default and byte-for-byte unchanged, so the node never stalls.

- **Signature enum** (`gossip/src/crds_value.rs`): `CrdsValue.signature` changed from the fixed 64-byte `Signature` to a `CrdsSignature` enum — `Ed25519(Signature)` or `MlDsa { pubkey: Box<MlDsaPublicKey>, signature: Box<MlDsaSignature> }`. The ML-DSA variant carries the 1312-byte public key so any peer verifies with no side channel (mirrors the Phase 1 transaction wire format). Boxed so `CrdsValue` stays pointer-sized.
- **Verification** (`CrdsSignature::verify(identity, message)`): the ML-DSA arm enforces the signature **and** `ml_dsa_address(pubkey) == identity` (i.e. `sha256(public_key)` equals the value's 32-byte CRDS identity), so a forged key can't impersonate an identity — the same binding as Phase 1. `CrdsValue`'s `Signable::verify` delegates to it; the shared `Signable` trait and `Ping`/`Pong`/`PruneData` (still Ed25519) are untouched.
- **Signing** (`CrdsValue::new_signed_ml_dsa(data, &MlDsaKeypair)`): fallible — errors if signing fails or the keypair's address ≠ the data identity, so a misconfigured value is rejected at the producer rather than silently dropped cluster-wide. Ed25519 `new_signed`/`new_unsigned` are unchanged (`new_unsigned` defaults to `CrdsSignature::Ed25519`).
- **Live relay** (`ClusterInfo::push_signed_crds_value`): lets a node gossip a pre-signed (e.g. ML-DSA) CRDS value over real gossip; refuses+logs an unverifiable value. The 2-node test `gossip/tests/gossip.rs::ml_dsa_crds_value_propagates_between_live_nodes` proves an ML-DSA-signed value propagates A→B and lands in B's table **only because** B's sigverify pass validated the post-quantum signature.
- **frozen-abi:** `CrdsValue`/`Protocol` digests change; `MlDsaSignature`/`MlDsaPublicKey` got manual specialization-gated `AbiExample` impls. The `Protocol` digest must be regenerated under a nightly/specialization build (`SOLANA_ABI_BULK_UPDATE=1 cargo test`); it is **inert on the pinned stable 1.76 toolchain**, so it does not block our builds/tests.
- **Sizing:** an ML-DSA `CrdsValue` is ~3.7 KB (sig 2420 + pubkey 1312 + data), which fits one gossip packet (`PUSH_MESSAGE_MAX_PAYLOAD_SIZE = PACKET_DATA_SIZE - 44 = 8148`), so no packet-constant change — but packing drops to ~3 PQ values/packet and ML-DSA verify is heavier on the `par_verify` path.
- **⚠️ Caveats (Phase-2b core):** only the *transport + verify* path is proven live; a node does **not** yet sign its own gossip identity with ML-DSA (deferred to Phase 3). Two-node general CRDS propagation is finicky (ContactInfo propagates; stake-weighted votes don't at N=2), so the demo uses a ContactInfo value and a bidirectional `insert_info`. Pre-existing: 3 `crds_gossip_pull` bloom-filter unit tests fail on this branch independent of this work (a consequence of the fork's `PACKET_DATA_SIZE=8192`).

Test it:
```bash
cargo test -p solana-gossip --test gossip crds_value   # CrdsSignature verify/binding/round-trip + Ed25519 regression
cargo test -p solana-gossip --test gossip ml_dsa_crds_value_propagates_between_live_nodes  # live 2-node propagation + verify
```

## Post-quantum block broadcasting (Phase 3) — done & verified
Building on the SDK ML-DSA types, **a leader's block shreds can now carry a post-quantum ML-DSA-44 signature** — the fifth and final signing surface (block broadcasting / turbine). **Additive & flag-gated**: default off = byte-for-byte unchanged Ed25519 shreds (the node never stalls); on (`--ml-dsa-shred`) = each FEC-set Merkle root is *also* signed with ML-DSA-44, verified by upgraded peers. The node identity stays Ed25519 (flip deferred — see caveats).

- **Why additive, not a cutover:** a 2420-byte ML-DSA signature dwarfs a shred's 64-byte signature field, and turbine fans *individual* shreds to *different* peers, so each shred must verify standalone. The existing Merkle scheme already signs **once per FEC set** over a 32-byte Merkle root and copies that signature into all ~67 shreds; Phase 3 keeps that model and makes the *carried* signature ML-DSA-sized. Ed25519 stays the load-bearing liveness signature; ML-DSA is an additional attestation.
- **Wire format** (`ledger/src/shred.rs`, `ledger/src/shred/merkle.rs`): a new `ml_dsa: bool` on `ShredVariant::{MerkleData,MerkleCode}` (high nibbles `0xC0/0xD0` data, `0xE0/0xF0` code; the slot's final resigned set adds `0x30` data / `0x20` code — see Hardening). An ml_dsa shred reserves, out of data *capacity* (packet size unchanged at 8192), a **32-byte commitment** = `sha256(leader ML-DSA pubkey)` inside the Ed25519-signed Merkle region (right before the proof), plus a **3732-byte trailer** = `[ML-DSA pubkey 1312 ‖ signature 2420]` after the proof (excluded from the signed node). Cost: ~47% data-capacity drop ⇒ ~2× shred count when on.
- **Signing** (`merkle.rs` `make_erasure_batch`): before the Merkle tree is built, the commitment is written into every shred; the Ed25519 root signature is unchanged; then the root is signed **once** with ML-DSA-44 and the `[pubkey‖sig]` trailer is attached to every shred. The slot's final (`resigned`) FEC set is ml_dsa-signed too (`ml_dsa = keypair.is_some()`); for a resigned shred the dormant 64-byte retransmitter slot sits *after* the trailer at the payload end, so the capacity math reserves both. The keypair threads `Shredder::entries_to_shreds(…, ml_dsa_keypair: Option<&MlDsaKeypair>, …)` → `make_merkle_shreds_from_entries` → `make_shreds_from_data` → `make_erasure_batch`.
- **Verification** (`ledger/src/sigverify_shreds.rs`): `verify_shred_ml_dsa_cpu` does a three-step check — (a) the Ed25519 signature authenticates the Merkle root against the slot leader, (b) `ml_dsa_address(trailer_pubkey) == the in-root commitment` (binds the trailer key to the leader — this stops an attacker swapping the unsigned trailer), and (c) the ML-DSA-44 signature over the root verifies. `audit_ml_dsa_shreds` runs this as a **non-gating** pass at turbine ingress (`turbine/src/sigverify_shreds.rs`, after the Ed25519 `mark_disabled`), emitting `ml_dsa_shred_verify_ok/_fail` metrics without ever discarding a shred. The opt-in **`enforce_ml_dsa_shreds`** (under `--ml-dsa-shred-strict`) is the gating sibling — it additionally `set_discard`s any failing ml_dsa shred (non-ml_dsa and already-discarded packets untouched) and emits `ml_dsa_shred_dropped`. The GPU path is untouched (ml_dsa shreds keep a valid Ed25519 sig at bytes 0..64).
- **Flags** — **`--ml-dsa-shred <KEYFILE>`**: `ValidatorConfig.ml_dsa_shred: Option<Arc<MlDsaKeypair>>` threads through `Tpu::new` → `new_broadcast_stage` → `StandardBroadcastRun` (a field) into both `entries_to_shreds` calls. **`--ml-dsa-shred-strict`** (bool, requires `--ml-dsa-shred`): `ValidatorConfig.ml_dsa_shred_strict` → `TvuConfig` → `spawn_shred_sigverify` → `verify_packets`, which branches `enforce` vs `audit`. `solana-test-validator` loads or mints the keypair and reads the strict flag; `TestValidatorGenesis::ml_dsa_shred(...)` / `ml_dsa_shred_strict(...)` are the builders. Mirrors the Phase 2 `--ml-dsa-vote` plumbing, routed to broadcast/turbine.
- **⚠️ Caveats (Phase-3 PoC):** node identity stays Ed25519 — the QUIC/TLS handshake authenticates a node by an **Ed25519-keyed X.509 cert** (`streamer/src/tls_certificates.rs`); an ML-DSA address has no Ed25519 secret, so flipping `id()` would partition the node from turbine/repair (deferred, and still what blocks a node's own ML-DSA *gossip* identity). ml_dsa verification is **advisory by default** (telemetry, not a liveness gate); `--ml-dsa-shred-strict` makes it gating (drops failing ml_dsa shreds). A single node's *own* shreds bypass turbine sigverify (that path verifies peer shreds), so on one node the audit counter does not climb — verify a produced shred offline instead (the demo does this). Same Phase-1 CPU-sigverify family; ~2× shred volume when on.

### Phase 3 hardening (2026-06) — done & verified
Three follow-ups closed the v1 rough edges; each is a separate commit on `feat/ml-dsa-44`, reviewed via `pr-review-toolkit:review-pr`:
1. **RS recovery for ml_dsa sets** (`merkle.rs` `recover`/`from_recovered_shard`): erasure recovery now reads the shared 32-byte commitment from a surviving shred and restores it onto recovered shreds — exactly like the chained Merkle root — so an incomplete ml_dsa FEC set **recovers cleanly** (previously it failed RS-recovery and fell back to repair). Recovered shreds still lack the (non-erasure-coded) trailer, so they pass Ed25519/liveness; the advisory audit only runs on received turbine shreds, never on locally-recovered ones.
2. **ml_dsa-sign the resigned final set** (`shred.rs` codec + `merkle.rs` gating): the slot's final (resigned) FEC set is now also ML-DSA-signed (new variant bytes `0x20` code / `0x30` data; `ml_dsa = keypair.is_some()`). No layout/offset math changed — capacity already reserved both the dormant 64-byte retransmitter slot and the ml_dsa overhead; the trailer stays right after the proof, the dormant slot at the payload end. resigned-without-chained stays invalid.
3. **`--ml-dsa-shred-strict`** (`enforce_ml_dsa_shreds` + turbine/config plumbing): opt-in gating verify that drops failing ml_dsa shreds (default off = unchanged advisory).

**Pre-existing fork test failures (not from this work):** on `cargo test -p solana-ledger --lib shred` the baseline (fork `PACKET_DATA_SIZE=8192`) is **10 red** — `shred::legacy::test_sanitize_data_shred`, `shred::tests::test_serde_compat_shred_{code,data,data_empty}`, four `blockstore::tests::*` byte-layout tests, and two `shredder::tests::test_shred_fec_set_index::*`. All hardcode the old 1232 packet size; confirmed identical red set with the hardening changes stashed (the hardening adds only passing tests). Plus the 3 `crds_gossip_pull` bloom tests noted elsewhere.

Test it:
```bash
cargo test -p solana-ledger --lib shred::merkle      # ml_dsa signing round-trip (chained+unchained, multi-FEC), resigned set is ml_dsa-signed, partial recovery succeeds (data/coding x chained)
cargo test -p solana-ledger --lib sigverify_shreds   # verify pass: honest, wrong leader, tampered sig, forged trailer (commitment binding), resigned shred, strict-drop vs advisory-keep + Ed25519 regression
bash programs/ml-dsa-tests/demo-shred.sh             # live: a --ml-dsa-shred validator produces blocks; a produced shred's ML-DSA-44 trailer verifies offline
```

## Running the demos — combined script vs. two terminals
The live demos (`programs/ml-dsa-tests/demo.sh` Phase 0, `demo-transfer.sh` Phase 1, `demo-vote.sh` Phase 2, `demo-shred.sh` Phase 3) print the full keys/signatures and the **raw JSON-RPC request + response at every chain interaction** (so nothing is faked). Run any of them **two ways** from a WSL shell (bash only):

**A) One combined script** — boots a throwaway validator, runs the demo, tears it down:
```bash
bash programs/ml-dsa-tests/demo.sh           # Phase 0 — precompile verifies a PQ signature
bash programs/ml-dsa-tests/demo-transfer.sh  # Phase 1 — a PQ-signed SOL transfer
bash programs/ml-dsa-tests/demo-vote.sh      # Phase 2 — the validator's own votes are PQ
bash programs/ml-dsa-tests/demo-shred.sh     # Phase 3 — the validator's own block shreds are PQ-signed
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
