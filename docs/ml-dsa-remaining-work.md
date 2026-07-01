# ML-DSA-44 — Remaining Work & Go-Live Guide

> **Audience:** the next developer picking up the Ed25519 → ML-DSA-44 (post-quantum) migration
> on this fork. This is the *continuation* doc — what is done, what is only half-done, what was
> added but never tested at scale, and how to keep going.
>
> **⚠️ This is a living, ephemeral doc.** As work lands, **delete** the completed item rather than
> striking it through or marking it "✅ done." This doc should always describe only what is *still*
> pending — its whole value is that everything in it is true *right now*. Completion history belongs
> in git and the commit log, not here. (The same goes for the status matrix in §1 and the footer
> stamp: update them, don't annotate them.)
>
> **Companion docs (read these for context, not repeated here):**
> - `docs/ml-dsa-migration.md` — the *why* and *strategy* (the two walls, the five surfaces, the risk table).
> - `CLAUDE.md` — the *per-phase implementation notes* (every phase has a "done & verified" block with the exact files, flags, and caveats).
> - `README.md` — the one-page summary.
>
> This doc assumes you have read neither and is self-contained, but it links to both rather than
> duplicating their strategy.

---

## 0. What "go live" means here (read this first)

**It does not mean joining the live public Solana network.** That is impossible **by design**, and
nothing in the remaining work changes it. To fit one ~3.9 KB ML-DSA transaction into a packet, the
fork raises `PACKET_DATA_SIZE` from `1232` to `8192` (`sdk/src/packet.rs:24`). Every stock Solana
node enforces the old limit and will drop our packets. We cannot change that constant on other
people's machines. **This fork is a self-hosted network only.**

So in this doc, **"go live"** = taking the post-quantum chain from *"all five surfaces coded and
demoed on a single validator"* (where it is today) to *"a production-grade, multi-node, self-hosted
post-quantum network."* That is the gap this doc catalogs.

The migration touches **five signing surfaces** (see `docs/ml-dsa-migration.md` §2). All five are
coded. The remaining work is about **hardening, multi-node, and the one surface that is only
partially done (node identity).**

---

## 1. Current state at a glance

| Phase | Surface | Flag / entry point | Status | Test level |
|------:|---------|--------------------|--------|------------|
| **0** | App-level verify (precompile) | program id `6Cjqvizo…VbNSs`, `feature: None` | ✅ Done | unit + integration + FIPS-204 KAT + live RPC |
| **1** | User payments | `0x00` tx marker (`ML_DSA_TX_MARKER`) | ✅ Done | unit + bank-exec + live RPC |
| **2a** | Validator votes | `--ml-dsa-vote <KEYFILE>` | ✅ Done | unit + **single-node** live (finalizes) |
| **2b** | Node-to-node gossip (CRDS) | `CrdsSignature` enum | ✅ **Core** done | unit + **2-node** live wire |
| **3** | Block broadcasting (shreds/turbine) | `--ml-dsa-shred[-strict]` | ✅ Done (additive) | unit + **single-node** produce + offline verify |
| **4** | **Node identity → ML-DSA** | — | ❌ **Not started** (blocked) | — |

**One-line takeaway:** every signing surface has been implemented and demonstrated on **one**
validator. Nothing here has been exercised on a real multi-node cluster, and the node's own
*network identity* (QUIC/TLS, gossip ContactInfo, repair) is still Ed25519. That is the frontier.

Everything **coexists** with Ed25519 and is flag-gated or marker-tagged, so a stock single-node
validator never stalls — the post-quantum paths are additive.

---

## 2. Terms used below

Full glossary in `docs/ml-dsa-migration.md`. Only the terms specific to the *pending* discussion:

| Term | Meaning |
|------|---------|
| **`0x00` marker** | Lead byte tagging a Phase-1 ML-DSA transaction; a stock tx starts with a signature count ≥ 1. |
| **Commitment** (P3) | `sha256(leader ML-DSA pubkey)`, 32 B, inside the Ed25519-signed Merkle region of a shred. |
| **Trailer** (P3) | `[pubkey 1312 ‖ sig 2420]` = 3732 B after the Merkle proof (not Ed25519-signed, not erasure-coded). |
| **Advisory vs strict** | Phase-3 shred verify is *advisory* (telemetry) by default; `--ml-dsa-shred-strict` makes it *gating*. |

---

## 3. Blockers to going live (summary)

This is the load-bearing section. Each blocker is tagged **Fixable → §7.x** (there is a path, see the
continuation roadmap) or **By-design** (will not and should not be fixed on this fork).

| # | Blocker | Impact | Verdict |
|---|---------|--------|---------|
| B1 | **Node identity is still Ed25519** (QUIC/TLS X.509 cert is Ed25519-keyed). Can't flip `id()` to the ML-DSA address without partitioning from turbine/repair/TPU. | The node authenticates itself to peers with Ed25519. The "PQ validator" is PQ in its *payloads*, not its *identity*. | **Fixable → §7.1** (large: TLS + ping/pong/prune) |
| B2 | **No GPU/CUDA path for ML-DSA.** Any batch containing a `0x00`/ml_dsa packet falls back to CPU sigverify. | Throughput of PQ traffic is CPU-bound. Fine for a demo; a bottleneck at load. | **Fixable → §7.2** (new CUDA kernel; not on critical path) |
| B3 | **Multi-node propagation is unproven.** PQ votes ride the regular TPU over **UDP**, not gossip CRDS; the 2b gossip path is verified only at N=2. | A real cluster's behavior (vote propagation, CRDS packing, repair) has never been observed. | **Fixable → §7.3** |
| B4 | **Shred verify is advisory by default.** ML-DSA shred failures emit metrics but don't drop packets unless `--ml-dsa-shred-strict`. | Out of the box, a forged ML-DSA trailer is logged, not rejected. | **Fixable → §7.5** (flip default once multi-node-proven) |
| B5 | **frozen-abi digests changed** (`CrdsValue`, `Protocol`) and are only regenerable under a nightly/specialization build. | Inert on our pinned stable 1.76 — but blocks any move to nightly ABI-checked CI. | **Fixable → §7.4** |
| B6 | **Phase-1 scope is single-signer, legacy `Message` only.** Multi-sig / v0 / address-lookup-table txs are rejected at decode. | No PQ multi-sig, no versioned transactions. | **Fixable → §7.5** |
| — | **Not compatible with live Solana** (`PACKET_DATA_SIZE` 8192 vs 1232). | Self-hosted only. | **By-design** — out of scope, permanent. |
| — | **Throughput parity with Ed25519.** ML-DSA verify ≈ **2.3×** cost (`ML_DSA_VERIFY_COST=5310` vs `ED25519_VERIFY_COST=2280` CU); shred volume ≈ **2×** when on. | Lower TPS ceiling; bigger blocks. | **By-design** — priced into the cost model, not a bug. |

---

## 4. Added but untested / unproven at scale

Distinct from §3: these are things the code *claims* to support (or a reader might assume it
supports) but which have **never been exercised** the way production would exercise them. Treat every
item here as "believed to work, not proven."

1. **A node signing its *own* gossip identity with ML-DSA — NOT IMPLEMENTED.** Phase 2b only proves
   the *transport + verify* path: a node can relay and verify *someone else's* pre-signed ML-DSA
   CRDS value (`ClusterInfo::push_signed_crds_value`). A node still signs its own `ContactInfo`,
   `Ping`, `Pong`, `PruneData` with Ed25519. This is entangled with B1 (identity) — see §7.1.

2. **Multi-node consensus finalizing on PQ votes — single-node only.** `demo-vote.sh` proves *one*
   validator roots on its own ML-DSA votes. A cluster where several validators vote post-quantum and
   must observe each other's votes has never run. Votes propagate over UDP TPU, **not** gossip, so a
   second node only learns of them via block replay (`core/src/voting_service.rs` `VoteOp::*MlDsaVote`).

3. **Strict shred enforcement across real turbine fan-out — unit-tested only.** `enforce_ml_dsa_shreds`
   (`ledger/src/sigverify_shreds.rs:153`) is covered by unit tests, but a **node's own** shreds bypass
   turbine sigverify (that path verifies *peer* shreds), so on one node the counters never climb.
   Behavior when a real peer sends a real ML-DSA shred under `--ml-dsa-shred-strict` is unobserved.

4. **ML-DSA CRDS propagation at N > 2 — untested and known-finicky.** Even at N=2, general CRDS
   propagation is delicate: `ContactInfo` propagates, but stake-weighted votes don't, so the 2b demo
   uses a `ContactInfo` value + bidirectional `insert_info`. Larger fan-out, packing (~3 PQ values per
   packet), and pull/push dynamics are unexplored.

5. **The GPU fallback path — never exercised.** `batches_contain_ml_dsa` +
   `warn_ml_dsa_gpu_fallback_once` (`perf/src/sigverify.rs:632`, `:643`) route PQ batches to CPU when
   perf-libs is loaded. `solana-test-validator` runs with GPU sigverify **off**, so this branch has
   never actually run against a real CUDA build. If you enable perf-libs, verify the fallback trips.

6. **Multi-signer / v0 / address-lookup-table transactions — rejected at decode.** Phase 1
   (`sdk/src/ml_dsa_transaction.rs`) accepts exactly one required signer and legacy `Message` only.
   Anything else errors out — there is no partial support to "test," just a hard boundary to extend.

7. **Pre-existing red tests (not regressions, but not green either).** On this branch, independent of
   the ML-DSA work, these fail because they hardcode the old 1232 packet size:
   - **10** in `cargo test -p solana-ledger --lib shred` — `shred::legacy::test_sanitize_data_shred`,
     `shred::tests::test_serde_compat_shred_{code,data,data_empty}`, four `blockstore::tests::*`
     byte-layout tests, two `shredder::tests::test_shred_fec_set_index::*`.
   - **3** `crds_gossip_pull` bloom-filter tests in `solana-gossip`.

   Confirmed identical red set with all ML-DSA changes stashed. **Before calling anything
   "production" these must be either fixed to the new packet size or explicitly waived** — right now
   a green/red diff can't cleanly tell "my change broke something" from "the baseline was already red."

---

## 5. What's already done (documented elsewhere — not repeated here)

This doc covers only what's *pending*. What each completed phase does, the files it touches, its
caveats, and how to test it is **not** duplicated here — that already lives in `CLAUDE.md` (each
phase's "done & verified" block) and `docs/ml-dsa-migration.md` (strategy). The *remaining* concern
for every phase is captured above in §3 (blockers) and §4 (added-but-untested); the §1 matrix
indexes each surface's status. Start there.

---

## 6. Go-live runbook — a single fully-post-quantum node

This is the most PQ you can be **today** on one machine. Everything below is verified to work.

```bash
# 0. Build (WSL2 Ubuntu, Rust pinned 1.76.0 — never native Windows; see CLAUDE.md).
cd /mnt/d/Work/infinia/solana-ml-dsa-44
cargo build --release --bin solana-test-validator --bin solana --bin solana-keygen

# 1. Mint the two post-quantum node keys (or let the flags mint them).
#    --ml-dsa-vote  = the validator signs its consensus VOTES post-quantum.
#    --ml-dsa-shred = the validator ALSO ML-DSA-signs the Merkle root of every FEC set it broadcasts.
#    --ml-dsa-shred-strict = additionally DROP any peer ml_dsa shred that fails verification.
export PATH="$PWD/target/release:$PATH"
solana-test-validator --reset --ledger ~/solana-test-ledger-mldsa \
  --ml-dsa-vote  /tmp/mldsa-vote.bin \
  --ml-dsa-shred /tmp/mldsa-shred.bin \
  --ml-dsa-shred-strict
```

With those flags this one node:
- ✅ **accepts & executes** ML-DSA user transactions (Phase 1) — `0x00` marker, CPU sigverify;
- ✅ **finalizes** on its own post-quantum consensus **votes** (Phase 2a) — authorized voter = ML-DSA address;
- ✅ **relays & verifies** others' ML-DSA gossip **CRDS** values (Phase 2b core);
- ✅ **ML-DSA-signs the Merkle root** of every block it broadcasts (Phase 3), and (strict) drops
  failing peer ml_dsa shreds.

Its *identity* (QUIC/TLS cert, gossip ContactInfo, repair) is still **Ed25519** — that is B1.

**Verify each surface (second terminal / after boot):**
```bash
# Phase 1: a PQ-signed SOL transfer confirms; a forged one is rejected.
./target/release/examples/ml_dsa_transfer           # or: bash programs/ml-dsa-tests/demo-transfer.sh
# Phase 2a: slots + the vote account's lastVote/root climb; authorized voter is the PQ address.
solana slot --commitment finalized
curl -s http://127.0.0.1:8899 -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getVoteAccounts","params":[{"commitment":"processed"}]}'
# Phase 3: a produced shred's ML-DSA-44 trailer verifies offline (own shreds bypass turbine verify).
./target/release/examples/verify_ml_dsa_shreds       # or: bash programs/ml-dsa-tests/demo-shred.sh
```

**Operational caveats:**
- Run with **GPU sigverify disabled** (the `solana-test-validator` default). If perf-libs/CUDA is
  loaded, ML-DSA batches fall back to CPU (B2); confirm the fallback trips before trusting it.
- Keep the ledger on **ext4** (`~/…`), not `/mnt/d` (slow drvfs I/O).
- PQ votes pay ~5000 lamports each; genesis funds the vote address (1M SOL). Budget for it.
- Switching a vote account between Ed25519 and ML-DSA is a **restart**, not a live swap (one
  authorized voter per epoch).

---

## 7. Continuation roadmap — unblocking the fixable items

Referenced from §3. Ordered roughly by leverage. Each item: the blocker, a proposed approach, the
files to touch, and rough effort.

### 7.1 Phase 4 — flip the node identity to the ML-DSA address  *(B1 · largest)*

**Why it's blocked.** The data plane authenticates a node by an **Ed25519-keyed X.509 certificate**
in the QUIC/TLS handshake. `streamer/src/tls_certificates.rs::new_dummy_x509_certificate` hardcodes:
- a PKCS#8 prefix carrying OID `1.3.101.112` (curveEd25519) and the raw **32-byte Ed25519 secret**
  (`keypair.secret().as_bytes()`, line 27);
- a SubjectPublicKeyInfo that embeds `keypair.pubkey()` (line 65), which the peer reads back via
  `get_pubkey_from_tls_certificate` to learn the node's `id()`.

An ML-DSA address is `sha256(1312-byte pubkey)` — it has **no Ed25519 secret** to key the TLS
handshake with. Flip `id()` to it while the cert stays Ed25519 and the node partitions from
turbine/repair/TPU (peers authenticate a different key than the one advertised).

Also fixed-width and Ed25519-only, so they block the flip too:
- `Ping` / `Pong` / `PruneData` in gossip carry a fixed 64-byte `Signature` (the shared `Signable`
  trait, untouched by Phase 2b). These are signed *by the node identity*.
- `gossip/src/cluster_info.rs` asserts `contact_info.pubkey() == keypair.pubkey()` and uses `id()`
  as the identity throughout.

**Proposed approach (sequenced):**
1. **ML-DSA X.509 for QUIC.** Teach `tls_certificates.rs` to build (and parse) a certificate whose
   SubjectPublicKeyInfo carries the ML-DSA public key, and wire an ML-DSA-capable signature scheme
   into the rustls config. This is the crux and the riskiest — rustls/QUIC must accept the custom
   scheme on both the server and client verifier. Prototype in isolation first.
2. **ML-DSA `Ping`/`Pong`/`PruneData`.** Give each an enum-signature treatment mirroring Phase 2b's
   `CrdsSignature` (fixed sig → `Ed25519 | MlDsa`), so a node can sign these under its ML-DSA identity.
3. **Own gossip identity.** With `id()` = ML-DSA address, make `ContactInfo` (and the other self-signed
   values) sign via `CrdsValue::new_signed_ml_dsa` — this is exactly item §4.1, unblocked here.
4. **Identity threading.** `id()` is set once at startup (`core/src/validator.rs`) and threaded
   statically through `tpu.rs`/`tvu.rs`/`cluster_info.rs`; audit every `keypair.pubkey()` assumption.

**Effort:** the biggest remaining piece — comparable to Phases 2 and 3 combined (TLS-library work +
three protocol-message variants + a full identity audit + multi-node testing). Do it **after** §7.3
(you need a working multi-node harness to prove it).

### 7.2 GPU / CUDA ML-DSA verify  *(B2 · off critical path)*

**Where the fallback is.** `perf/src/sigverify.rs`: `ed25519_verify` (line 653) calls
`batches_contain_ml_dsa` (line 632); if any batch holds a `0x00` packet it logs once
(`warn_ml_dsa_gpu_fallback_once`), bumps `sigverify_ml_dsa_gpu_fallback`, and routes the whole set to
`ed25519_verify_cpu`. The CUDA `ed25519_verify` kernel's fixed 64-byte offset math would mis-parse a
2420-byte signature and silently fail, which is why the fallback exists.

**Approach.** Write a new CUDA kernel in perf-libs that understands the ML-DSA packet layout (2420-B
sig, 1312-B key, `sha256(pubkey)==signer` binding) and dispatch PQ batches to it instead of the CPU.
Large, standalone, NVIDIA-specific.

**Effort:** comparable to Phase 4, but **not on the critical path** — CPU sigverify is correct today;
the GPU kernel is a throughput optimization only. Defer until PQ load actually matters.

### 7.3 True multi-node post-quantum propagation  *(B3 · do this first)*

This is the highest-leverage next step because it *unblocks the testing* for everything else (§7.1, B4).

- **Stand up a multi-node harness.** `multinode-demo/*` boots an Ed25519 cluster; adapt it to pass the
  `--ml-dsa-vote` / `--ml-dsa-shred` flags and PQ vote authorities to each validator. This is where
  §4.2/§4.3/§4.4 finally get observed.
- **Make PQ votes gossip-observable.** Today they ride UDP TPU only (`voting_service.rs`
  `VoteOp::PushMlDsaVote` → `send_transaction_raw`), so peers see them only via block replay. Decide
  whether that's acceptable for a cluster or whether votes should also propagate via a CRDS value
  (needs the 2b path + likely §7.1 for a PQ-signed identity).
- **Exercise CRDS at N>2.** Confirm PQ `CrdsValue`s propagate under real pull/push, measure packing
  (~3 PQ values/packet), and re-check the N=2 finickiness (stake-weighted votes) at scale.

**Effort:** medium — mostly harness + observation, minimal new crypto. Highest priority.

### 7.4 frozen-abi digest regeneration  *(B5 · inert today)*

Phase 2b changed the `CrdsValue` and `Protocol` ABI digests; `MlDsaSignature`/`MlDsaPublicKey` got
manual specialization-gated `AbiExample` impls. On the pinned **stable 1.76** toolchain frozen-abi is
inert, so builds/tests are unaffected. If you move to a nightly/specialization build (or add
ABI-checked CI), regenerate:
```bash
SOLANA_ABI_BULK_UPDATE=1 cargo test    # under a nightly/specialization build
```
**Effort:** trivial once on the right toolchain — but a hard prerequisite for any ABI-gated CI.

### 7.5 Productionizing the rest  *(B4, B6)*

- **Flip shred verify to gating by default (B4).** Once multi-node (§7.3) proves honest ml_dsa shreds
  verify reliably, consider making `enforce_ml_dsa_shreds` the default and `--ml-dsa-shred-strict` the
  no-op / removing the advisory mode. Keep advisory as the roll-out safety valve until then.
- **Multi-signer / v0 / lookup-table transactions (B6).** Extend `sdk/src/ml_dsa_transaction.rs` beyond
  one required signer and legacy `Message`. Note the size ceiling: each additional ML-DSA signer adds
  ~3.7 KB, so cap signers per transaction and document the limit (this is the "multi-signer balloons"
  risk in `docs/ml-dsa-migration.md` §8).
- **Resolve the pre-existing red tests (§4.7).** Update the 10 ledger `shred` + 3 bloom tests to the
  `PACKET_DATA_SIZE=8192` layout, or mark them `#[ignore]` with a comment, so the suite is a clean
  signal.

---

## 8. Invariants that must hold

A continuation dev can silently corrupt the wire layout by breaking one of these. (The per-surface
file/anchor map is not duplicated here — it's in `CLAUDE.md`, and the anchors relevant to pending
work are cited inline in §3/§4/§7.)

- ML-DSA **pubkey = 1312 B, signature = 2420 B** (`const_assert`ed in `merkle.rs:63`,`:65`).
- **Commitment = `sha256(ml_dsa_pubkey)`**, and every verify enforces `ml_dsa_address(pubkey) ==
  identity/commitment` (Phase 1, 2b, 3 all share this binding). This is what stops a forged key /
  swapped trailer.
- **Empty signing context** for every `fips204` sign/verify — required for `@noble/post-quantum`
  byte-compatibility. Do not add a context.
- **`PACKET_DATA_SIZE = 8192`.** If it ever changes again, the ripple `const_assert`s must move
  together: `sdk/src/offchain_message.rs`, the shred-size asserts in `ledger/src/shred*.rs` +
  `core/src/repair/serve_repair.rs`, and the base58/base64 caps in `rpc/src/rpc.rs`
  (`MAX_BASE58_SIZE`/`MAX_BASE64_SIZE`). (See `CLAUDE.md` "Packet ceiling raised.")
- **`fips204` pinned at 0.4.6** — the vendored NIST vectors and MSRV depend on it.

---

## 9. Pointers

- **Strategy & risk:** `docs/ml-dsa-migration.md` (§2 the five surfaces, §7 roadmap, §8 risks).
- **Per-phase implementation notes & caveats:** `CLAUDE.md` (each phase has a "done & verified" block).
- **Conformance suite:** `programs/ml-dsa-tests/tests/fips204_vectors.rs` +
  `programs/ml-dsa-tests/cross-impl/` (NIST ACVP KAT + `@noble/post-quantum` byte-for-byte cross-check).
- **Live demos** (each prints raw JSON-RPC at every chain interaction; run from a WSL bash shell):
  `programs/ml-dsa-tests/demo.sh` (P0) · `demo-transfer.sh` (P1) · `demo-vote.sh` (P2a) ·
  `demo-shred.sh` (P3). Pass `--build` after editing Rust.
- **Examples:** `programs/ml-dsa-tests/examples/{submit_live,ml_dsa_transfer,bench_verify,verify_ml_dsa_shreds}.rs`,
  `perf/examples/sigverify_ml_dsa.rs`.

---

*Last updated for `feat/ml-dsa-44` at commit `5340dd5c8`. Phases 0–3 (+ 2b core) delivered and
single-node verified; Phase 4 (node identity) not started. "Live" throughout means a self-hosted
post-quantum network, never live public Solana.*
