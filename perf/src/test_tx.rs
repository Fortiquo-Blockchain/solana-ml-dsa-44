use {
    crate::packet::{Packet, PacketBatch},
    rand::{CryptoRng, Rng, RngCore},
    solana_sdk::{
        clock::Slot,
        hash::Hash,
        instruction::CompiledInstruction,
        message::Message,
        ml_dsa_keypair::MlDsaKeypair,
        ml_dsa_transaction::MlDsaTransaction,
        pubkey::Pubkey,
        signature::{Keypair, Signer},
        stake,
        system_instruction::{self, SystemInstruction},
        system_program, system_transaction,
        transaction::Transaction,
    },
    solana_vote_program::vote_transaction,
};

pub fn test_tx() -> Transaction {
    let keypair1 = Keypair::new();
    let pubkey1 = keypair1.pubkey();
    let zero = Hash::default();
    system_transaction::transfer(&keypair1, &pubkey1, 42, zero)
}

pub fn test_invalid_tx() -> Transaction {
    let mut tx = test_tx();
    tx.signatures = vec![Transaction::get_invalid_signature()];
    tx
}

pub fn test_multisig_tx() -> Transaction {
    let keypair0 = Keypair::new();
    let keypair1 = Keypair::new();
    let keypairs = vec![&keypair0, &keypair1];
    let lamports = 5;
    let blockhash = Hash::default();

    let transfer_instruction = SystemInstruction::Transfer { lamports };

    let program_ids = vec![system_program::id(), stake::program::id()];

    let instructions = vec![CompiledInstruction::new(
        0,
        &transfer_instruction,
        vec![0, 1],
    )];

    Transaction::new_with_compiled_instructions(
        &keypairs,
        &[],
        blockhash,
        program_ids,
        instructions,
    )
}

/// Raw wire bytes of a post-quantum ML-DSA-44 (Phase 1, `0x00`-marked) transfer
/// transaction — the workload the CPU verify pipeline runs through
/// `sigverify::verify_ml_dsa_packet` (deserialize, `sha256(pubkey)==account_key`
/// address binding, then ML-DSA-44 verify). Each call mints a fresh keypair and
/// signs, so it is comparatively slow (ML-DSA keygen + sign); build one wire and
/// reuse it across packets when benchmarking *verify* throughput.
pub fn test_ml_dsa_tx_wire() -> Vec<u8> {
    let payer = MlDsaKeypair::new().expect("ml-dsa keygen");
    let message = Message::new(
        &[system_instruction::transfer(
            &payer.address(),
            &Pubkey::new_unique(),
            42,
        )],
        Some(&payer.address()),
    );
    MlDsaTransaction::sign(message, &[&payer])
        .expect("ml-dsa sign")
        .serialize()
}

/// Pack raw ML-DSA `0x00` wire bytes into a [`Packet`] verbatim — the wire IS the
/// payload (it is NOT bincode-framed like an Ed25519 [`Transaction`]), matching
/// how the validator ingests post-quantum packets.
pub fn ml_dsa_packet(wire: &[u8]) -> Packet {
    let mut packet = Packet::from_data(None, 0u8).expect("packet header");
    packet.buffer_mut()[..wire.len()].copy_from_slice(wire);
    packet.meta_mut().size = wire.len();
    packet
}

/// Repeat one prepared ML-DSA `0x00` packet into `Vec<PacketBatch>` of
/// `packets_per_batch` (the final batch holds the remainder), for sigverify
/// throughput benchmarking through `sigverify::ed25519_verify`.
pub fn ml_dsa_packet_batches(
    wire: &[u8],
    total_packets: usize,
    packets_per_batch: usize,
) -> Vec<PacketBatch> {
    debug_assert!(packets_per_batch > 0, "packets_per_batch must be non-zero");
    let packet = ml_dsa_packet(wire);
    let mut batches = Vec::new();
    let mut remaining = total_packets;
    while remaining > 0 {
        let n = remaining.min(packets_per_batch);
        let mut batch = PacketBatch::with_capacity(n);
        batch.resize(n, packet.clone());
        batches.push(batch);
        remaining -= n;
    }
    batches
}

pub fn new_test_vote_tx<R>(rng: &mut R) -> Transaction
where
    R: CryptoRng + RngCore,
{
    let mut slots: Vec<Slot> = std::iter::repeat_with(|| rng.gen()).take(5).collect();
    slots.sort_unstable();
    slots.dedup();
    let switch_proof_hash = rng.gen_bool(0.5).then(Hash::new_unique);
    vote_transaction::new_vote_transaction(
        slots,
        Hash::new_unique(), // bank_hash
        Hash::new_unique(), // blockhash
        &Keypair::new(),    // node_keypair
        &Keypair::new(),    // vote_keypair
        &Keypair::new(),    // authorized_voter_keypair
        switch_proof_hash,
    )
}
