//! Live, narrated end-to-end demo of the ML-DSA-44 (post-quantum) precompile.
//!
//! Connects to a running `solana-test-validator`, funds an ordinary Ed25519 fee
//! payer, then submits a transaction whose instruction asks the validator to
//! verify a post-quantum (ML-DSA-44) signature. It shows the size contrast with
//! today's Ed25519, re-fetches the confirmed transaction from the chain to prove
//! it is real, and finally proves a tampered signature is rejected. Only the
//! precompile payload is post-quantum; the fee payer is still Ed25519.
//!
//! Easiest way to run it (boots + tears down a fresh validator for you):
//!   bash programs/ml-dsa-tests/demo.sh
//!
//! Or, against a validator you already have running:
//!   cargo run --release -p solana-ml-dsa-program-tests --example submit_live

use {
    fips204::ml_dsa_44,
    solana_rpc_client::rpc_client::RpcClient,
    solana_sdk::{
        commitment_config::CommitmentConfig,
        ml_dsa_instruction::{
            new_ml_dsa_instruction, DATA_START, PUBKEY_SERIALIZED_SIZE, SIGNATURE_SERIALIZED_SIZE,
        },
        native_token::LAMPORTS_PER_SOL,
        signature::{Keypair, Signer},
        transaction::Transaction,
    },
};

const RULE: &str = "═══════════════════════════════════════════════════════════════";

fn main() {
    println!("\n{RULE}");
    println!("  Post-quantum signatures on a Solana validator — live demo");
    println!("{RULE}\n");

    println!("  Scheme                      Public key     Signature");
    println!("  Ed25519 (today)                   32 B           64 B");
    println!(
        "  ML-DSA-44 (post-quantum)        {PUBKEY_SERIALIZED_SIZE} B         {SIGNATURE_SERIALIZED_SIZE} B   (~38x bigger)\n"
    );
    println!("  A stock Solana transaction is capped at 1232 bytes — one ML-DSA-44");
    println!("  signature alone is {SIGNATURE_SERIALIZED_SIZE}. This fork widened that limit so it fits.\n");

    let client = RpcClient::new_with_commitment(
        "http://127.0.0.1:8899".to_string(),
        CommitmentConfig::confirmed(),
    );

    // [1/3] Fund an ordinary Ed25519 fee payer.
    print!("  [1/3] Funding a fee payer on the local chain ... ");
    let payer = Keypair::new();
    let airdrop_sig = client
        .request_airdrop(&payer.pubkey(), LAMPORTS_PER_SOL)
        .expect("airdrop request failed — is the validator running on :8899?");
    let mut confirmed = false;
    for _ in 0..60 {
        match client.confirm_transaction(&airdrop_sig) {
            Ok(true) => {
                confirmed = true;
                break;
            }
            Ok(false) => {}
            Err(e) => eprintln!("airdrop confirm check failed (retrying): {e}"),
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    if !confirmed {
        eprintln!("FAILED: airdrop never confirmed after ~30s");
        std::process::exit(1);
    }
    println!("done");
    println!("        fee payer: {}\n", payer.pubkey());

    // [2/3] Submit a transaction carrying a real ML-DSA-44 signature.
    let message = b"post-quantum hello from infinia";
    let (public_key, private_key) = ml_dsa_44::try_keygen().expect("ml-dsa keygen");
    let ix = new_ml_dsa_instruction(&private_key, public_key, message);
    let blockhash = client.get_latest_blockhash().expect("blockhash");
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    let tx_size = bincode::serialize(&tx).expect("serialize tx").len();

    println!("  [2/3] Submitting a transaction with a {SIGNATURE_SERIALIZED_SIZE}-byte ML-DSA-44 signature");
    println!("        transaction size: {tx_size} bytes  (stock Solana limit: 1232)");
    let sig = match client.send_and_confirm_transaction(&tx) {
        Ok(sig) => sig,
        Err(e) => {
            eprintln!("        UNEXPECTED: the valid tx was rejected: {e}");
            std::process::exit(1);
        }
    };
    println!("        ✅ ACCEPTED — the chain verified the post-quantum signature");
    println!("        signature: {sig}");

    // Independently re-fetch from the chain to prove it really landed in a block.
    if let Ok(statuses) = client.get_signature_statuses(&[sig]) {
        if let Some(Some(status)) = statuses.value.into_iter().next() {
            println!(
                "        ↳ re-fetched from the chain: confirmed in slot {}",
                status.slot
            );
        }
    }
    println!();

    // [3/3] Flip one byte of the signature and prove the validator rejects it.
    let (public_key, private_key) = ml_dsa_44::try_keygen().expect("ml-dsa keygen");
    let mut bad_ix = new_ml_dsa_instruction(&private_key, public_key, message);
    // A byte well inside the 2420-byte signature region.
    let tamper_at = DATA_START + PUBKEY_SERIALIZED_SIZE + 100;
    bad_ix.data[tamper_at] ^= 0xff;
    let blockhash = client.get_latest_blockhash().expect("blockhash");
    let bad_tx =
        Transaction::new_signed_with_payer(&[bad_ix], Some(&payer.pubkey()), &[&payer], blockhash);

    println!("  [3/3] Submitting the same transaction with one signature byte flipped");
    match client.send_and_confirm_transaction(&bad_tx) {
        Ok(sig) => {
            eprintln!("        UNEXPECTED: the tampered tx was confirmed: {sig}");
            std::process::exit(1);
        }
        Err(_) => println!("        ✅ REJECTED — the validator caught the forgery"),
    }

    println!("\n{RULE}");
    println!("  Result: the chain accepted a quantum-resistant signature and");
    println!("  rejected a tampered one. Phase 0 works.");
    println!("{RULE}\n");
}
