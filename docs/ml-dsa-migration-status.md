# Post-Quantum Blockchain Migration — Team Status

**For:** Product, engineering, and leadership  
**Updated:** June 2026  
**One-line summary:** We are building a quantum-safe version of our blockchain. The core proof works today on our own test network.

---

## At a glance

| Question | Answer |
|----------|--------|
| **What are we building?** | A blockchain that uses **post-quantum (PQ) signatures** instead of today’s standard |
| **Why?** | Future quantum computers may break today’s cryptography. We want to be ready early |
| **Where does it run?** | On **our own network only** — not on public Solana mainnet |
| **Is it working?** | **Yes** — proven end-to-end in local demos |
| **Is it production-ready?** | **Not yet** — needs wallet, explorer, multi-node testing, and further hardening |

---

## Overall progress

```
Core validator work (Phases 0–3)     ████████████████████░░  ~90%  ✅ Proven
Pilot network (wallet, explorer)     ████████░░░░░░░░░░░░░░░░  ~40%  🔄 Next
Full PQ-only production network      █████░░░░░░░░░░░░░░░░░░░  ~25%  ⏳ Later
```

**Key message for the team:** The hardest part — proving PQ signatures work on a real chain — is **done**. What’s left is mostly **product tooling and rollout**, not inventing new cryptography.

---

## What we’re changing (in simple terms)

Today’s blockchain uses **Ed25519** signatures — small and fast, but vulnerable to future quantum attacks.

We are moving to **ML-DSA-44**, the NIST-standard post-quantum algorithm. The trade-off: signatures are much larger, so we had to adapt how the network handles keys and transactions.

| Challenge | How we solved it |
|-----------|------------------|
| Keys are too large to use as account addresses | Use a short address derived from the key; carry the full key in the transaction |
| Transactions are too large for the old network limits | Raised limits on **our** network (this is why we run our own chain) |

---

## Five areas we upgraded — all proven ✅

A validator signs five different types of messages. We have upgraded all five at demo level.

| Area | What it means | Status |
|------|---------------|--------|
| **1. Signature verification** | The chain can verify a PQ signature on demand | ✅ Done |
| **2. User payments** | Someone can send funds using a PQ-signed transaction | ✅ Done |
| **3. Validator votes** | The network can agree on blocks using PQ-signed votes | ✅ Done |
| **4. Node communication** | Validators can exchange PQ-signed messages | ✅ Core done |
| **5. Block broadcasting** | Block producers can attach PQ attestations to blocks | ✅ Done |

**How it runs today:** The old (Ed25519) path still works by default. PQ is turned on feature-by-feature so nothing breaks during rollout.

---

## What we delivered — by milestone

### ✅ Milestone 1 — Prove PQ signatures work on-chain
- Valid PQ signatures are accepted; invalid ones are rejected
- Tested against official NIST standards

### ✅ Milestone 2 — PQ payments
- Users can send SOL with post-quantum signatures
- Works alongside normal transactions

### ✅ Milestone 3 — PQ consensus votes
- The validator can vote and finalize blocks using PQ signatures
- Tested on a live local network

### ✅ Milestone 4 — PQ node messaging
- PQ-signed messages can pass between nodes and be verified

### ✅ Milestone 5 — PQ block attestations
- Blocks can carry additional PQ proof that they are authentic

**Also delivered:** key generation tools, cost modeling, automated tests, live demo scripts, and documentation.

---

## Known limitations today

These are acceptable for a demo — not for a public launch:

- Best tested on a **single machine** so far (multi-node cluster still needed)
- **No browser wallet** yet — demos run from the command line
- **No block explorer** yet — hard to show progress to non-engineers
- PQ is **optional** — the network still relies on Ed25519 in several places
- **Slower** than standard signatures (expected for post-quantum)

---

## What’s next — three phases of remaining work

### Phase A — Pilot network *(recommended focus)*

