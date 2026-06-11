//! ML-DSA-44 vs ed25519 **crypto baseline** — keygen, sign, and verify latency
//! measured on the same host, plus the key/signature size facts. This is the
//! reference baseline for EPIC 4-2 (benchmarks): the throughput harnesses
//! (bench-tps, banking-bench, perf sigverify) are dominated by these primitive
//! costs, and their reference comments cite the ratios printed here.
//!
//! It also derives the `ML_DSA_VERIFY_COST` compute-unit constant in
//! `cost-model/src/block_cost_limits.rs` (EPIC 3-2). The cost model prices each
//! precompile verification in "cluster microsecond units" times
//! `COMPUTE_UNIT_TO_US_RATIO`:
//!     ED25519_VERIFY_COST   = 30 * 76   (76 us-units on the cluster reference HW)
//!     SECP256K1_VERIFY_COST = 30 * 223
//! Those 76 / 223 figures are *cluster-averaged*, not this machine. So we cannot
//! paste a raw host measurement of ML-DSA verify — that would mix reference
//! frames. Instead we measure ed25519 AND ML-DSA verify on the *same* host, take
//! the ratio, and scale the 76 us-unit ed25519 anchor by it:
//!     ML_DSA_VERIFY_COST = 30 * round(76 * ml_dsa_verify / ed25519_verify)
//! The committed constant (177 = 5310 CU) is the MEDIAN of repeated runs; a single
//! run varies by a few us-units, so the figure this example prints is an estimate.
//!
//! Run (from a WSL shell, on the pinned 1.76.0 toolchain):
//!     cargo run --release -p solana-ml-dsa-program-tests --example bench_verify

use {
    fips204::{
        ml_dsa_44::{self, PK_LEN, SIG_LEN},
        traits::{Signer as _, Verifier as _},
    },
    solana_sdk::signature::{Keypair, Signer as _},
    std::{hint::black_box, time::Instant},
};

// Mirror of `block_cost_limits::COMPUTE_UNIT_TO_US_RATIO`.
const COMPUTE_UNIT_TO_US_RATIO: f64 = 30.0;
// The cluster-averaged ed25519 verify cost, in us-units, that the existing
// `ED25519_VERIFY_COST` encodes (`block_cost_limits.rs`).
const ED25519_US_UNITS: f64 = 76.0;

// FIPS 204 context, empty to match the precompile (`ml_dsa_instruction.rs`).
const ML_DSA_CONTEXT: &[u8] = &[];

/// Time `f` over `iters` runs (after `warmup`) and return mean microseconds.
fn bench_us(iters: u32, warmup: u32, mut f: impl FnMut()) -> f64 {
    for _ in 0..warmup {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    start.elapsed().as_secs_f64() * 1e6 / f64::from(iters)
}

fn main() {
    let message = b"benchmark message for ML-DSA-44 vs ed25519 baseline timing";

    // Shared key material (sign/verify reuse one keypair; keygen is timed separately).
    let ed_keypair = Keypair::new();
    let ed_pubkey = ed_keypair.pubkey();
    let ed_sig = ed_keypair.sign_message(message);
    assert!(ed_sig.verify(ed_pubkey.as_ref(), message));

    let (ml_pub, ml_priv) = ml_dsa_44::try_keygen().expect("keygen");
    let ml_sig = ml_priv.try_sign(message, ML_DSA_CONTEXT).expect("sign");
    assert!(ml_pub.verify(message, &ml_sig, ML_DSA_CONTEXT));

    // keygen + sign are slower (ML-DSA sign uses rejection sampling), so fewer iters.
    let (kg_iters, kg_warmup) = (500u32, 50u32);
    let (vf_iters, vf_warmup) = (2000u32, 200u32);

    let ed_keygen = bench_us(kg_iters, kg_warmup, || {
        black_box(Keypair::new());
    });
    let ml_keygen = bench_us(kg_iters, kg_warmup, || {
        black_box(ml_dsa_44::try_keygen().expect("keygen"));
    });
    let ed_sign = bench_us(kg_iters, kg_warmup, || {
        black_box(ed_keypair.sign_message(message));
    });
    let ml_sign = bench_us(kg_iters, kg_warmup, || {
        black_box(ml_priv.try_sign(message, ML_DSA_CONTEXT).expect("sign"));
    });
    let ed_verify = bench_us(vf_iters, vf_warmup, || {
        black_box(ed_sig.verify(ed_pubkey.as_ref(), message));
    });
    let ml_verify = bench_us(vf_iters, vf_warmup, || {
        black_box(ml_pub.verify(message, &ml_sig, ML_DSA_CONTEXT));
    });

    let row = |op: &str, ed: f64, ml: f64| {
        println!("{op:<8} {ed:>10.2} us {ml:>12.2} us   {:>5.1}x", ml / ed);
    };
    println!("--- ML-DSA-44 vs ed25519 crypto baseline (same host) ---");
    println!("{:<8} {:>13} {:>15}   {:>6}", "op", "ed25519", "ML-DSA-44", "ratio");
    row("keygen", ed_keygen, ml_keygen);
    row("sign", ed_sign, ml_sign);
    row("verify", ed_verify, ml_verify);
    println!();
    println!("sizes        ed25519     ML-DSA-44     ratio");
    println!(
        "public key   {:>5} B    {:>6} B     {:>4.0}x",
        32,
        PK_LEN,
        PK_LEN as f64 / 32.0
    );
    println!(
        "signature    {:>5} B    {:>6} B     {:>4.0}x",
        64,
        SIG_LEN,
        SIG_LEN as f64 / 64.0
    );
    println!();

    // --- cost-model derivation (verify ratio -> ML_DSA_VERIFY_COST), EPIC 3-2 ---
    let ratio = ml_verify / ed_verify;
    let ml_dsa_us_units = ED25519_US_UNITS * ratio;
    let us_units = ml_dsa_us_units.round() as u64;
    // Derive CU from the rounded us-units so "30 * {us_units} = {cu}" matches the constant.
    let cu = COMPUTE_UNIT_TO_US_RATIO as u64 * us_units;
    println!("--- cost-model derivation (verify), single-run estimate ---");
    println!("ED25519_VERIFY_COST   = 30 * 76  = 2280 CU  (cluster anchor)");
    println!("ML-DSA us-units       = 76 * {ratio:.2} = {ml_dsa_us_units:.1} -> {us_units}");
    println!("=> ML_DSA_VERIFY_COST = 30 * {us_units} = {cu} CU  (this run)");
    println!("   committed: 30 * 177 = 5310 CU (median of repeated runs; single runs vary)");
}
