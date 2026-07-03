//! Phase 3 demo helper: open a stopped validator's blockstore and post-quantum-
//! verify the ML-DSA-44 shreds it produced as leader.
//!
//! Usage: verify_ml_dsa_shreds <LEDGER_PATH> <LEADER_IDENTITY_PUBKEY>
//!
//! For every shred in the ledger it runs the production three-step check
//! (`verify_shred_ml_dsa_cpu`): (a) the Ed25519 signature authenticates the
//! Merkle root against the leader, (b) the trailer public key matches the
//! in-root commitment, and (c) the ML-DSA-44 signature over the root verifies.
//! Exits non-zero if no ML-DSA shreds are found or any fails to verify.

use {
    solana_ledger::{
        blockstore::Blockstore, shred, sigverify_shreds::verify_shred_ml_dsa_cpu,
    },
    solana_sdk::{packet::Packet, pubkey::Pubkey},
    std::{collections::HashMap, path::Path, process::exit, str::FromStr},
};

fn main() {
    let mut args = std::env::args().skip(1);
    let ledger_path = args.next().unwrap_or_else(|| {
        eprintln!("usage: verify_ml_dsa_shreds <LEDGER_PATH> <LEADER_IDENTITY_PUBKEY>");
        exit(2);
    });
    let identity = args
        .next()
        .and_then(|s| Pubkey::from_str(&s).ok())
        .unwrap_or_else(|| {
            eprintln!("usage: verify_ml_dsa_shreds <LEDGER_PATH> <LEADER_IDENTITY_PUBKEY>");
            exit(2);
        });

    let blockstore = Blockstore::open(Path::new(&ledger_path)).unwrap_or_else(|err| {
        eprintln!("failed to open blockstore at {ledger_path}: {err}");
        exit(1);
    });

    println!("Leader identity (slot leader for every slot on this single node): {identity}");
    println!("Scanning blockstore at {ledger_path} for ML-DSA-44 shreds...\n");

    let (mut total, mut ml_dsa, mut verified, mut failed) = (0usize, 0usize, 0usize, 0usize);
    let mut slots_with_ml_dsa: Vec<(u64, usize)> = Vec::new();

    let slots = blockstore.slot_meta_iterator(0).unwrap_or_else(|err| {
        eprintln!("failed to iterate slots: {err}");
        exit(1);
    });
    for (slot, _meta) in slots {
        let mut shreds = blockstore.get_data_shreds_for_slot(slot, 0).unwrap_or_default();
        shreds.append(&mut blockstore.get_coding_shreds_for_slot(slot, 0).unwrap_or_default());
        if shreds.is_empty() {
            continue;
        }
        // Single-node: the leader for every slot is this node's identity.
        let slot_leaders = HashMap::from([(slot, identity)]);
        let mut slot_ml_dsa = 0usize;
        for shred in &shreds {
            total += 1;
            if !shred::layout::is_ml_dsa_shred(shred.payload()) {
                continue;
            }
            ml_dsa += 1;
            slot_ml_dsa += 1;
            let mut packet = Packet::default();
            shred.copy_to_packet(&mut packet);
            if verify_shred_ml_dsa_cpu(&packet, &slot_leaders) {
                verified += 1;
            } else {
                failed += 1;
                eprintln!("  ✗ slot {slot} index {} FAILED post-quantum verify", shred.index());
            }
        }
        if slot_ml_dsa > 0 {
            slots_with_ml_dsa.push((slot, slot_ml_dsa));
        }
    }

    println!("Per-slot ML-DSA-44 shred counts (first 10 slots with ml_dsa shreds):");
    for (slot, count) in slots_with_ml_dsa.iter().take(10) {
        println!("  slot {slot}: {count} ML-DSA shreds");
    }
    println!();
    println!("==================== summary ====================");
    println!("  total shreds scanned : {total}");
    println!("  ML-DSA-44 shreds     : {ml_dsa}");
    println!("  post-quantum verified: {verified}");
    println!("  verify failures      : {failed}");
    println!("=================================================");

    if ml_dsa == 0 {
        eprintln!(
            "\nFAIL: no ML-DSA-44 shreds found. Was the validator started with --ml-dsa-shred?"
        );
        exit(1);
    }
    if failed > 0 {
        eprintln!("\nFAIL: {failed} ML-DSA-44 shred(s) did not verify.");
        exit(1);
    }
    println!(
        "\nOK: all {ml_dsa} ML-DSA-44 shred(s) verify post-quantum (Ed25519 root + \
         commitment binding + ML-DSA-44 signature)."
    );
}
