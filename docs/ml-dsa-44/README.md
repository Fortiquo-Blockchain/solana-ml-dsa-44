# ML-DSA-44 Migration — Documentation

This folder is the **single home** for all documentation on this fork's
migration from Ed25519 to **ML-DSA-44** (post-quantum, NIST FIPS 204). If it's
about the ML-DSA migration, it lives here.

**Status in one line:** Phases 0–3 delivered & verified on a single node (all
five signing surfaces upgraded, coexisting with Ed25519); Phase 4 (flip the node
identity itself to an ML-DSA address) is deferred. Authoritative detail in
[`overview.md`](./overview.md).

---

## Map — which file owns what

Each topic has exactly **one owner**. Read the file that matches your need;
don't expect the same topic documented in two places.

| File                                           | Audience             | Owns (single source of truth)                                                                                            |
| ---------------------------------------------- | -------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| [`overview.md`](./overview.md)                 | Management / leads   | Goal, progress %, status dashboard, **the roadmap & timeline**, delivered-by-phase summary, leadership risks & decisions |
| [`strategy.md`](./strategy.md)                 | Technical / leads    | _Why_ it's hard (the two walls), the five surfaces explained, order of difficulty, technology choice, technical risks    |
| [`implementation.md`](./implementation.md)     | Engineers            | Per-phase **engineering detail** — exact files, flags, wire formats, caveats                                             |
| [`remaining-work.md`](./remaining-work.md)     | Engineers (next dev) | Blockers, added-but-untested, the continuation roadmap, **wire invariants**                                              |
| [`runbook.md`](./runbook.md)                   | Anyone running it    | **Every** build / run / demo / test command (scripts + manual CLI)                                                       |
| [`explorer-roadmap.md`](./explorer-roadmap.md) | Product / leads      | The Solscan-style PQ block-explorer build plan                                                                           |
| [`glossary.md`](./glossary.md)                 | Everyone             | Term definitions                                                                                                         |

**Quick routes:**

- _"Present this to leadership"_ → [`overview.md`](./overview.md)
- _"Why is this hard / how does it work?"_ → [`strategy.md`](./strategy.md)
- _"I'm implementing / debugging a surface"_ →
  [`implementation.md`](./implementation.md) +
  [`remaining-work.md`](./remaining-work.md)
- _"How do I run or test it?"_ → [`runbook.md`](./runbook.md)

Engineering environment (build via WSL2, symlink fix, toolchain) is in the repo
root [`../../CLAUDE.md`](../../CLAUDE.md); the public summary is
[`../../README.md`](../../README.md).

---

## Maintaining these docs — read before editing

To keep this from re-fragmenting (the reason it was consolidated):

1. **Keep every ML-DSA doc in this folder.** Never scatter new ML-DSA notes into
   other directories, commit messages aside. If you add a doc, add it to the map
   above.
2. **Never edit the upstream Docusaurus site under `docs/src/`** (or the tooling
   in `docs/`). That is the original Solana documentation, kept untouched. This
   folder is separate on purpose.
3. **One topic, one owner** (see the map). Extend the owning file; **link**,
   don't re-document. If you catch yourself pasting the roadmap, a command
   block, or a term definition into a second file, stop — link to the owner
   instead.
4. **`remaining-work.md` is a living doc.** When an item lands, **delete** it
   there (and update the §1 status matrix) rather than marking it "done" —
   completion history belongs in git, not the doc.
5. **Keep `../../CLAUDE.md` a pointer.** Per-phase engineering detail belongs in
   [`implementation.md`](./implementation.md), not back in CLAUDE.md.
