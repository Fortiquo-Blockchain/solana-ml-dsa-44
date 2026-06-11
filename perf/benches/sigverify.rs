#![feature(test)]

extern crate test;

use {
    log::*,
    rand::{thread_rng, Rng},
    solana_perf::{
        packet::{to_packet_batches, Packet, PacketBatch},
        recycler::Recycler,
        sigverify,
        test_tx::{test_multisig_tx, test_tx},
    },
    test::Bencher,
};

// --- ML-DSA-44 vs ed25519 sigverify baseline (EPIC 4-2) ---
// The benches below sign with ed25519 (`test_tx`). ML-DSA-44 packets (the 0x00
// wire) verify on the CPU verify path (`verify_packet`, reached via
// `ed25519_verify`) at a measured ~2.3x the per-signature cost of ed25519 verify,
// and are 38x larger (2420 B sig vs 64 B). See the runnable, stable baseline
// `programs/ml-dsa-tests/examples/bench_verify.rs` for the point-in-time host
// figures. A dedicated
// ML-DSA `#[bench]` is NOT added here: these benches are nightly-only
// (`#![feature(test)]`), are not part of the CI bench job (ci/test-bench.sh), and
// the available nightly cannot build the 2.0-era dep tree (ahash 0.7.6 uses the
// removed `stdsimd` feature).

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
