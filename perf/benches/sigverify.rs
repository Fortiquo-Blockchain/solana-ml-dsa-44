#![feature(test)]

extern crate test;

use {
    log::*,
    rand::{thread_rng, Rng},
    solana_perf::{
        packet::{to_packet_batches, Packet, PacketBatch},
        recycler::Recycler,
        sigverify,
        test_tx::{ml_dsa_packet_batches, test_ml_dsa_tx_wire, test_multisig_tx, test_tx},
    },
    test::Bencher,
};

// --- ML-DSA-44 vs ed25519 sigverify pipeline baseline (EPIC 4-2) ---
// `bench_sigverify_simple` is the ed25519 baseline; `bench_sigverify_ml_dsa`
// (below) drives post-quantum ML-DSA-44 `0x00` packets through the SAME CPU
// verify pipeline (`sigverify::ed25519_verify` -> `verify_ml_dsa_packet`:
// deserialize + `sha256(pubkey)==account_key` binding + ML-DSA-44 verify), so it
// measures the real Phase-1 verify path, not the raw crypto primitive.
//
// These `#[bench]`es are nightly-only (`#![feature(test)]`) and the available
// nightly cannot build the 2.0-era dep tree (pre-existing: ahash 0.7.6 uses the
// removed `stdsimd` feature), so they are NOT part of the CI bench job
// (ci/test-bench.sh). For a stable, runnable measurement on the pinned 1.76
// toolchain, run:  cargo run --release -p solana-perf --example sigverify_ml_dsa
//
// Recorded pipeline baseline (release, build host, CPU verify path / no perf-libs;
// NUM=256 packets, 128/batch):
//   ed25519   verify: 18.24 us/packet
//   ML-DSA-44 verify: 43.69 us/packet   (~2.4x)
//   packet size: ed25519 183 B vs ML-DSA-44 3885 B (~21x)  (sig 2420 vs 64, pubkey 1312 vs 32)
// Re-run the example to refresh these figures on a given host.

const NUM: usize = 256;
const LARGE_BATCH_PACKET_COUNT: usize = 128;

#[bench]
fn bench_sigverify_simple(bencher: &mut Bencher) {
    let tx = test_tx();
    let num_packets = NUM;

    // generate packet vector
    let mut batches = to_packet_batches(
        &std::iter::repeat(tx).take(num_packets).collect::<Vec<_>>(),
        128,
    );

    let recycler = Recycler::default();
    let recycler_out = Recycler::default();
    // verify packets
    bencher.iter(|| {
        sigverify::ed25519_verify(&mut batches, &recycler, &recycler_out, false, num_packets);
    })
}

#[bench]
fn bench_sigverify_ml_dsa(bencher: &mut Bencher) {
    // Post-quantum ML-DSA-44 (`0x00`) packets through the SAME CPU verify pipeline
    // as `bench_sigverify_simple` (`ed25519_verify` -> `verify_ml_dsa_packet`).
    // One signed wire is reused across all packets (verify cost is per-packet and
    // independent of payload identity); see the module header for recorded numbers
    // and the stable runnable example.
    let wire = test_ml_dsa_tx_wire();
    let num_packets = NUM;
    let mut batches = ml_dsa_packet_batches(&wire, num_packets, 128);

    let recycler = Recycler::default();
    let recycler_out = Recycler::default();
    // verify packets
    bencher.iter(|| {
        sigverify::ed25519_verify(&mut batches, &recycler, &recycler_out, false, num_packets);
    })
}

fn gen_batches(
    use_same_tx: bool,
    packets_per_batch: usize,
    total_packets: usize,
) -> Vec<PacketBatch> {
    if use_same_tx {
        let tx = test_tx();
        to_packet_batches(&vec![tx; total_packets], packets_per_batch)
    } else {
        let txs: Vec<_> = std::iter::repeat_with(test_tx)
            .take(total_packets)
            .collect();
        to_packet_batches(&txs, packets_per_batch)
    }
}

#[bench]
#[ignore]
fn bench_sigverify_low_packets_small_batch(bencher: &mut Bencher) {
    let num_packets = sigverify::VERIFY_PACKET_CHUNK_SIZE - 1;
    let mut batches = gen_batches(false, 1, num_packets);
    let recycler = Recycler::default();
    let recycler_out = Recycler::default();
    bencher.iter(|| {
        sigverify::ed25519_verify(&mut batches, &recycler, &recycler_out, false, num_packets);
    })
}

