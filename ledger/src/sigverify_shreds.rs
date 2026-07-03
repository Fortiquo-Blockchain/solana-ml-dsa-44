#![allow(clippy::implicit_hasher)]
use {
    crate::shred::{self, SIZE_OF_MERKLE_ROOT},
    itertools::{izip, Itertools},
    rayon::{prelude::*, ThreadPool},
    sha2::{Digest, Sha512},
    solana_metrics::{inc_new_counter_debug, inc_new_counter_info, inc_new_counter_warn},
    solana_perf::{
        cuda_runtime::PinnedVec,
        packet::{Packet, PacketBatch},
        perf_libs,
        recycler_cache::RecyclerCache,
        sigverify::{self, count_packets_in_batches, TxOffset},
    },
    solana_sdk::{
        clock::Slot,
        hash::Hash,
        ml_dsa_keypair::{ml_dsa_address, MlDsaKeypair},
        ml_dsa_public_key::MlDsaPublicKey,
        pubkey::Pubkey,
        signature::{Keypair, Signature, Signer},
    },
    std::{collections::HashMap, iter::repeat, mem::size_of, ops::Range, sync::Arc},
};

const SIGN_SHRED_GPU_MIN: usize = 256;

#[must_use]
pub fn verify_shred_cpu(packet: &Packet, slot_leaders: &HashMap<Slot, Pubkey>) -> bool {
    if packet.meta().discard() {
        return false;
    }
    let Some(shred) = shred::layout::get_shred(packet) else {
        return false;
    };
    let Some(slot) = shred::layout::get_slot(shred) else {
        return false;
    };
    trace!("slot {}", slot);
    let Some(pubkey) = slot_leaders.get(&slot) else {
        return false;
    };
    let Some(signature) = shred::layout::get_signature(shred) else {
        return false;
    };
    trace!("signature {}", signature);
    let Some(data) = shred::layout::get_signed_data(shred) else {
        return false;
    };
    signature.verify(pubkey.as_ref(), data.as_ref())
}

// fork (Phase 3): additive post-quantum verification of an ML-DSA-44 shred. A
// shred is a valid post-quantum shred iff:
//   (a) the Ed25519 signature authenticates the Merkle root (and thus the
//       commitment hashed into it) against the slot leader, exactly as the
//       Ed25519 liveness path checks;
//   (b) the trailer public key is the one the leader committed to:
//       ml_dsa_address(pubkey) == the in-root commitment; and
//   (c) the ML-DSA-44 signature over the Merkle root verifies under that key.
// This is a NON-GATING audit (telemetry only) — the Ed25519 path gates liveness;
// ml_dsa verification never discards a shred in v1. Returns false (not panics)
// for non-ml_dsa or malformed shreds.
#[must_use]
pub fn verify_shred_ml_dsa_cpu(packet: &Packet, slot_leaders: &HashMap<Slot, Pubkey>) -> bool {
    if packet.meta().discard() {
        return false;
    }
    let Some(shred) = shred::layout::get_shred(packet) else {
        return false;
    };
    if !shred::layout::is_ml_dsa_shred(shred) {
        return false;
    }
    let Some(slot) = shred::layout::get_slot(shred) else {
        return false;
    };
    let Some(pubkey) = slot_leaders.get(&slot) else {
        return false;
    };
    let Some(signature) = shred::layout::get_signature(shred) else {
        return false;
    };
    let Some(data) = shred::layout::get_signed_data(shred) else {
        return false;
    };
    // (a) Ed25519 authenticates the Merkle root (which commits to the ml_dsa key).
    if !signature.verify(pubkey.as_ref(), data.as_ref()) {
        return false;
    }
    let merkle_root = data.as_ref();
    let Some((commitment, ml_dsa_pubkey, ml_dsa_signature)) =
        shred::layout::get_ml_dsa_regions(shred)
    else {
        return false;
    };
    let Ok(ml_dsa_pubkey) = <[u8; MlDsaKeypair::PUBLIC_KEY_LENGTH]>::try_from(ml_dsa_pubkey) else {
        return false;
    };
    // (b) bind the trailer key to the leader via the in-root commitment.
    if ml_dsa_address(&ml_dsa_pubkey).as_ref() != commitment {
        return false;
    }
    // (c) the post-quantum signature over the Merkle root verifies.
    let Ok(ml_dsa_signature) = <[u8; MlDsaKeypair::SIGNATURE_LENGTH]>::try_from(ml_dsa_signature)
    else {
        return false;
    };
    MlDsaPublicKey::from(ml_dsa_pubkey).verify(merkle_root, &ml_dsa_signature)
}

// fork (Phase 3): non-gating audit pass — verify every ml_dsa shred in the
// batches and emit ok/fail telemetry. Does NOT mark any packet discard; the
// Ed25519 pass is the liveness gate. Returns (ok, fail) counts (also for tests).
pub fn audit_ml_dsa_shreds(
    batches: &[PacketBatch],
    slot_leaders: &HashMap<Slot, Pubkey>,
) -> (usize, usize) {
    let (mut ok, mut fail) = (0usize, 0usize);
    for batch in batches {
        for packet in batch.iter() {
            if packet.meta().discard() {
                continue;
            }
            let Some(shred) = shred::layout::get_shred(packet) else {
                continue;
            };
            if !shred::layout::is_ml_dsa_shred(shred) {
                continue;
            }
            if verify_shred_ml_dsa_cpu(packet, slot_leaders) {
                ok += 1;
            } else {
                fail += 1;
            }
        }
    }
    if ok > 0 {
        inc_new_counter_info!("ml_dsa_shred_verify_ok", ok);
    }
    if fail > 0 {
        inc_new_counter_warn!("ml_dsa_shred_verify_fail", fail);
    }
    (ok, fail)
}

