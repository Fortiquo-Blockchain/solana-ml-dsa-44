# ML-DSA-44 FIPS 204 conformance & cross-implementation suite

This locks down the claim the fork rests on: our signatures really are **NIST
FIPS 204 (ML-DSA-44)**, and our Rust validator and JS wallet stack produce
**identical bytes**. It closes the open High risk in `docs/ml-dsa-44/strategy.md`
§8 ("must match the reference implementation exactly").

## Pieces

| File                                        | Side                       | What it pins                                                                                                                                 |
| ------------------------------------------- | -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `../tests/vectors/ml_dsa_44_kat.json`       | shared                     | NIST ACVP answer-key excerpt (keygen, sigGen, sigVer)                                                                                        |
| `../tests/fips204_vectors.rs`               | Rust (`fips204`)           | keygen on our `MlDsaKeypair` API + sign/verify on the `fips204` core, all vs NIST; our external sign round-trip; interop vs the JS signature |
| `check_vectors.mjs`                         | JS (`@noble/post-quantum`) | keygen vs NIST; emits the deterministic external signature the Rust interop check consumes                                                   |
| `../tests/vectors/cross_impl_external.json` | shared (generated)         | the JS-produced external signature artifact                                                                                                  |
| `gen_fixture.py`                            | provenance                 | regenerates the NIST fixture from `fips204`'s vendored vectors                                                                               |

## Key facts the design hinges on

- **Keygen is deterministic** in the 32-byte seed → exact byte match against NIST,
  on our real `MlDsaKeypair::from_seed`.
- **Signing is hedged** (randomized) by default in both libraries → its bytes are
  not reproducible. For the answer-key checks we use deterministic signing
  (`fips204` `_internal_sign` with rnd = 0; `@noble` `sign(..., {extraEntropy:false})`).
- The NIST sigGen/sigVer vectors here are the **internal** interface (raw message).
  Our product path is the **external** interface with an **empty context**, which
  FIPS 204 formats as `M' = 0x00 || 0x00 || M` before internal signing — that is
  the bridge the interop check uses.

## Run it

Rust half (in WSL, pinned toolchain — see `rust-toolchain.toml`):

```bash
cargo test -p solana-ml-dsa-program-tests --test fips204_vectors
```

JS half (Windows Node — there is no Node inside WSL). Resolves `@noble` from the
sibling `../ml-dsa-44/` sample's `node_modules` (tested against
`@noble/post-quantum` 0.6.1):

```bash
node programs/ml-dsa-tests/cross-impl/check_vectors.mjs
# or point at another @noble install:
NOBLE_MLDSA=/abs/path/to/@noble/post-quantum/ml-dsa.js node programs/ml-dsa-tests/cross-impl/check_vectors.mjs
```

The JS half (re)writes `cross_impl_external.json`; the Rust half reads it. Both are
committed, so each side also runs standalone.