#[bench]
#[ignore]
fn bench_sigverify_low_packets_large_batch(bencher: &mut Bencher) {
    let num_packets = sigverify::VERIFY_PACKET_CHUNK_SIZE - 1;
    let mut batches = gen_batches(false, LARGE_BATCH_PACKET_COUNT, num_packets);
    let recycler = Recycler::default();
    let recycler_out = Recycler::default();
    bencher.iter(|| {
        sigverify::ed25519_verify(&mut batches, &recycler, &recycler_out, false, num_packets);
    })
}

#[bench]
#[ignore]
fn bench_sigverify_medium_packets_small_batch(bencher: &mut Bencher) {
    let num_packets = sigverify::VERIFY_PACKET_CHUNK_SIZE * 8;
    let mut batches = gen_batches(false, 1, num_packets);
    let recycler = Recycler::default();
    let recycler_out = Recycler::default();
    bencher.iter(|| {
        sigverify::ed25519_verify(&mut batches, &recycler, &recycler_out, false, num_packets);
    })
}

#[bench]
#[ignore]
fn bench_sigverify_medium_packets_large_batch(bencher: &mut Bencher) {
    let num_packets = sigverify::VERIFY_PACKET_CHUNK_SIZE * 8;
    let mut batches = gen_batches(false, LARGE_BATCH_PACKET_COUNT, num_packets);
    let recycler = Recycler::default();
    let recycler_out = Recycler::default();
    bencher.iter(|| {
        sigverify::ed25519_verify(&mut batches, &recycler, &recycler_out, false, num_packets);
    })
}

#[bench]
#[ignore]
fn bench_sigverify_high_packets_small_batch(bencher: &mut Bencher) {
    let num_packets = sigverify::VERIFY_PACKET_CHUNK_SIZE * 32;
    let mut batches = gen_batches(false, 1, num_packets);
    let recycler = Recycler::default();
    let recycler_out = Recycler::default();
    bencher.iter(|| {
        sigverify::ed25519_verify(&mut batches, &recycler, &recycler_out, false, num_packets);
    })
}

#[bench]
#[ignore]
fn bench_sigverify_high_packets_large_batch(bencher: &mut Bencher) {
    let num_packets = sigverify::VERIFY_PACKET_CHUNK_SIZE * 32;
    let mut batches = gen_batches(false, LARGE_BATCH_PACKET_COUNT, num_packets);
    let recycler = Recycler::default();
    let recycler_out = Recycler::default();
    // verify packets
    bencher.iter(|| {
        sigverify::ed25519_verify(&mut batches, &recycler, &recycler_out, false, num_packets);
    })
}

#[bench]
#[ignore]
fn bench_sigverify_uneven(bencher: &mut Bencher) {
    solana_logger::setup();
    let simple_tx = test_tx();
    let multi_tx = test_multisig_tx();
    let mut tx;

    let num_packets = NUM * 50;
    let mut num_valid = 0;
    let mut current_packets = 0;
    // generate packet vector
    let mut batches = vec![];
    while current_packets < num_packets {
        let mut len: usize = thread_rng().gen_range(1..128);
        current_packets += len;
        if current_packets > num_packets {
            len -= current_packets - num_packets;
            current_packets = num_packets;
        }
        let mut batch = PacketBatch::with_capacity(len);
        batch.resize(len, Packet::default());
        for packet in batch.iter_mut() {
            if thread_rng().gen_ratio(1, 2) {
                tx = simple_tx.clone();
            } else {
                tx = multi_tx.clone();
            };
            Packet::populate_packet(packet, None, &tx).expect("serialize request");
            if thread_rng().gen_ratio((num_packets - NUM) as u32, num_packets as u32) {
                packet.meta_mut().set_discard(true);
            } else {
                num_valid += 1;
            }
        }
        batches.push(batch);
    }
    info!("num_packets: {} valid: {}", num_packets, num_valid);

    let recycler = Recycler::default();
    let recycler_out = Recycler::default();
    // verify packets
    bencher.iter(|| {
        sigverify::ed25519_verify(&mut batches, &recycler, &recycler_out, false, num_packets);
    })
}

#[bench]
fn bench_get_offsets(bencher: &mut Bencher) {
    let tx = test_tx();

    // generate packet vector
    let mut batches =
        to_packet_batches(&std::iter::repeat(tx).take(1024).collect::<Vec<_>>(), 1024);

    let recycler = Recycler::default();
    // verify packets
    bencher.iter(|| {
        let _ans = sigverify::generate_offsets(&mut batches, &recycler, false);
    })
}