// fork (Phase 3): GATING variant of audit_ml_dsa_shreds, used only under the
// opt-in --ml-dsa-shred-strict flag. Same per-shred verification, but DROPS
// (set_discard) any ml_dsa shred that fails. Non-ml_dsa packets are untouched
// (their Ed25519 gate already ran). Default validators use the non-gating audit
// pass; strict mode is opt-in so an ml_dsa verification gap can never stall the
// node unless the operator explicitly asks to enforce it. Returns (ok, fail).
pub fn enforce_ml_dsa_shreds(
    batches: &mut [PacketBatch],
    slot_leaders: &HashMap<Slot, Pubkey>,
) -> (usize, usize) {
    let (mut ok, mut fail) = (0usize, 0usize);
    for batch in batches.iter_mut() {
        for packet in batch.iter_mut() {
            if packet.meta().discard() {
                continue;
            }
            // Drop the get_shred borrow before the mutable set_discard below.
            let is_ml_dsa = shred::layout::get_shred(packet)
                .map(shred::layout::is_ml_dsa_shred)
                .unwrap_or(false);
            if !is_ml_dsa {
                continue;
            }
            if verify_shred_ml_dsa_cpu(packet, slot_leaders) {
                ok += 1;
            } else {
                fail += 1;
                packet.meta_mut().set_discard(true);
            }
        }
    }
    if ok > 0 {
        inc_new_counter_info!("ml_dsa_shred_verify_ok", ok);
    }
    if fail > 0 {
        inc_new_counter_warn!("ml_dsa_shred_verify_fail", fail);
        // Strict mode dropped these; a distinct counter makes the enforcement
        // action observable separately from the advisory-only fail count (which
        // audit_ml_dsa_shreds also emits without discarding).
        inc_new_counter_warn!("ml_dsa_shred_dropped", fail);
    }
    (ok, fail)
}

fn verify_shreds_cpu(
    thread_pool: &ThreadPool,
    batches: &[PacketBatch],
    slot_leaders: &HashMap<Slot, Pubkey>,
) -> Vec<Vec<u8>> {
    let packet_count = count_packets_in_batches(batches);
    debug!("CPU SHRED ECDSA for {}", packet_count);
    let rv = thread_pool.install(|| {
        batches
            .into_par_iter()
            .map(|batch| {
                batch
                    .par_iter()
                    .map(|packet| u8::from(verify_shred_cpu(packet, slot_leaders)))
                    .collect()
            })
            .collect()
    });
    inc_new_counter_debug!("ed25519_shred_verify_cpu", packet_count);
    rv
}

fn slot_key_data_for_gpu(
    thread_pool: &ThreadPool,
    batches: &[PacketBatch],
    slot_keys: &HashMap<Slot, Pubkey>,
    recycler_cache: &RecyclerCache,
) -> (/*pubkeys:*/ PinnedVec<u8>, TxOffset) {
    //TODO: mark Pubkey::default shreds as failed after the GPU returns
    assert_eq!(slot_keys.get(&Slot::MAX), Some(&Pubkey::default()));
    let slots: Vec<Slot> = thread_pool.install(|| {
        batches
            .into_par_iter()
            .flat_map_iter(|batch| {
                batch.iter().map(|packet| {
                    if packet.meta().discard() {
                        return Slot::MAX;
                    }
                    let shred = shred::layout::get_shred(packet);
                    match shred.and_then(shred::layout::get_slot) {
                        Some(slot) if slot_keys.contains_key(&slot) => slot,
                        _ => Slot::MAX,
                    }
                })
            })
            .collect()
    });
    let keys_to_slots: HashMap<Pubkey, Vec<Slot>> = slots
        .iter()
        .map(|slot| (slot_keys[slot], *slot))
        .into_group_map();
    let mut keyvec = recycler_cache.buffer().allocate("shred_gpu_pubkeys");
    keyvec.set_pinnable();

    let keyvec_size = keys_to_slots.len() * size_of::<Pubkey>();
    resize_buffer(&mut keyvec, keyvec_size);

    let key_offsets: HashMap<Slot, /*key offset:*/ usize> = {
        let mut next_offset = 0;
        keys_to_slots
            .into_iter()
            .flat_map(|(key, slots)| {
                let offset = next_offset;
                next_offset += std::mem::size_of::<Pubkey>();
                keyvec[offset..next_offset].copy_from_slice(key.as_ref());
                slots.into_iter().zip(repeat(offset))
            })
            .collect()
    };
    let mut offsets = recycler_cache.offsets().allocate("shred_offsets");
    offsets.set_pinnable();
    for slot in slots {
        offsets.push(key_offsets[&slot] as u32);
    }
    trace!("keyvec.len: {}", keyvec.len());
    trace!("keyvec: {:?}", keyvec);
    trace!("offsets: {:?}", offsets);
    (keyvec, offsets)
}

// Recovers merkle roots from shreds binary.
fn get_merkle_roots(
    thread_pool: &ThreadPool,
    packets: &[PacketBatch],
    recycler_cache: &RecyclerCache,
) -> (
    PinnedVec<u8>,      // Merkle roots
    Vec<Option<usize>>, // Offsets
) {
    let merkle_roots: Vec<Option<Hash>> = thread_pool.install(|| {
        packets
            .par_iter()
            .flat_map(|packets| {
                packets.par_iter().map(|packet| {
                    if packet.meta().discard() {
                        return None;
                    }
                    let shred = shred::layout::get_shred(packet)?;
                    shred::layout::get_merkle_root(shred)
                })
            })
            .collect()
    });
    let num_merkle_roots = merkle_roots.iter().flatten().count();
    let mut buffer = recycler_cache.buffer().allocate("shred_gpu_merkle_roots");
    buffer.set_pinnable();
    resize_buffer(&mut buffer, num_merkle_roots * SIZE_OF_MERKLE_ROOT);
    let offsets = {
        let mut next_offset = 0;
        merkle_roots
            .into_iter()
            .map(|root| {
                let root = root?;
                let offset = next_offset;
                next_offset += SIZE_OF_MERKLE_ROOT;
                buffer[offset..next_offset].copy_from_slice(root.as_ref());
                Some(offset)
            })
            .collect()
    };
    (buffer, offsets)
}

// Resizes the buffer to >= size and a multiple of
// std::mem::size_of::<Packet>().
fn resize_buffer(buffer: &mut PinnedVec<u8>, size: usize) {
    //HACK: Pubkeys vector is passed along as a `PacketBatch` buffer to the GPU
    //TODO: GPU needs a more opaque interface, which can handle variable sized structures for data
    //Pad the Pubkeys buffer such that it is bigger than a buffer of Packet sized elems
    let num_packets = (size + std::mem::size_of::<Packet>() - 1) / std::mem::size_of::<Packet>();
    let size = num_packets * std::mem::size_of::<Packet>();
    buffer.resize(size, 0u8);
}

