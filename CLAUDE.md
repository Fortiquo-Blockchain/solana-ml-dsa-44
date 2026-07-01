# CLAUDE.md — infinia (Solana validator clone)

Clone of the official Solana validator monorepo (Rust/Cargo workspace; crates & binaries are still branded `solana`, version `2.0.0`). **Built and run via WSL2 Ubuntu — native Windows is not supported for the validator.**

## Workflow — what runs where ⚠️ read first

This repo is edited on Windows but builds/runs in WSL. Keep the three roles separate:

| Role                                | Launch from                                                                           | Runs in    | For                                       |
| ----------------------------------- | ------------------------------------------------------------------------------------- | ---------- | ----------------------------------------- |
| **Claude Code** (this agent)        | a **Windows** terminal (PowerShell): `cd D:\Work\infinia\solana-ml-dsa-44` → `claude` | Windows    | driving work; runs builds/git via `wsl …` |
| **Cursor** (editor + rust-analyzer) | a **WSL** shell: `cursor .`                                                           | Remote-WSL | reading/editing code                      |
| **Build / run / git**               | a **WSL** shell (or Cursor's terminal)                                                | WSL        | `cargo`, `solana-test-validator`, `git`   |

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

- JSON-RPC: `http://127.0.0.1:8899` • WebSocket: `ws://127.0.0.1:8900`
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

1. From a WSL shell in the repo: `cursor .` (or Cursor → Command Palette → "Reopen Folder in WSL").
2. Install the **rust-analyzer** extension _in WSL_ (the Extensions panel offers "Install in WSL: Ubuntu").
3. `rust-src` is installed and `.vscode/settings.json` tunes rust-analyzer for this large workspace (separate target dir so it won't invalidate your `--release` build). First index takes a few minutes.

## Post-quantum signatures (ML-DSA-44) — the fork goal

This fork migrates Solana's signatures from **Ed25519 to ML-DSA-44** (NIST FIPS 204, post-quantum) across the **five signing surfaces** a validator uses (app-level verify, user payments, validator votes, node gossip, block broadcasting). Phases 0–3 are **done & verified** on a single node; every post-quantum path **coexists** with Ed25519 (flag-gated / `0x00`-marker), so the node never stalls. Phase 4 (flip the node identity itself to an ML-DSA address) is deferred — blocked by the QUIC/TLS Ed25519 cert wall.

**All ML-DSA-44 docs live in [`docs/ml-dsa-44/`](docs/ml-dsa-44/README.md)** — read the file relevant to the task. The per-phase engineering detail is no longer duplicated here.

| Doc                                                                                                                      | Read it for                                                                  |
| ------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------- |
| [`docs/ml-dsa-44/overview.md`](docs/ml-dsa-44/overview.md)                                                               | Status, progress, the roadmap, delivered-by-phase (management view)          |
| [`docs/ml-dsa-44/strategy.md`](docs/ml-dsa-44/strategy.md)                                                               | Why it's hard (the two walls), the five surfaces, technology choice          |
| [`docs/ml-dsa-44/implementation.md`](docs/ml-dsa-44/implementation.md)                                                   | Per-phase **engineering detail** — exact files, flags, wire formats, caveats |
| [`docs/ml-dsa-44/remaining-work.md`](docs/ml-dsa-44/remaining-work.md)                                                   | Blockers, added-but-untested, the continuation roadmap, **wire invariants**  |
| [`docs/ml-dsa-44/runbook.md`](docs/ml-dsa-44/runbook.md)                                                                 | Every build / run / demo / test command                                      |
| [`docs/ml-dsa-44/explorer-roadmap.md`](docs/ml-dsa-44/explorer-roadmap.md) · [`glossary.md`](docs/ml-dsa-44/glossary.md) | The PQ block-explorer plan · term definitions                                |

**Maintaining these docs (do this, don't drift):** keep every ML-DSA doc inside `docs/ml-dsa-44/` — never scatter new ML-DSA notes into other folders, and never edit the upstream Docusaurus site under `docs/src/`. One topic has **one owner** (roadmap → `overview.md`; commands → `runbook.md`; per-phase engineering → `implementation.md`; terms → `glossary.md`); link, don't re-document. `remaining-work.md` is a _living_ doc — when an item lands, **delete** it there rather than marking it done (history belongs in git). See [`docs/ml-dsa-44/README.md`](docs/ml-dsa-44/README.md) for the full map + maintenance rules.

## Don't

- Don't build from native Windows/PowerShell (symlinks are WSL-style; the validator is unsupported on Windows).
- Don't run the bash scripts (`multinode-demo/*`, `scripts/cargo-install-all.sh`) from PowerShell — they are bash-only.
