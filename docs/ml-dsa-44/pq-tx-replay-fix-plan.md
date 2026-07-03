# Plan — make post-quantum transactions survive multi-node block replay

> **Status:** Option A **fully implemented** (steps 1–4). Votes and user payments
> both ride the envelope carrier; the `0x00` `MlDsaTransaction` format and its
> banking/sigverify/RPC handlers are deleted. Owner of the problem statement:
> [`remaining-work.md`](./remaining-work.md) §4.2 / B3 / §7.3.

> **What actually shipped vs. this proposal** (the design below is the reasoning; a
> few load-bearing details settled differently during implementation — trust this
> note over the prose):
>
> - **Carrier is the _last_ instruction**, not instruction 0 (`ml_dsa_envelope.rs`;
>   the verifier takes `instructions.len()-1` as the carrier and rebuilds everything
>   before it as the signed core).
> - **The envelope check self-verifies** the ML-DSA signature, the
>   `sha256(pubkey)==signer` binding, and the anti-lift rebuild inside
>   `verify_and_hash_message`. It does **not** rely on `verify_precompiles` for
>   authorization (that still runs, but enforces none of those bindings).
> - **The placeholder/synthetic signature is retained**, repurposed as the
>   dedup/status id (and pinned by a check to block id-malleability) — not deleted.
> - **Scope is legacy message + single signer** (V0 is rejected).
> - **No runtime feature gate** (see the constraint note in `ml_dsa_envelope.rs`):
>   inert for non-carrier traffic, but relies on all nodes being upgraded together.
> - The §7 "open questions" (signing context, address binding, PQ-slot marking) are
>   all resolved in code: empty FIPS-204 context; `sha256(pubkey)==signer`; PQ-ness
>   is "the last instruction is an ML-DSA carrier".

## Plain-language summary (read this first)

Today a post-quantum (ML-DSA) transaction works on one machine but breaks the
moment a second machine checks it. When a validator files the transaction into a
block, it **throws the real signature away** and pins on a fake placeholder.
Another machine opens the block, looks for a valid signature, finds only the
placeholder, and rejects the whole block — so a real multi-node cluster can never
agree. This hits **both** post-quantum votes _and_ post-quantum user payments
(single-node just never noticed, because a machine never re-checks its own
blocks).

The fix: **stop throwing the signature away — carry it inside the transaction as
a standard "attachment" that every machine already knows how to verify**, and
teach the one check that currently rejects the placeholder to trust that
attachment instead. This reuses the ML-DSA verifier built in Phase 0, changes
nothing about how blocks are stored or hashed, and lets us delete the broken
placeholder trick entirely.

We chose this over the alternative (rebuilding how blocks are stored) because
this leaves the most dangerous, irreversible layer untouched and keeps the fork
close to upstream Solana. See "Why this and not the block-format change" below.

---

## 1. The defect, precisely

An ML-DSA transaction (`sdk/src/ml_dsa_transaction.rs`, `0x00` marker) carries a
2420-byte signature that does not fit a transaction's 64-byte signature slot. The
current design (Phase 1) verifies the ML-DSA signature **only at TPU ingress**
(`perf/src/sigverify.rs`), then bridges the packet to a normal
`VersionedTransaction` whose single signature is a 64-byte **synthetic id**
(`MlDsaTransaction::synthetic_signature` = `sha256(sig) ‖ sha256(msg)`), throwing
the real proof away (`core/src/banking_stage/immutable_deserialized_packet.rs::new_ml_dsa`).

That synthetic-id transaction is what gets recorded into the block. When a **peer
replays** the block, `blockstore_processor` →
`bank.verify_transaction(.., FullVerification)` →
`VersionedTransaction::verify_and_hash_message` Ed25519-verifies the synthetic id
against the message and returns `SignatureFailure`, so the slot is marked **dead**
(`replay-stage-mark_dead_slot`, `InvalidTransaction(SignatureFailure)`). The block
carries no ML-DSA proof, so a peer cannot re-verify even in principle.

**This is the whole `0x00` family, not just votes.** Any `0x00` transaction that
lands in a block dies on peer replay by the same path — Phase-1 user payments
included. They have only ever been exercised single-node, which is why it went
unnoticed.

## 2. Decision: precompile-backed signatures (Option A), not a block-format change (Option B)

| | **A — carry the proof as a precompile instruction (chosen)** | **B — store the raw ML-DSA tx in the block** |
| --- | --- | --- |
| Proof travels in | the transaction (instruction data) — already recorded & replayed | a new block/entry structure |
| Verified on replay | yes — `verify_precompiles` already runs there (see §3) | yes, via new replay logic |
| Touches PoH / shreds / blockstore serialization | **no** | **yes** (deepest, irreversible-if-wrong layer) |
| ML-DSA verification pathways | **one** (reuses the Phase-0 precompile) | two (Phase-0 precompile + a second tx path) |
| Divergence from upstream Solana (fork maintainability) | low (block format identical) | high (core consensus format forked) |
| Deletes the `0x00` hack | **yes** | no — entrenches a bespoke format |
| New/novel semantics | "a signer's signature can be proven by a precompile" | new block serialization |

**Rationale:** Option A leaves the layer where mistakes are catastrophic and
unrecoverable (block format, PoH hashing) completely untouched, keeps the fork
mergeable with upstream, collapses onto a single ML-DSA verifier, and removes the
`0x00` debt instead of relocating it. Its one genuinely new idea — a signature
proven by a precompile rather than the transaction envelope — is contained to the
signature-verification step (§4). Option B is a legitimate, secure alternative;
we reject it only because it changes the most dangerous layer and worsens fork
maintenance.

