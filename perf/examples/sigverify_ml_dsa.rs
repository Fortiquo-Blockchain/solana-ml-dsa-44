//! Stable, runnable ML-DSA-44 vs ed25519 sigverify **pipeline** baseline (EPIC 4-2).
//!
//! The nightly `#[bench]` harness (`perf/benches/sigverify.rs`) cannot run on the
//! available nightly — pre-existing: `ahash 0.7.6` uses the removed `stdsimd`
//! feature — so this example measures the SAME signal on the pinned 1.76 stable
//! toolchain. It drives the real CPU verify pipeline
//! `solana_perf::sigverify::ed25519_verify` over a batch of ed25519 packets and
//! over a batch of replay-safe post-quantum ML-DSA-44 envelope packets, and
//! reports the per-packet verify latency and the ML-DSA / ed25519 ratio.
//!
//! Unlike `programs/ml-dsa-tests/examples/bench_verify.rs` (which times the raw
//! `fips204` primitives), this exercises the actual packet path:
//! `verify_packet` -> `verify_ml_dsa_envelope_packet` (deserialize, carrier
//! precompile ML-DSA-44 verify + `sha256(pubkey) == signer` binding + anti-lift)
//! inside the batched pipeline — the same code a validator runs at TPU ingress.
//!
//! The ratio is apples-to-apples only on a host WITHOUT perf-libs/CUDA loaded
//! (the default for `cargo run` and `solana-test-validator`): then both schemes
//! use the CPU verify path. With perf-libs loaded, ed25519 would use the GPU
//! kernel while ML-DSA envelope packets have no cheap GPU-diversion marker and are
//! only verified when the batch falls to the CPU (see remaining-work B2), so the
//! two would not be directly comparable.
//!
//! Run (WSL, pinned 1.76):
//!     cargo run --release -p solana-perf --example sigverify_ml_dsa

use {
    solana_perf::{
        packet::{to_packet_batches, PacketBatch},
        recycler::Recycler,
        sigverify,
        test_tx::{ml_dsa_packet_batches, test_ml_dsa_tx_wire, test_tx},
    },
    std::time::Instant,
};

const NUM_PACKETS: usize = 256;
const PACKETS_PER_BATCH: usize = 128;
const ITERS: u32 = 50;
const WARMUP: u32 = 5;

/// Mean per-packet verify latency (microseconds) of `ed25519_verify` over
/// `batches`. All packets are valid, so none are discarded and every iteration
/// re-verifies the full set.
fn per_packet_us(batches: &mut [PacketBatch], num_packets: usize) -> f64 {
    let recycler = Recycler::default();
    let recycler_out = Recycler::default();
    for _ in 0..WARMUP {
        sigverify::ed25519_verify(batches, &recycler, &recycler_out, false, num_packets);
    }
    let start = Instant::now();
    for _ in 0..ITERS {
        sigverify::ed25519_verify(batches, &recycler, &recycler_out, false, num_packets);
    }
    let total_us = start.elapsed().as_secs_f64() * 1e6;
    total_us / (f64::from(ITERS) * num_packets as f64)
}

/// First packet's on-wire size in a batch set, for reporting.
fn first_packet_size(batches: &[PacketBatch]) -> usize {
    batches
        .first()
        .and_then(|b| b.iter().next())
        .map(|p| p.meta().size)
        .unwrap_or(0)
}

fn main() {
    // ed25519 workload: one transfer tx per packet (matches `bench_sigverify_simple`).
    let ed_tx = test_tx();
    let mut ed_batches = to_packet_batches(
        &std::iter::repeat(ed_tx).take(NUM_PACKETS).collect::<Vec<_>>(),
        PACKETS_PER_BATCH,
    );

    // ML-DSA workload: one signed `0x00` post-quantum transfer, repeated per packet
    // (verify cost is per-packet and independent of payload identity).
    let wire = test_ml_dsa_tx_wire();
    let mut ml_batches = ml_dsa_packet_batches(&wire, NUM_PACKETS, PACKETS_PER_BATCH);

    let ed_size = first_packet_size(&ed_batches);
    let ml_size = first_packet_size(&ml_batches);

    let ed_us = per_packet_us(&mut ed_batches, NUM_PACKETS);
    let ml_us = per_packet_us(&mut ml_batches, NUM_PACKETS);

    println!("--- ML-DSA-44 vs ed25519 sigverify PIPELINE baseline (same host, release) ---");
    println!("packets: {NUM_PACKETS} ({PACKETS_PER_BATCH}/batch), iters: {ITERS} (+{WARMUP} warmup)");
    println!();
    println!("{:<10} {:>14} {:>14}   {:>6}", "op", "ed25519", "ML-DSA-44", "ratio");
    println!(
        "{:<10} {:>11.3} us {:>11.3} us   {:>5.1}x",
        "verify", ed_us, ml_us, ml_us / ed_us
    );
    println!();
    println!("{:<10} {:>14} {:>14}   {:>6}", "size", "ed25519", "ML-DSA-44", "ratio");
    println!(
        "{:<10} {:>12} B {:>12} B   {:>5.0}x",
        "packet", ed_size, ml_size, ml_size as f64 / ed_size as f64
    );
    println!("{:<10} {:>12} B {:>12} B   {:>5.0}x", "signature", 64, 2420, 2420.0 / 64.0);
    println!("{:<10} {:>12} B {:>12} B   {:>5.0}x", "public key", 32, 1312, 1312.0 / 32.0);
}