Make the project visible and usable beyond engineering.

| Item | Why the team should care | Rough effort |
|------|--------------------------|--------------|
| **Block explorer** | Lets anyone see PQ transactions in a browser | 6–10 weeks |
| **Web / mobile wallet** | Needed for real users to send PQ payments | 8–12 weeks |
| **Multi-node test cluster** | Proves the network works with multiple validators | 4–6 weeks |
| **Test cleanup** | Improves CI confidence | 1–2 weeks |

See [`ml-dsa-explorer-roadmap.md`](./ml-dsa-explorer-roadmap.md) for explorer details.

---

### Phase B — Full identity switch *(not started)*

Replace remaining Ed25519 dependencies so the network can run PQ-only.

| Item | Why it matters |
|------|----------------|
| PQ node identity | Nodes authenticate fully as PQ, not hybrid |
| Network security (TLS) redesign | Today’s secure connections require old-style keys |
| Complete PQ gossip identity | Finish node-to-node PQ signing |

**This is the largest remaining engineering effort** — estimated 3–4 months when we start.

---

### Phase C — Production readiness *(after pilot)*

| Item | Why it matters |
|------|----------------|
| Multi-signer transactions | Real apps often need more than one signer |
| Broader transaction types | Compatibility with modern app patterns |
| Performance tuning | PQ verification is CPU-heavy |
| Operations playbooks | Monitoring, key rotation, incident response |
| External security audit | Required before any production launch |

---

## Suggested timeline

| When | Focus |
|------|-------|
| **Now – Sep 2026** | Explorer MVP, wallet, multi-node cluster |
| **Sep – Dec 2026** | Phase B design and start (if approved) |
| **Q1 2027** | Production hardening and security audit |

*Timeline depends on team size and priorities.*

---

## Risks the team should know

| Risk | What it means for us |
|------|----------------------|
| **Not on public Solana** | This is our chain — by design |
| **Slower than today** | Acceptable for pilot; not a mainnet competitor yet |
| **Two systems in parallel** | Ed25519 + PQ coexist until Phase B is done |
| **No wallet or explorer yet** | Hard to demo to customers without Phase A |
| **Knowledge in one place** | Documentation and handoffs are important |

---

## Decisions we need from leadership

1. **What is the target?** Demo only, pilot with users, or full PQ production?
2. **Fund Phase A?** Explorer + wallet are the fastest way to make progress visible
3. **Is PQ-only a near-term goal?** If yes, Phase B needs to be prioritized sooner
4. **Accept scope:** This will not plug into public Solana mainnet

---

## How engineering proves it works

Engineering can run **live demos** that start a test network, submit real PQ transactions, and show the results. Each demo is self-contained and uses real cryptography — nothing is mocked.

For step-by-step instructions, see [`ml-dsa-runbook.md`](./ml-dsa-runbook.md).

---

## Quick glossary

| Term | Simple meaning |
|------|----------------|
| **Post-quantum (PQ)** | Cryptography safe against future quantum computers |
| **Ed25519** | Today’s standard signature type (fast, not quantum-safe) |
| **ML-DSA-44** | The PQ signature standard we are adopting (NIST approved) |
| **Validator** | The server that runs the blockchain |
| **Proof of concept** | Works in controlled tests — not yet ready for public launch |

---

## More detail (for those who want it)

| Document | Contents |
|----------|----------|
| [`ml-dsa-overview.md`](./ml-dsa-overview.md) | Presentation overview — goal, roadmap, done vs remaining |
| [`ml-dsa-migration.md`](./ml-dsa-migration.md) | Full technical strategy |
| [`ml-dsa-explorer-roadmap.md`](./ml-dsa-explorer-roadmap.md) | Block explorer plan |
| [`ml-dsa-runbook.md`](./ml-dsa-runbook.md) | How to run and test each phase |
| [`../CLAUDE.md`](../CLAUDE.md) | Build and environment setup |
