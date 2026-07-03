//! NIST FIPS 204 (ML-DSA-44) known-answer & cross-implementation conformance tests.
//!
//! Why this exists: the whole fork rests on the claim that our signatures really
//! are FIPS 204. `docs/ml-dsa-44/strategy.md` §8 flagged "must match the reference
//! implementation exactly" as a High risk whose mitigation ("a shared
//! cross-implementation test") was still open. This locks it down with an
//! authoritative answer key, so a future `fips204` bump or a wiring slip fails
//! loudly instead of silently shipping a FIPS-204 lookalike.
//!
//! What is pinned, and to what:
//!  * Key generation -> **NIST gold**, on our real API. `MlDsaKeypair::from_seed`
//!    must reproduce, byte-for-byte, the public+secret key NIST derives from each
//!    seed.
//!  * Signing / verification -> **NIST gold**, on the `fips204` core we delegate
//!    to. The ACVP vectors here are the *internal* interface (they sign the raw
//!    message), so they are checked through `fips204`'s `_internal_sign` /
//!    `_internal_verify` — exactly as `fips204`'s own conformance test does.
//!    (Those two are `#[deprecated]` upstream and marked "will be removed" once
//!    NIST publishes external-interface vectors; a major `fips204` bump may then
//!    require rehosting these byte checks on the external API.)
//!  * Our external path (empty-context `MlDsaKeypair::sign`) -> round-trips and
//!    rejects tampering. ML-DSA signing is *hedged* (randomized), so its bytes
//!    are not reproducible; we assert verify-accepts, not byte-equality.
//!  * Wallet(JS) -> validator(Rust) interop -> the validator's external verify
//!    accepts a deterministic external signature produced by `@noble/post-quantum`
//!    (the sibling JS sample), and the two implementations agree byte-for-byte.
//!    The JS half lives in `../cross-impl/check_vectors.mjs`; it also re-checks
//!    keygen against the same NIST fixture and emits `cross_impl_external.json`.
//!
//! Vectors: `tests/vectors/ml_dsa_44_kat.json`, an excerpt of the official NIST
//! ACVP FIPS 204 vectors (vendored in `fips204` 0.4.6; see the file header for
//! the upstream commit and the regeneration script).
//!
//! (The ml_dsa_* SDK modules are gated behind solana-sdk's `full` feature, which
//! is in its default set, so this downstream test crate gets them with no extra
//! feature wiring — no `#![cfg(feature = "full")]` gate here, which would refer to
//! a `full` feature this test crate does not have and silently skip every test.)

use {
    fips204::{
        ml_dsa_44::{self, PrivateKey, PublicKey},
        traits::{SerDes, Signer},
    },
    serde_json::Value,
    solana_sdk::{
        ml_dsa_keypair::{ml_dsa_address, MlDsaKeypair},
        ml_dsa_public_key::MlDsaPublicKey,
    },
};

const KAT: &str = include_str!("vectors/ml_dsa_44_kat.json");
const CROSS_IMPL: &str = include_str!("vectors/cross_impl_external.json");

const PK_LEN: usize = 1312;
const SK_LEN: usize = 2560;
const SIG_LEN: usize = 2420;

fn hexd(v: &Value, key: &str) -> Vec<u8> {
    hex::decode(v[key].as_str().unwrap_or_else(|| panic!("missing/!string field {key}")))
        .unwrap_or_else(|_| panic!("field {key} is not valid hex"))
}

fn fixed<const N: usize>(bytes: Vec<u8>, what: &str) -> [u8; N] {
    bytes
        .try_into()
        .unwrap_or_else(|v: Vec<u8>| panic!("{what}: expected {N} bytes, got {}", v.len()))
}

fn kat() -> Value {
    serde_json::from_str(KAT).expect("ml_dsa_44_kat.json parses")
}

/// Secret-key half of an `MlDsaKeypair` (`to_bytes()` is `[public(1312) || secret(2560)]`).
fn secret_of(kp: &MlDsaKeypair) -> [u8; SK_LEN] {
    fixed(kp.to_bytes()[PK_LEN..].to_vec(), "keypair secret half")
}

