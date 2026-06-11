//! Microbenchmark for ML-DSA-44 signature *verification*, used to derive the
//! `ML_DSA_VERIFY_COST` compute-unit constant in
//! `cost-model/src/block_cost_limits.rs`.
//!
//! The cost model prices each precompile verification in "cluster microsecond
//! units" times `COMPUTE_UNIT_TO_US_RATIO`:
//!     ED25519_VERIFY_COST   = 30 * 76   (76 us-units on the cluster reference HW)
//!     SECP256K1_VERIFY_COST = 30 * 223
//! Those 76 / 223 figures are *cluster-averaged*, not this machine. So we cannot
//! simply paste a raw host measurement of ML-DSA verify — that would mix
//! reference frames and undercount it on a fast host. Instead we measure
//! ed25519 verify AND ML-DSA verify on the *same* host, take the ratio, and
//! scale the existing 76 us-unit ed25519 anchor by it:
//!     ml_dsa_us_units = 76 * (ml_dsa_verify_host / ed25519_verify_host)
//!     ML_DSA_VERIFY_COST = 30 * round(ml_dsa_us_units)
//! This keeps ML-DSA in the same units as the constants it sits beside and
//! expresses the ticket's "more compute-intensive than ed25519" as a measured
//! multiple.
//!
//! Run (from a WSL shell, on the pinned 1.76.0 toolchain):
//!     cargo run --release -p solana-ml-dsa-program-tests --example bench_verify

use {
    fips204::{
        ml_dsa_44,
        traits::{Signer as _, Verifier as _},
    },
    solana_sdk::signature::{Keypair, Signer as _},
    std::time::Instant,
};

// Mirror of `block_cost_limits::COMPUTE_UNIT_TO_US_RATIO`.
const COMPUTE_UNIT_TO_US_RATIO: f64 = 30.0;
// The cluster-averaged ed25519 verify cost, in us-units, that the existing
// `ED25519_VERIFY_COST` encodes (`block_cost_limits.rs`).
const ED25519_US_UNITS: f64 = 76.0;

// FIPS 204 context, empty to match the precompile (`ml_dsa_instruction.rs`).
const ML_DSA_CONTEXT: &[u8] = &[];

const WARMUP: u32 = 200;
const ITERS: u32 = 2000;

/// Time `f` over ITERS runs (after WARMUP) and return mean microseconds.
fn time_us(mut f: impl FnMut() -> bool) -> f64 {
    for _ in 0..WARMUP {
        assert!(std::hint::black_box(f()));
    }
    let start = Instant::now();
    let mut ok = 0u64;
    for _ in 0..ITERS {
        if std::hint::black_box(f()) {
            ok += 1;
        }
    }
    let elapsed = start.elapsed();
    assert_eq!(ok, ITERS as u64, "all verifications should succeed");
    elapsed.as_secs_f64() * 1e6 / f64::from(ITERS)
}

fn main() {
    let message = b"benchmark message for signature verification timing";

    // --- ed25519 (host reference) ---
    let ed_keypair = Keypair::new();
    let ed_pubkey = ed_keypair.pubkey();
    let ed_sig = ed_keypair.sign_message(message);
    assert!(ed_sig.verify(ed_pubkey.as_ref(), message));
    let ed25519_us = time_us(|| ed_sig.verify(ed_pubkey.as_ref(), message));

    // --- ML-DSA-44 ---
    let (ml_pub, ml_priv) = ml_dsa_44::try_keygen().expect("keygen");
    let ml_sig = ml_priv.try_sign(message, ML_DSA_CONTEXT).expect("sign");
    assert!(ml_pub.verify(message, &ml_sig, ML_DSA_CONTEXT));
    let ml_dsa_us = time_us(|| ml_pub.verify(message, &ml_sig, ML_DSA_CONTEXT));

    let ratio = ml_dsa_us / ed25519_us;
    let ml_dsa_us_units = ED25519_US_UNITS * ratio;
    let us_units = ml_dsa_us_units.round() as u64;
    // Derive CU from the rounded us-units (not the raw float) so the printed
    // "30 * {us_units} = {cu}" equation matches the pasted constant exactly.
    let cu = COMPUTE_UNIT_TO_US_RATIO as u64 * us_units;

    println!("--- signature verify benchmark (same host) ---");
    println!("iterations            : {ITERS} (after {WARMUP} warmup)");
    println!("ed25519 verify (host) : {ed25519_us:.3} us");
    println!("ML-DSA-44 verify(host): {ml_dsa_us:.3} us");
    println!("measured ratio        : {ratio:.2}x ed25519");
    println!();
    println!("ED25519_VERIFY_COST   = 30 * 76  = 2280 CU  (cluster anchor)");
    println!("SECP256K1_VERIFY_COST = 30 * 223 = 6690 CU  (reference)");
    println!("ML-DSA us-units       = 76 * {ratio:.2} = {ml_dsa_us_units:.1} -> {us_units}");
    println!("=> ML_DSA_VERIFY_COST = 30 * {us_units} = {cu} CU");
    println!();
    println!("Paste into cost-model/src/block_cost_limits.rs:");
    println!("    pub const ML_DSA_VERIFY_COST: u64 = COMPUTE_UNIT_TO_US_RATIO * {us_units};");
}