fn elems_from_buffer(buffer: &PinnedVec<u8>) -> perf_libs::Elems {
    // resize_buffer ensures that buffer size is a multiple of Packet size.
    debug_assert_eq!(buffer.len() % std::mem::size_of::<Packet>(), 0);
    let num_packets = buffer.len() / std::mem::size_of::<Packet>();
    perf_libs::Elems {
        elems: buffer.as_ptr().cast::<u8>(),
        num: num_packets as u32,
    }
}

fn shred_gpu_offsets(
    offset: usize,
    batches: &[PacketBatch],
    merkle_roots_offsets: impl IntoIterator<Item = Option<usize>>,
    recycler_cache: &RecyclerCache,
) -> (TxOffset, TxOffset, TxOffset) {
    fn add_offset(range: Range<usize>, offset: usize) -> Range<usize> {
        range.start + offset..range.end + offset
    }
    let mut signature_offsets = recycler_cache.offsets().allocate("shred_signatures");
    signature_offsets.set_pinnable();
    let mut msg_start_offsets = recycler_cache.offsets().allocate("shred_msg_starts");
    msg_start_offsets.set_pinnable();
    let mut msg_sizes = recycler_cache.offsets().allocate("shred_msg_sizes");
    msg_sizes.set_pinnable();
    let offsets = std::iter::successors(Some(offset), |offset| {
        offset.checked_add(std::mem::size_of::<Packet>())
    });
    let packets = batches.iter().flatten();
    for (offset, packet, merkle_root_offset) in izip!(offsets, packets, merkle_roots_offsets) {
        let sig = shred::layout::get_signature_range();
        let sig = add_offset(sig, offset);
        debug_assert_eq!(sig.end - sig.start, std::mem::size_of::<Signature>());
        // Signature may verify for an empty message but the packet will be
        // discarded during deserialization.
        let msg: Range<usize> = match merkle_root_offset {
            None => {
                let shred = shred::layout::get_shred(packet);
                let msg = shred.and_then(shred::layout::get_signed_data_offsets);
                add_offset(msg.unwrap_or_default(), offset)
            }
            Some(merkle_root_offset) => {
                merkle_root_offset..merkle_root_offset + SIZE_OF_MERKLE_ROOT
            }
        };
        signature_offsets.push(sig.start as u32);
        msg_start_offsets.push(msg.start as u32);
        let msg_size = msg.end.saturating_sub(msg.start);
        msg_sizes.push(msg_size as u32);
    }
    (signature_offsets, msg_start_offsets, msg_sizes)
}

pub fn verify_shreds_gpu(
    thread_pool: &ThreadPool,
    batches: &[PacketBatch],
    slot_leaders: &HashMap<Slot, Pubkey>,
    recycler_cache: &RecyclerCache,
) -> Vec<Vec<u8>> {
    let Some(api) = perf_libs::api() else {
        return verify_shreds_cpu(thread_pool, batches, slot_leaders);
    };
    let (pubkeys, pubkey_offsets) =
        slot_key_data_for_gpu(thread_pool, batches, slot_leaders, recycler_cache);
    //HACK: Pubkeys vector is passed along as a `PacketBatch` buffer to the GPU
    //TODO: GPU needs a more opaque interface, which can handle variable sized structures for data
    let (merkle_roots, merkle_roots_offsets) =
        get_merkle_roots(thread_pool, batches, recycler_cache);
    // Merkle roots are placed after pubkeys; adjust offsets accordingly.
    let merkle_roots_offsets = {
        let shift = pubkeys.len();
        merkle_roots_offsets
            .into_iter()
            .map(move |offset| Some(offset? + shift))
    };
    let offset = pubkeys.len() + merkle_roots.len();
    let (signature_offsets, msg_start_offsets, msg_sizes) =
        shred_gpu_offsets(offset, batches, merkle_roots_offsets, recycler_cache);
    let mut out = recycler_cache.buffer().allocate("out_buffer");
    out.set_pinnable();
    out.resize(signature_offsets.len(), 0u8);
    let mut elems = vec![
        elems_from_buffer(&pubkeys),
        elems_from_buffer(&merkle_roots),
    ];
    elems.extend(batches.iter().map(|batch| perf_libs::Elems {
        elems: batch.as_ptr().cast::<u8>(),
        num: batch.len() as u32,
    }));
    let num_packets = elems.iter().map(|elem| elem.num).sum();
    trace!("Starting verify num packets: {}", num_packets);
    trace!("elem len: {}", elems.len() as u32);
    trace!("packet sizeof: {}", size_of::<Packet>() as u32);
    const USE_NON_DEFAULT_STREAM: u8 = 1;
    unsafe {
        let res = (api.ed25519_verify_many)(
            elems.as_ptr(),
            elems.len() as u32,
            size_of::<Packet>() as u32,
            num_packets,
            signature_offsets.len() as u32,
            msg_sizes.as_ptr(),
            pubkey_offsets.as_ptr(),
            signature_offsets.as_ptr(),
            msg_start_offsets.as_ptr(),
            out.as_mut_ptr(),
            USE_NON_DEFAULT_STREAM,
        );
        if res != 0 {
            trace!("RETURN!!!: {}", res);
        }
    }
    trace!("done verify");
    trace!("out buf {:?}", out);

    // Each shred has exactly one signature.
    let v_sig_lens = batches.iter().map(|batch| repeat(1u32).take(batch.len()));
    let mut rvs: Vec<_> = batches.iter().map(|batch| vec![0u8; batch.len()]).collect();
    sigverify::copy_return_values(v_sig_lens, &out, &mut rvs);

    inc_new_counter_debug!("ed25519_shred_verify_gpu", out.len());
    rvs
}

fn sign_shred_cpu(keypair: &Keypair, packet: &mut Packet) {
    let sig = shred::layout::get_signature_range();
    let msg = shred::layout::get_shred(packet)
        .and_then(shred::layout::get_signed_data)
        .unwrap();
    assert!(
        packet.meta().size >= sig.end,
        "packet is not large enough for a signature"
    );
    let signature = keypair.sign_message(msg.as_ref());
    trace!("signature {:?}", signature);
    packet.buffer_mut()[sig].copy_from_slice(signature.as_ref());
}

pub fn sign_shreds_cpu(thread_pool: &ThreadPool, keypair: &Keypair, batches: &mut [PacketBatch]) {
    let packet_count = count_packets_in_batches(batches);
    debug!("CPU SHRED ECDSA for {}", packet_count);
    thread_pool.install(|| {
        batches.par_iter_mut().for_each(|batch| {
            batch[..]
                .par_iter_mut()
                .for_each(|p| sign_shred_cpu(keypair, p));
        });
    });
    inc_new_counter_debug!("ed25519_shred_sign_cpu", packet_count);
}