/// (A) Key generation, against the NIST answer key, on our real signing API.
///
/// FIPS 204 KeyGen is deterministic in its 32-byte seed (ξ), so this is an exact
/// byte match: our `from_seed` must produce NIST's public AND secret key.
#[test]
fn keygen_matches_nist_gold() {
    let v = kat();
    let cases = v["keygen"].as_array().expect("keygen array");
    assert!(cases.len() >= 3, "expected >=3 keygen vectors, got {}", cases.len());
    for t in cases {
        let seed: [u8; 32] = fixed(hexd(t, "seed"), "seed");
        let pk_exp = hexd(t, "pk");
        let sk_exp = hexd(t, "sk");

        let kp = MlDsaKeypair::from_seed(&seed);
        assert_eq!(
            &kp.public_key_bytes()[..],
            pk_exp.as_slice(),
            "public key mismatch vs NIST, tcId {}",
            t["tcId"]
        );
        assert_eq!(
            &secret_of(&kp)[..],
            sk_exp.as_slice(),
            "secret key mismatch vs NIST, tcId {}",
            t["tcId"]
        );
        // The on-chain address binding stays sha256(pubkey).
        assert_eq!(kp.address(), ml_dsa_address(&pk_exp));
    }
}

/// (B1) Signature generation, against the NIST answer key, on the `fips204` core.
///
/// These are the deterministic (rnd = 0) ACVP sigGen vectors, internal interface
/// (raw message). Verified through `_internal_sign`, mirroring `fips204`'s own
/// NIST conformance test (the API is `#[deprecated]` but public for exactly this).
#[test]
fn internal_sign_matches_nist_gold() {
    let v = kat();
    let cases = v["siggen_det"].as_array().expect("siggen_det array");
    assert!(cases.len() >= 2, "expected >=2 deterministic siggen vectors, got {}", cases.len());
    for t in cases {
        let sk: [u8; SK_LEN] = fixed(hexd(t, "sk"), "sk");
        let message = hexd(t, "message");
        let sig_exp = hexd(t, "signature");

        let sk = PrivateKey::try_from_bytes(sk).expect("sk parses");
        #[allow(deprecated)]
        let sig = ml_dsa_44::_internal_sign(&sk, &message, &[], [0u8; 32]).expect("sign");
        assert_eq!(
            &sig[..],
            sig_exp.as_slice(),
            "signature mismatch vs NIST, tcId {}",
            t["tcId"]
        );
    }
}

/// (B2) Signature verification, against the NIST answer key (valid AND invalid).
///
/// ACVP sigVer vectors carry a `testPassed` flag; our verdict must match it on
/// both accepted and deliberately-corrupted signatures.
#[test]
fn internal_verify_matches_nist_gold() {
    let v = kat();
    let cases = v["sigver"].as_array().expect("sigver array");
    assert!(cases.len() >= 4, "expected >=4 sigver vectors, got {}", cases.len());
    let (mut saw_pass, mut saw_fail, mut checked) = (false, false, 0usize);
    for t in cases {
        let pk: [u8; PK_LEN] = fixed(hexd(t, "pk"), "pk");
        let message = hexd(t, "message");
        let sig: [u8; SIG_LEN] = fixed(hexd(t, "signature"), "signature");
        let expected = t["testPassed"].as_bool().expect("testPassed bool");

        let pk = PublicKey::try_from_bytes(pk).expect("pk parses");
        #[allow(deprecated)]
        let got = ml_dsa_44::_internal_verify(&pk, &message, &sig, &[]);
        assert_eq!(
            got,
            expected,
            "verify verdict mismatch vs NIST, tcId {} ({})",
            t["tcId"],
            t["reason"]
        );
        saw_pass |= expected;
        saw_fail |= !expected;
        checked += 1;
    }
    // Guard against a refactor that skips the per-case verify yet still "passes".
    assert_eq!(checked, cases.len(), "every sigver case must be verified");
    assert!(saw_pass && saw_fail, "fixture must cover a valid AND an invalid sigVer case");
}

/// (C) Our actual product path: hedged external sign, then verify; tampering fails.
///
/// Exercises `MlDsaKeypair::sign` (empty FIPS 204 context, randomized) and both
/// the keypair's and the standalone `MlDsaPublicKey`'s verify.
#[test]
fn our_external_sign_round_trips() {
    let v = kat();
    let cases = v["keygen"].as_array().expect("keygen array");
    assert!(!cases.is_empty(), "no keygen vectors");
    let seed: [u8; 32] = fixed(hexd(&cases[0], "seed"), "seed");
    let kp = MlDsaKeypair::from_seed(&seed);
    let pk = MlDsaPublicKey::from(*kp.public_key_bytes());

    // Cover the empty message (where the external M' = 0x00 || 0x00 || M prefix is
    // most fragile), a typical message, and a large multi-block message.
    let big = vec![0xABu8; 10_000];
    for msg in [
        b"".as_slice(),
        b"phase1 round-trip via our hedged external sign".as_slice(),
        big.as_slice(),
    ] {
        let sig = kp.sign(msg).expect("sign");
        assert!(kp.verify(msg, &sig), "keypair must verify its own signature");
        assert!(pk.verify(msg, &sig), "MlDsaPublicKey must verify the same signature");

        // Tampering the message must fail.
        let mut tampered = msg.to_vec();
        tampered.push(0x01);
        assert!(!kp.verify(&tampered, &sig), "tampered message must fail");

        // A different (well-sized but garbage) public key must reject a valid signature.
        assert!(
            !MlDsaPublicKey::from([0xFFu8; PK_LEN]).verify(msg, &sig),
            "a wrong public key must reject the signature"
        );
    }
    assert_eq!(pk.address(), kp.address());
}

