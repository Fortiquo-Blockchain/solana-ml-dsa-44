# ML-DSA Migration — Glossary

One place for the terms used across the migration docs. Plain-language first;
the fork-specific wire terms follow.

## General terms

| Term                     | Plain meaning                                                                                                                                    |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Post-quantum (PQ)**    | Designed to stay secure even against future quantum computers.                                                                                   |
| **Ed25519**              | Today's signature scheme — small (32-byte key, 64-byte signature) and fast.                                                                      |
| **ML-DSA-44 / FIPS 204** | The quantum-resistant replacement and the NIST standard that defines it (finalized Aug 2024). Much larger: 1,312-byte key, 2,420-byte signature. |
| **Public key / address** | Today the same 32-byte value; the migration splits them apart — `address = sha256(public_key)`.                                                  |
| **Precompile**           | A built-in on-chain verify function (no custom program needed) — the self-contained surface we start with (Phase 0).                             |
| **Validator**            | A server/node that processes transactions and produces blocks.                                                                                   |
| **Gossip**               | How validators discover and communicate with each other.                                                                                         |
| **Shred**                | A small fragment of a block sent over the network (turbine).                                                                                     |
| **Packet**               | One network-sized chunk; today an entire transaction must fit in one.                                                                            |
| **PoC**                  | Proof of concept — works in controlled conditions, not yet production-hardened.                                                                  |

## Fork-specific wire terms

| Term                          | Meaning                                                                                                                                               |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`0x00` marker**             | Lead byte tagging a Phase-1 ML-DSA transaction; a stock Ed25519 tx starts with a signature count ≥ 1.                                                 |
| **Synthetic tx id**           | The 64-byte transaction id for an ML-DSA tx = `sha256(sigs)‖sha256(msg)` (a 2,420-byte signature can't be the id the runtime keys on).                |
| **Commitment** (Phase 3)      | `sha256(leader ML-DSA pubkey)`, 32 B, embedded inside the Ed25519-signed Merkle region of a shred.                                                    |
| **Trailer** (Phase 3)         | `[pubkey 1312 ‖ signature 2420]` = 3,732 B carried after the Merkle proof (not Ed25519-signed, not erasure-coded).                                    |
| **Advisory vs strict**        | Phase-3 shred verify is _advisory_ (telemetry only) by default; `--ml-dsa-shred-strict` makes it _gating_ (drops failing PQ shreds).                  |
| **Address binding**           | Every PQ verify enforces `ml_dsa_address(pubkey) == identity/commitment` — this is what stops a forged key or a swapped trailer.                      |
| **`PACKET_DATA_SIZE = 8192`** | The fork's raised packet ceiling (was 1,232) so one ~3.9 KB PQ transaction fits — the change that makes the fork incompatible with mainnet by design. |