## 3. What we verified against the code (the previously-uncertain part)

The concern was fee-payer / signer ordering. Findings:

- `verify_precompiles` **already runs on both processing paths**: replay
  (`runtime/src/bank.rs:6597`, inside `verify_transaction`) and TPU/banking
  (`core/src/banking_stage/immutable_deserialized_packet.rs:151`). A precompile
  proof is therefore verified deterministically on every node with no new
  plumbing.
- It runs **before fee collection** (`load_and_execute_transactions`,
  `bank.rs:4582`, executes after `verify_transaction`), so a fee-payer proven by
  a precompile is established before the fee is charged.
- The **only** thing that rejects the transaction on replay is the envelope
  signature check `verify_and_hash_message` (`bank.rs:6584`, immediately before
  `verify_precompiles` in the same function). That single function is the hook.

So the fee-payer worry resolves: we keep the signer **slot** exactly as it is
(fee, `is_signer`, and vote-authority machinery unchanged) and only change **what
verifies that slot** — Ed25519 today, ML-DSA-precompile proof for PQ signers.

## 4. Design

A PQ transaction becomes a **standard `VersionedTransaction`** that:

1. keeps its ML-DSA signer(s) in the normal signer slot(s) (so `is_signer`, fee
   payment, and program authority checks are untouched), with a **placeholder**
   envelope signature for each PQ signer;
2. carries, as **instruction 0**, a Phase-0 ML-DSA **precompile instruction**
   (`sdk/src/ml_dsa_instruction.rs::new_ml_dsa_instruction`) proving that the
   ML-DSA public key signed **the transaction message** and that
   `sha256(pubkey) == the signer address` (the same binding Phase 1/2b/3 already
   enforce).

Verification changes (feature-gated) at the two envelope-signature checkpoints:

- **Replay:** `verify_and_hash_message` / `_verify_with_results`
  (`sdk/src/transaction/versioned/mod.rs`, `sdk/src/transaction/mod.rs`) — for a
  signer slot marked PQ, do **not** Ed25519-verify the placeholder; instead
  require a matching, valid ML-DSA precompile in the same transaction (the
  precompile itself is verified by the adjacent `verify_precompiles`). Bind the
  precompile's signed message to this transaction's message so a proof cannot be
  lifted to another transaction.
- **TPU:** `perf/src/sigverify.rs` already special-cases `0x00`; it moves to the
  same "trust the precompile" rule so both paths agree.

Everything downstream (fee, `is_signer`, vote program, gossip, repair, RPC) sees
an ordinary transaction with an ordinary signer, so **nothing else changes** —
and because the vote becomes an ordinary transaction again, it can propagate via
gossip like any vote, which **closes the Step-4 propagation question** too.

## 5. Rollout (staged, feature-gated, no big-bang)

1. **Add the new path behind a feature gate**, `0x00` still working. Define the
   PQ-signer transaction shape + the precompile-backed verification in
   `verify_and_hash_message` and `perf` sigverify.
2. **Move votes onto it** (`core/src/replay_stage.rs::generate_vote_tx`): build a
   normal vote tx with the ML-DSA precompile instead of the `0x00`
   `MlDsaTransaction`. Flip the ignored `test_mldsa_votes_finalize_cluster` to a
   passing multi-node finalization test.
3. **Move user payments onto it** (`sdk/src/ml_dsa_transaction.rs` callers). Add
   a multi-node Phase-1 payment test to the harness.
4. **Delete `0x00`**: remove `MlDsaTransaction`, `synthetic_signature`, and the
   TPU-only bridge once nothing uses them. This is the debt-removal commit.

## 6. Effort, risk, and testing

- **Effort:** roughly 1–2 weeks. Bulk is step 1 (the verification change +
  feature gate) and careful tests; steps 2–4 are mechanical once step 1 lands.
- **Risk:** consensus-critical (signature verification). Mitigations: feature
  gate; keep `0x00` alive until parity is proven; the existing N=2 multi-node
  harness (`local-cluster/tests/ml_dsa_cluster.rs`) is the acceptance test — the
  vote and payment tests must root across peers.
- **Failure mode is contained:** a bug here rejects/accepts a transaction (caught
  by tests, reversible by flipping the feature flag) — it does **not** corrupt
  the ledger, unlike Option B.

## 7. Open questions to nail during implementation

- **Message binding & malleability:** exactly which bytes the precompile signs
  (the transaction message with signature slots zeroed) so a proof is bound to
  its transaction and can't be replayed into another.
- **Signing context:** the Phase-0 precompile signs with `ML_DSA_CONTEXT`; the
  wire-format invariant elsewhere is an **empty** context
  ([`remaining-work.md`](./remaining-work.md) §8). Confirm one consistent context
  for the vote/payment path (and its `@noble/post-quantum` cross-impl impact).
- **Address binding in the precompile:** confirm the precompile enforces (or is
  wrapped to enforce) `sha256(pubkey) == signer address`, not just "sig valid".
- **How a signer slot is marked "PQ"** (a marker byte vs. deriving it from the
  presence of a matching precompile) — pick the representation that keeps
  `verify_and_hash_message` simple and unambiguous.
- **frozen-abi:** confirm no digest impact (expected: none, since the transaction
  stays a standard `VersionedTransaction`).
- **Multi-signer:** whether to support >1 PQ signer per tx now or keep the
  single-signer boundary (interacts with B6).