/// (D) Cross-implementation interop: a JS wallet's signature is accepted by the
/// Rust validator, and the two implementations agree byte-for-byte.
///
/// `cross_impl_external.json` is produced by `../cross-impl/check_vectors.mjs`
/// using `@noble/post-quantum` deterministic external signing (empty context) —
/// the exact path a Phase-1 wallet uses. We (1) verify it through our real
/// `MlDsaPublicKey::verify`, and (2) reproduce the same bytes via FIPS 204's
/// external->internal message formatting `M' = 0x00 || 0x00 || M`.
#[test]
fn external_interop_with_noble_js() {
    let v = kat();
    let xi = &v["external_interop"];
    assert!(xi.is_object(), "external_interop block missing from fixture");
    let seed: [u8; 32] = fixed(hexd(xi, "seed"), "seed");
    let pk_bytes: [u8; PK_LEN] = fixed(hexd(xi, "pk"), "pk");
    let xi_message = xi["message_utf8"].as_str().expect("external_interop.message_utf8");
    let message = xi_message.as_bytes();

    let cross: Value = serde_json::from_str(CROSS_IMPL).expect("cross_impl_external.json parses");
    // Unwrap both sides so a dropped field is a hard error, not a silent None == None.
    let cross_message = cross["message_utf8"].as_str().expect("cross artifact: message_utf8");
    assert_eq!(
        cross_message, xi_message,
        "cross-impl artifact is for a different message; regenerate check_vectors.mjs"
    );
    let js_sig: [u8; SIG_LEN] = fixed(hexd(&cross, "ext_sig_det"), "ext_sig_det");

    // (1) Product path: the validator's external verify accepts the JS signature,
    //     and rejects it against a tampered message.
    let pk = MlDsaPublicKey::from(pk_bytes);
    assert!(
        pk.verify(message, &js_sig),
        "Rust external verify must accept @noble's external signature (wallet->validator interop)"
    );
    assert!(
        !pk.verify(b"a different message", &js_sig),
        "JS signature must NOT verify against a different message"
    );

    // (2) Byte-for-byte: Rust deterministic external sign == JS deterministic external sign.
    let kp = MlDsaKeypair::from_seed(&seed);
    assert_eq!(&kp.public_key_bytes()[..], &pk_bytes[..], "seed must derive the fixture's pk");
    let sk = PrivateKey::try_from_bytes(secret_of(&kp)).expect("sk parses");
    let mut m_prime = vec![0u8, 0u8]; // 0x00 || len(ctx)=0 for the empty context
    m_prime.extend_from_slice(message);
    #[allow(deprecated)]
    let rust_sig = ml_dsa_44::_internal_sign(&sk, &m_prime, &[], [0u8; 32]).expect("sign");
    assert_eq!(
        &rust_sig[..],
        &js_sig[..],
        "Rust and @noble deterministic external signatures must be byte-identical"
    );
}

/// (E) The empty-context invariant. The whole design signs/verifies with an EMPTY
/// FIPS 204 context (CLAUDE.md) so the Rust validator and JS wallet stay
/// byte-compatible. This locks the boundary: a signature made with a NON-empty
/// context must be REJECTED by our empty-context verify, so the two can never be
/// accidentally interoperable.
#[test]
fn non_empty_context_is_rejected() {
    let v = kat();
    let cases = v["keygen"].as_array().expect("keygen array");
    assert!(!cases.is_empty(), "no keygen vectors");
    let seed: [u8; 32] = fixed(hexd(&cases[0], "seed"), "seed");
    let kp = MlDsaKeypair::from_seed(&seed);
    let pk = MlDsaPublicKey::from(*kp.public_key_bytes());

    let msg = b"empty-context invariant";
    // Sign the same message with a NON-empty context via fips204's external signer.
    let sk = PrivateKey::try_from_bytes(secret_of(&kp)).expect("sk parses");
    let sig_ctx = sk.try_sign(msg, b"ctx").expect("sign with context");

    assert!(
        !pk.verify(msg, &sig_ctx),
        "a non-empty-context signature must NOT verify under empty-context verify"
    );
    // Sanity: the same key's empty-context signature still verifies.
    assert!(pk.verify(msg, &kp.sign(msg).expect("sign")), "empty-context signature must verify");
}