pub fn sign_shreds_gpu_pinned_keypair(keypair: &Keypair, cache: &RecyclerCache) -> PinnedVec<u8> {
    let mut vec = cache.buffer().allocate("pinned_keypair");
    let pubkey = keypair.pubkey().to_bytes();
    let secret = keypair.secret().to_bytes();
    let mut hasher = Sha512::default();
    hasher.update(secret);
    let mut result = hasher.finalize();
    result[0] &= 248;
    result[31] &= 63;
    result[31] |= 64;
    let size = pubkey.len() + result.len();
    resize_buffer(&mut vec, size);
    vec[0..pubkey.len()].copy_from_slice(&pubkey);
    vec[pubkey.len()..size].copy_from_slice(&result);
    vec
}

pub fn sign_shreds_gpu(
    thread_pool: &ThreadPool,
    keypair: &Keypair,
    pinned_keypair: &Option<Arc<PinnedVec<u8>>>,
    batches: &mut [PacketBatch],
    recycler_cache: &RecyclerCache,
) {
    let sig_size = size_of::<Signature>();
    let pubkey_size = size_of::<Pubkey>();
    let packet_count = count_packets_in_batches(batches);
    if packet_count < SIGN_SHRED_GPU_MIN || pinned_keypair.is_none() {
        return sign_shreds_cpu(thread_pool, keypair, batches);
    }
    let Some(api) = perf_libs::api() else {
        return sign_shreds_cpu(thread_pool, keypair, batches);
    };
    let pinned_keypair = pinned_keypair.as_ref().unwrap();

    //should be zero
    let mut pubkey_offsets = recycler_cache.offsets().allocate("pubkey offsets");
    pubkey_offsets.resize(packet_count, 0);

    let mut secret_offsets = recycler_cache.offsets().allocate("secret_offsets");
    secret_offsets.resize(packet_count, pubkey_size as u32);

    let (merkle_roots, merkle_roots_offsets) =
        get_merkle_roots(thread_pool, batches, recycler_cache);
    // Merkle roots are placed after the keypair; adjust offsets accordingly.
    let merkle_roots_offsets = {
        let shift = pinned_keypair.len();
        merkle_roots_offsets
            .into_iter()
            .map(move |offset| Some(offset? + shift))
    };
    let offset = pinned_keypair.len() + merkle_roots.len();
    trace!("offset: {}", offset);
    let (signature_offsets, msg_start_offsets, msg_sizes) =
        shred_gpu_offsets(offset, batches, merkle_roots_offsets, recycler_cache);
    let total_sigs = signature_offsets.len();
    let mut signatures_out = recycler_cache.buffer().allocate("ed25519 signatures");
    signatures_out.set_pinnable();
    signatures_out.resize(total_sigs * sig_size, 0);

    let mut elems = vec![
        elems_from_buffer(pinned_keypair),
        elems_from_buffer(&merkle_roots),
    ];
    elems.extend(batches.iter().map(|batch| perf_libs::Elems {
        elems: batch.as_ptr().cast::<u8>(),
        num: batch.len() as u32,
    }));
    let num_packets = elems.iter().map(|elem| elem.num).sum();
    trace!("Starting verify num packets: {}", num_packets);
    trace!("elem len: {}", elems.len() as u32);
    trace!("packet sizeof: {}", size_of::<Packet>() as u32);
    const USE_NON_DEFAULT_STREAM: u8 = 1;
    unsafe {
        let res = (api.ed25519_sign_many)(
            elems.as_mut_ptr(),
            elems.len() as u32,
            size_of::<Packet>() as u32,
            num_packets,
            total_sigs as u32,
            msg_sizes.as_ptr(),
            pubkey_offsets.as_ptr(),
            secret_offsets.as_ptr(),
            msg_start_offsets.as_ptr(),
            signatures_out.as_mut_ptr(),
            USE_NON_DEFAULT_STREAM,
        );
        if res != 0 {
            trace!("RETURN!!!: {}", res);
        }
    }
    trace!("done sign");
    // Cumulative number of packets within batches.
    let num_packets: Vec<_> = batches
        .iter()
        .scan(0, |num_packets, batch| {
            let out = *num_packets;
            *num_packets += batch.len();
            Some(out)
        })
        .collect();
    thread_pool.install(|| {
        batches
            .par_iter_mut()
            .zip(num_packets)
            .for_each(|(batch, num_packets)| {
                batch[..]
                    .par_iter_mut()
                    .enumerate()
                    .for_each(|(packet_ix, packet)| {
                        let sig_ix = packet_ix + num_packets;
                        let sig_start = sig_ix * sig_size;
                        let sig_end = sig_start + sig_size;
                        packet.buffer_mut()[..sig_size]
                            .copy_from_slice(&signatures_out[sig_start..sig_end]);
                    });
            });
    });
    inc_new_counter_debug!("ed25519_shred_sign_gpu", packet_count);
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{
            shred::{ProcessShredsStats, Shred, ShredFlags, LEGACY_SHRED_DATA_CAPACITY},
            shredder::{ReedSolomonCache, Shredder},
        },
        assert_matches::assert_matches,
        rand::{seq::SliceRandom, Rng},
        rayon::ThreadPoolBuilder,
        solana_entry::entry::Entry,
        solana_sdk::{
            hash,
            hash::Hash,
            signature::{Keypair, Signer},
            system_transaction,
            transaction::Transaction,
        },
        std::iter::{once, repeat_with},
    };

    fn run_test_sigverify_shred_cpu(slot: Slot) {
        solana_logger::setup();
        let mut packet = Packet::default();
        let mut shred = Shred::new_from_data(
            slot,
            0xc0de,
            0xdead,
            &[1, 2, 3, 4],
            ShredFlags::LAST_SHRED_IN_SLOT,
            0,
            0,
            0xc0de,
        );
        assert_eq!(shred.slot(), slot);
        let keypair = Keypair::new();
        shred.sign(&keypair);
        trace!("signature {}", shred.signature());
        packet.buffer_mut()[..shred.payload().len()].copy_from_slice(shred.payload());
        packet.meta_mut().size = shred.payload().len();

        let leader_slots = HashMap::from([(slot, keypair.pubkey())]);
        assert!(verify_shred_cpu(&packet, &leader_slots));

        let wrong_keypair = Keypair::new();
        let leader_slots = HashMap::from([(slot, wrong_keypair.pubkey())]);
        assert!(!verify_shred_cpu(&packet, &leader_slots));

        let leader_slots = HashMap::new();
        assert!(!verify_shred_cpu(&packet, &leader_slots));
    }

    #[test]
    fn test_sigverify_shred_cpu() {
        run_test_sigverify_shred_cpu(0xdead_c0de);
    }

    // fork (Phase 3): build an ml_dsa merkle data shred (Ed25519 leader =
    // `keypair`, post-quantum signer = `ml_dsa_keypair`). With is_last_in_slot
    // the final FEC set is resigned (ml_dsa+chained+resigned, 0x30) and we return
    // a shred from it; otherwise the set is non-resigned (0xC0/0xD0).
    fn make_ml_dsa_data_shred<R: Rng>(
        rng: &mut R,
        slot: Slot,
        keypair: &Keypair,
        ml_dsa_keypair: &MlDsaKeypair,
        is_last_in_slot: bool,
    ) -> Shred {
        let reed_solomon_cache = ReedSolomonCache::default();
        let version = rng.gen();
        let num_entries = rng.gen_range(64..128);
        let chained_merkle_root = Hash::new_from_array(rng.gen());
        let entries = make_entries(rng, num_entries);
        let (data_shreds, _coding) = Shredder::new(slot, slot - 1, 0, version)
            .unwrap()
            .entries_to_shreds(
                keypair,
                Some(ml_dsa_keypair),
                &entries,
                is_last_in_slot,
                Some(chained_merkle_root), // chained_merkle_root
                0,                         // next_shred_index
                0,                         // next_code_index
                true,                      // merkle_variant
                &reed_solomon_cache,
                &mut ProcessShredsStats::default(),
            );
        // The resigned (final) FEC set is the last one, so its shreds are last.
        let shred = if is_last_in_slot {
            data_shreds.into_iter().last().unwrap()
        } else {
            data_shreds.into_iter().next().unwrap()
        };
        assert!(shred::layout::is_ml_dsa_shred(shred.payload()));
        shred
    }

    fn to_packet(shred: &Shred) -> Packet {
        let mut packet = Packet::default();
        shred.copy_to_packet(&mut packet);
        packet
    }

    #[test]
    fn test_sigverify_shred_ml_dsa_cpu() {
        solana_logger::setup();
        let mut rng = rand::thread_rng();
        let slot = 0xdead_c0de;
        let keypair = Keypair::new();
        let ml_dsa_keypair = MlDsaKeypair::from_seed(&[3u8; 32]);
        let shred = make_ml_dsa_data_shred(&mut rng, slot, &keypair, &ml_dsa_keypair, false);
        let leader_slots = HashMap::from([(slot, keypair.pubkey())]);

        // Honest ml_dsa shred verifies post-quantum.
        assert!(verify_shred_ml_dsa_cpu(&to_packet(&shred), &leader_slots));

        // Wrong / unknown leader => Ed25519 check (a) fails.
        assert!(!verify_shred_ml_dsa_cpu(
            &to_packet(&shred),
            &HashMap::from([(slot, Keypair::new().pubkey())])
        ));
        assert!(!verify_shred_ml_dsa_cpu(
            &to_packet(&shred),
            &HashMap::new()
        ));

        // Tampered ML-DSA signature (flip the last payload byte) => check (c) fails.
        let mut tampered = to_packet(&shred);
        let size = tampered.meta().size;
        tampered.buffer_mut()[size - 1] ^= 0xff;
        assert!(!verify_shred_ml_dsa_cpu(&tampered, &leader_slots));

        // Forged trailer: a DIFFERENT ml_dsa key validly signs the SAME Merkle
        // root, so check (c) would pass — but its address != the in-root
        // commitment, so the binding check (b) rejects it. This is the property
        // that stops an attacker from swapping the (unsigned) trailer.
        let merkle_root =
            shred::layout::get_merkle_root(shred::layout::get_shred(&to_packet(&shred)).unwrap())
                .unwrap();
        let attacker = MlDsaKeypair::from_seed(&[0xab; 32]);
        let attacker_sig = attacker.sign(merkle_root.as_ref()).unwrap();
        assert!(attacker.verify(merkle_root.as_ref(), &attacker_sig));
        let mut forged = to_packet(&shred);
        let size = forged.meta().size;
        let pubkey_start =
            size - (MlDsaKeypair::PUBLIC_KEY_LENGTH + MlDsaKeypair::SIGNATURE_LENGTH);
        let sig_start = pubkey_start + MlDsaKeypair::PUBLIC_KEY_LENGTH;
        forged.buffer_mut()[pubkey_start..sig_start].copy_from_slice(attacker.public_key_bytes());
        forged.buffer_mut()[sig_start..sig_start + MlDsaKeypair::SIGNATURE_LENGTH]
            .copy_from_slice(&attacker_sig);
        assert!(!verify_shred_ml_dsa_cpu(&forged, &leader_slots));

        // fork (Phase 3): a RESIGNED (final-FEC-set) ml_dsa shred also verifies.
        // Its layout differs — the dormant 64-byte retransmitter slot sits AFTER
        // the trailer at the very end of the payload — so this exercises
        // get_ml_dsa_regions / verify_shred_ml_dsa_cpu against the resigned trailer
        // offset (0x30 = MerkleData ml_dsa+chained+resigned).
        let resigned = make_ml_dsa_data_shred(&mut rng, slot, &keypair, &ml_dsa_keypair, true);
        // The shred-variant byte follows the 64-byte Ed25519 signature; high
        // nibble 0x30 == MerkleData ml_dsa+chained+resigned.
        assert_eq!(resigned.payload()[64] & 0xF0, 0x30);
        assert!(verify_shred_ml_dsa_cpu(&to_packet(&resigned), &leader_slots));
        // Wrong leader => Ed25519 check (a) fails on the resigned shred too.
        assert!(!verify_shred_ml_dsa_cpu(
            &to_packet(&resigned),
            &HashMap::from([(slot, Keypair::new().pubkey())])
        ));
        // Tampering the ml_dsa signature's last byte — which sits just before the
        // dormant 64-byte resigned slot, not at the payload end — fails check (c),
        // confirming the trailer is read from before that slot.
        let mut tampered_resigned = to_packet(&resigned);
        let size = tampered_resigned.meta().size;
        tampered_resigned.buffer_mut()[size - 64 - 1] ^= 0xff;
        assert!(!verify_shred_ml_dsa_cpu(&tampered_resigned, &leader_slots));

        // A plain Ed25519 (non-ml_dsa) shred is ignored by the ml_dsa pass.
        let mut ed_shred = Shred::new_from_data(
            slot,
            0,
            0,
            &[1, 2, 3, 4],
            ShredFlags::LAST_SHRED_IN_SLOT,
            0,
            0,
            0,
        );
        ed_shred.sign(&keypair);
        assert!(!verify_shred_ml_dsa_cpu(
            &to_packet(&ed_shred),
            &leader_slots
        ));

        // The non-gating audit pass counts the honest shred as ok.
        let batches = [PacketBatch::new(vec![to_packet(&shred)])];
        assert_eq!(audit_ml_dsa_shreds(&batches, &leader_slots), (1, 0));
    }

    // fork (Phase 3): a tampered ml_dsa shred is dropped under strict enforcement
    // (--ml-dsa-shred-strict) but left untouched by the default advisory audit.
    // Crucially, strict mode only ever touches ml_dsa shreds: a plain Ed25519
    // shred is never discarded (else strict mode would partition the node from
    // turbine), and already-discarded packets are skipped.
    #[test]
    fn test_enforce_ml_dsa_shreds_strict_vs_audit() {
        solana_logger::setup();
        let mut rng = rand::thread_rng();
        let slot = 0xdead_c0de;
        let keypair = Keypair::new();
        let ml_dsa_keypair = MlDsaKeypair::from_seed(&[5u8; 32]);
        let leader_slots = HashMap::from([(slot, keypair.pubkey())]);
        let honest = make_ml_dsa_data_shred(&mut rng, slot, &keypair, &ml_dsa_keypair, false);

        // Flip the last trailer byte => the ml_dsa signature fails to verify.
        let mut tampered = to_packet(&honest);
        let size = tampered.meta().size;
        tampered.buffer_mut()[size - 1] ^= 0xff;

        // A plain Ed25519 (non-ml_dsa) shred — strict mode must never touch it.
        let mut ed_shred = Shred::new_from_data(
            slot,
            0,
            0,
            &[1, 2, 3, 4],
            ShredFlags::LAST_SHRED_IN_SLOT,
            0,
            0,
            0,
        );
        ed_shred.sign(&keypair);

        // An already-discarded ml_dsa packet — both passes must skip it.
        let mut predropped = to_packet(&honest);
        predropped.meta_mut().set_discard(true);

        let mut batches = [PacketBatch::new(vec![
            to_packet(&honest),
            tampered,
            to_packet(&ed_shred),
            predropped,
        ])];

        // Advisory audit is non-gating: counts (1 ok, 1 fail) and discards nothing
        // new (the Ed25519 and already-discarded packets are skipped, not counted).
        assert_eq!(audit_ml_dsa_shreds(&batches, &leader_slots), (1, 1));
        assert!(!batches[0][0].meta().discard()); // honest ml_dsa
        assert!(!batches[0][1].meta().discard()); // tampered ml_dsa (advisory keeps)
        assert!(!batches[0][2].meta().discard()); // plain Ed25519
        assert!(batches[0][3].meta().discard()); // pre-discarded stays discarded

        // Strict enforcement drops ONLY the failing ml_dsa shred; the honest and
        // the plain Ed25519 shred are kept, and the pre-discarded one is skipped.
        assert_eq!(enforce_ml_dsa_shreds(&mut batches, &leader_slots), (1, 1));
        assert!(!batches[0][0].meta().discard()); // honest ml_dsa kept
        assert!(batches[0][1].meta().discard()); // tampered ml_dsa dropped
        assert!(!batches[0][2].meta().discard()); // plain Ed25519 untouched
        assert!(batches[0][3].meta().discard()); // pre-discarded unchanged
    }

    fn run_test_sigverify_shreds_cpu(thread_pool: &ThreadPool, slot: Slot) {
        solana_logger::setup();
        let mut batches = [PacketBatch::default()];
        let mut shred = Shred::new_from_data(
            slot,
            0xc0de,
            0xdead,
            &[1, 2, 3, 4],
            ShredFlags::LAST_SHRED_IN_SLOT,
            0,
            0,
            0xc0de,
        );
        let keypair = Keypair::new();
        shred.sign(&keypair);
        batches[0].resize(1, Packet::default());
        batches[0][0].buffer_mut()[..shred.payload().len()].copy_from_slice(shred.payload());
        batches[0][0].meta_mut().size = shred.payload().len();

        let leader_slots = HashMap::from([(slot, keypair.pubkey())]);
        let rv = verify_shreds_cpu(thread_pool, &batches, &leader_slots);
        assert_eq!(rv, vec![vec![1]]);

        let wrong_keypair = Keypair::new();
        let leader_slots = HashMap::from([(slot, wrong_keypair.pubkey())]);
        let rv = verify_shreds_cpu(thread_pool, &batches, &leader_slots);
        assert_eq!(rv, vec![vec![0]]);

        let leader_slots = HashMap::new();
        let rv = verify_shreds_cpu(thread_pool, &batches, &leader_slots);
        assert_eq!(rv, vec![vec![0]]);

        let leader_slots = HashMap::from([(slot, keypair.pubkey())]);
        batches[0][0].meta_mut().size = 0;
        let rv = verify_shreds_cpu(thread_pool, &batches, &leader_slots);
        assert_eq!(rv, vec![vec![0]]);
    }

    #[test]
    fn test_sigverify_shreds_cpu() {
        let thread_pool = ThreadPoolBuilder::new().num_threads(3).build().unwrap();
        run_test_sigverify_shreds_cpu(&thread_pool, 0xdead_c0de);
    }

    fn run_test_sigverify_shreds_gpu(thread_pool: &ThreadPool, slot: Slot) {
        solana_logger::setup();
        let recycler_cache = RecyclerCache::default();

        let mut batches = [PacketBatch::default()];
        let mut shred = Shred::new_from_data(
            slot,
            0xc0de,
            0xdead,
            &[1, 2, 3, 4],
            ShredFlags::LAST_SHRED_IN_SLOT,
            0,
            0,
            0xc0de,
        );
        let keypair = Keypair::new();
        shred.sign(&keypair);
        batches[0].resize(1, Packet::default());
        batches[0][0].buffer_mut()[..shred.payload().len()].copy_from_slice(shred.payload());
        batches[0][0].meta_mut().size = shred.payload().len();

        let leader_slots =
            HashMap::from([(std::u64::MAX, Pubkey::default()), (slot, keypair.pubkey())]);
        let rv = verify_shreds_gpu(thread_pool, &batches, &leader_slots, &recycler_cache);
        assert_eq!(rv, vec![vec![1]]);

        let wrong_keypair = Keypair::new();
        let leader_slots = HashMap::from([
            (std::u64::MAX, Pubkey::default()),
            (slot, wrong_keypair.pubkey()),
        ]);
        let rv = verify_shreds_gpu(thread_pool, &batches, &leader_slots, &recycler_cache);
        assert_eq!(rv, vec![vec![0]]);

        let leader_slots = HashMap::from([(std::u64::MAX, Pubkey::default())]);
        let rv = verify_shreds_gpu(thread_pool, &batches, &leader_slots, &recycler_cache);
        assert_eq!(rv, vec![vec![0]]);

        batches[0][0].meta_mut().size = 0;
        let leader_slots =
            HashMap::from([(std::u64::MAX, Pubkey::default()), (slot, keypair.pubkey())]);
        let rv = verify_shreds_gpu(thread_pool, &batches, &leader_slots, &recycler_cache);
        assert_eq!(rv, vec![vec![0]]);
    }

    #[test]
    fn test_sigverify_shreds_gpu() {
        let thread_pool = ThreadPoolBuilder::new().num_threads(3).build().unwrap();
        run_test_sigverify_shreds_gpu(&thread_pool, 0xdead_c0de);
    }

    fn run_test_sigverify_shreds_sign_gpu(thread_pool: &ThreadPool, slot: Slot) {
        solana_logger::setup();
        let recycler_cache = RecyclerCache::default();

        let num_packets = 32;
        let num_batches = 100;
        let mut packet_batch = PacketBatch::with_capacity(num_packets);
        packet_batch.resize(num_packets, Packet::default());

        for (i, p) in packet_batch.iter_mut().enumerate() {
            let shred = Shred::new_from_data(
                slot,
                0xc0de,
                i as u16,
                &[5; LEGACY_SHRED_DATA_CAPACITY],
                ShredFlags::LAST_SHRED_IN_SLOT,
                1,
                2,
                0xc0de,
            );
            shred.copy_to_packet(p);
        }
        let mut batches = vec![packet_batch; num_batches];
        let keypair = Keypair::new();
        let pinned_keypair = sign_shreds_gpu_pinned_keypair(&keypair, &recycler_cache);
        let pinned_keypair = Some(Arc::new(pinned_keypair));
        let pubkeys = HashMap::from([(std::u64::MAX, Pubkey::default()), (slot, keypair.pubkey())]);
        //unsigned
        let rv = verify_shreds_gpu(thread_pool, &batches, &pubkeys, &recycler_cache);
        assert_eq!(rv, vec![vec![0; num_packets]; num_batches]);
        //signed
        sign_shreds_gpu(
            thread_pool,
            &keypair,
            &pinned_keypair,
            &mut batches,
            &recycler_cache,
        );
        let rv = verify_shreds_cpu(thread_pool, &batches, &pubkeys);
        assert_eq!(rv, vec![vec![1; num_packets]; num_batches]);

        let rv = verify_shreds_gpu(thread_pool, &batches, &pubkeys, &recycler_cache);
        assert_eq!(rv, vec![vec![1; num_packets]; num_batches]);
    }

    #[test]
    fn test_sigverify_shreds_sign_gpu() {
        let thread_pool = ThreadPoolBuilder::new().num_threads(3).build().unwrap();
        run_test_sigverify_shreds_sign_gpu(&thread_pool, 0xdead_c0de);
    }

    fn run_test_sigverify_shreds_sign_cpu(thread_pool: &ThreadPool, slot: Slot) {
        solana_logger::setup();

        let mut batches = [PacketBatch::default()];
        let keypair = Keypair::new();
        let shred = Shred::new_from_data(
            slot,
            0xc0de,
            0xdead,
            &[1, 2, 3, 4],
            ShredFlags::LAST_SHRED_IN_SLOT,
            0,
            0,
            0xc0de,
        );
        batches[0].resize(1, Packet::default());
        batches[0][0].buffer_mut()[..shred.payload().len()].copy_from_slice(shred.payload());
        batches[0][0].meta_mut().size = shred.payload().len();

        let pubkeys = HashMap::from([(slot, keypair.pubkey()), (std::u64::MAX, Pubkey::default())]);
        //unsigned
        let rv = verify_shreds_cpu(thread_pool, &batches, &pubkeys);
        assert_eq!(rv, vec![vec![0]]);
        //signed
        sign_shreds_cpu(thread_pool, &keypair, &mut batches);
        let rv = verify_shreds_cpu(thread_pool, &batches, &pubkeys);
        assert_eq!(rv, vec![vec![1]]);
    }

    #[test]
    fn test_sigverify_shreds_sign_cpu() {
        let thread_pool = ThreadPoolBuilder::new().num_threads(3).build().unwrap();
        run_test_sigverify_shreds_sign_cpu(&thread_pool, 0xdead_c0de);
    }

    fn make_transaction<R: Rng>(rng: &mut R) -> Transaction {
        let block = rng.gen::<[u8; 32]>();
        let recent_blockhash = hash::hashv(&[&block]);
        system_transaction::transfer(
            &Keypair::new(),       // from
            &Pubkey::new_unique(), // to
            rng.gen(),             // lamports
            recent_blockhash,
        )
    }

    fn make_entry<R: Rng>(rng: &mut R, prev_hash: &Hash) -> Entry {
        let size = rng.gen_range(16..32);
        let txs = repeat_with(|| make_transaction(rng)).take(size).collect();
        Entry::new(
            prev_hash,
            rng.gen_range(1..64), // num_hashes
            txs,
        )
    }

    fn make_entries<R: Rng>(rng: &mut R, num_entries: usize) -> Vec<Entry> {
        let prev_hash = hash::hashv(&[&rng.gen::<[u8; 32]>()]);
        let entry = make_entry(rng, &prev_hash);
        std::iter::successors(Some(entry), |entry| Some(make_entry(rng, &entry.hash)))
            .take(num_entries)
            .collect()
    }

    fn make_shreds<R: Rng>(rng: &mut R, keypairs: &HashMap<Slot, Keypair>) -> Vec<Shred> {
        let reed_solomon_cache = ReedSolomonCache::default();
        let mut shreds: Vec<_> = keypairs
            .iter()
            .flat_map(|(&slot, keypair)| {
                let parent_slot = slot - rng.gen::<u16>().max(1) as Slot;
                let num_entries = rng.gen_range(64..128);
                let (data_shreds, coding_shreds) = Shredder::new(
                    slot,
                    parent_slot,
                    rng.gen_range(0..0x40), // reference_tick
                    rng.gen(),              // version
                )
                .unwrap()
                .entries_to_shreds(
                    keypair,
                    None, // ml_dsa_keypair
                    &make_entries(rng, num_entries),
                    rng.gen(), // is_last_in_slot
                    // chained_merkle_root
                    rng.gen::<bool>().then(|| Hash::new_from_array(rng.gen())),
                    rng.gen_range(0..2671), // next_shred_index
                    rng.gen_range(0..2781), // next_code_index
                    rng.gen(),              // merkle_variant,
                    &reed_solomon_cache,
                    &mut ProcessShredsStats::default(),
                );
                [data_shreds, coding_shreds]
            })
            .flatten()
            .collect();
        shreds.shuffle(rng);
        // Assert that all shreds verfiy and sanitize.
        for shred in &shreds {
            let pubkey = keypairs[&shred.slot()].pubkey();
            assert!(shred.verify(&pubkey));
            assert_matches!(shred.sanitize(), Ok(()));
        }
        // Verfiy using layout api.
        for shred in &shreds {
            let shred = shred.payload();
            let slot = shred::layout::get_slot(shred).unwrap();
            let signature = shred::layout::get_signature(shred).unwrap();
            let pubkey = keypairs[&slot].pubkey();
            if let Some(offsets) = shred::layout::get_signed_data_offsets(shred) {
                assert!(signature.verify(pubkey.as_ref(), &shred[offsets]));
            }
            let data = shred::layout::get_signed_data(shred).unwrap();
            assert!(signature.verify(pubkey.as_ref(), data.as_ref()));
        }
        shreds
    }

    fn make_packets<R: Rng>(rng: &mut R, shreds: &[Shred]) -> Vec<PacketBatch> {
        let mut packets = shreds.iter().map(|shred| {
            let mut packet = Packet::default();
            shred.copy_to_packet(&mut packet);
            packet
        });
        let packets: Vec<_> = repeat_with(|| {
            let size = rng.gen_range(0..16);
            let packets = packets.by_ref().take(size).collect();
            let batch = PacketBatch::new(packets);
            (size == 0 || !batch.is_empty()).then_some(batch)
        })
        .while_some()
        .collect();
        assert_eq!(
            shreds.len(),
            packets.iter().map(PacketBatch::len).sum::<usize>()
        );
        assert!(count_packets_in_batches(&packets) > SIGN_SHRED_GPU_MIN);
        packets
    }

    #[test]
    fn test_verify_shreds_fuzz() {
        let mut rng = rand::thread_rng();
        let thread_pool = ThreadPoolBuilder::new().num_threads(3).build().unwrap();
        let recycler_cache = RecyclerCache::default();
        let keypairs = repeat_with(|| rng.gen_range(169_367_809..169_906_789))
            .map(|slot| (slot, Keypair::new()))
            .take(3)
            .collect();
        let shreds = make_shreds(&mut rng, &keypairs);
        let pubkeys: HashMap<Slot, Pubkey> = keypairs
            .iter()
            .map(|(&slot, keypair)| (slot, keypair.pubkey()))
            .chain(once((Slot::MAX, Pubkey::default())))
            .collect();
        let mut packets = make_packets(&mut rng, &shreds);
        assert_eq!(
            verify_shreds_gpu(&thread_pool, &packets, &pubkeys, &recycler_cache),
            packets
                .iter()
                .map(|batch| vec![1u8; batch.len()])
                .collect::<Vec<_>>()
        );
        // Invalidate signatures for a random number of packets.
        let out: Vec<_> = packets
            .iter_mut()
            .map(|packets| {
                packets
                    .iter_mut()
                    .map(|packet| {
                        let coin_flip: bool = rng.gen();
                        if !coin_flip {
                            shred::layout::corrupt_packet(&mut rng, packet, &keypairs);
                        }
                        u8::from(coin_flip)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(
            verify_shreds_gpu(&thread_pool, &packets, &pubkeys, &recycler_cache),
            out
        );
    }

    #[test]
    fn test_sign_shreds_gpu() {
        let mut rng = rand::thread_rng();
        let thread_pool = ThreadPoolBuilder::new().num_threads(3).build().unwrap();
        let recycler_cache = RecyclerCache::default();
        let shreds = {
            let keypairs = repeat_with(|| rng.gen_range(169_367_809..169_906_789))
                .map(|slot| (slot, Keypair::new()))
                .take(3)
                .collect();
            make_shreds(&mut rng, &keypairs)
        };
        let keypair = Keypair::new();
        let pubkeys: HashMap<Slot, Pubkey> = {
            let pubkey = keypair.pubkey();
            shreds
                .iter()
                .map(Shred::slot)
                .map(|slot| (slot, pubkey))
                .chain(once((Slot::MAX, Pubkey::default())))
                .collect()
        };
        let mut packets = make_packets(&mut rng, &shreds);
        // Assert that initially all signatrues are invalid.
        assert_eq!(
            verify_shreds_gpu(&thread_pool, &packets, &pubkeys, &recycler_cache),
            packets
                .iter()
                .map(|batch| vec![0u8; batch.len()])
                .collect::<Vec<_>>()
        );
        let pinned_keypair = sign_shreds_gpu_pinned_keypair(&keypair, &recycler_cache);
        let pinned_keypair = Some(Arc::new(pinned_keypair));
        // Sign and verify shreds signatures.
        sign_shreds_gpu(
            &thread_pool,
            &keypair,
            &pinned_keypair,
            &mut packets,
            &recycler_cache,
        );
        assert_eq!(
            verify_shreds_gpu(&thread_pool, &packets, &pubkeys, &recycler_cache),
            packets
                .iter()
                .map(|batch| vec![1u8; batch.len()])
                .collect::<Vec<_>>()
        );
    }
}
