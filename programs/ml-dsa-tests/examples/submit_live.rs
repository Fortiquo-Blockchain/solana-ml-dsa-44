//! Live end-to-end demo for the ML-DSA-44 precompile.
//!
//! Submits a real transaction whose instruction asks the validator to verify a
//! post-quantum (ML-DSA-44) signature, and confirms it on-chain. The fee payer
//! is an ordinary Ed25519 keypair — only the precompile payload is
//! post-quantum. The transaction is ~3.9 KB, which only fits because this fork
//! raises `PACKET_DATA_SIZE`.
//!
//! Usage (run in WSL, with a validator already running):
//!   solana-test-validator --reset --ledger ~/solana-test-ledger
//!   cargo run --release -p solana-ml-dsa-program-tests --example submit_live

use {
    fips204::ml_dsa_44,
    solana_rpc_client::rpc_client::RpcClient,
    solana_sdk::{
        commitment_config::CommitmentConfig,
        ml_dsa_instruction::new_ml_dsa_instruction,
        native_token::LAMPORTS_PER_SOL,
        signature::{Keypair, Signer},
        transaction::Transaction,
    },
};

fn main() {
    let client = RpcClient::new_with_commitment(
        "http://127.0.0.1:8899".to_string(),
        CommitmentConfig::confirmed(),
    );

    // Ordinary Ed25519 fee payer (only the precompile payload is post-quantum).
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
        eprintln!("airdrop never confirmed after ~30s — aborting");
        std::process::exit(1);
    }
    println!("funded fee payer {}", payer.pubkey());

    let message = b"post-quantum hello from infinia";

    // ---- valid signature: expect the transaction to confirm ----
    let (public_key, private_key) = ml_dsa_44::try_keygen().expect("ml-dsa keygen");
    let ix = new_ml_dsa_instruction(&private_key, public_key, message);
    let blockhash = client.get_latest_blockhash().expect("blockhash");
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&payer.pubkey()),
        &[&payer],
        blockhash,
    );
    match client.send_and_confirm_transaction(&tx) {
        Ok(sig) => println!("✅ valid ML-DSA-44 precompile tx confirmed: {sig}"),
        Err(e) => {
            eprintln!("❌ expected the valid tx to confirm, but got: {e}");
            std::process::exit(1);
        }
    }

    // ---- tampered signature: expect the validator to reject it ----
    let (public_key, private_key) = ml_dsa_44::try_keygen().expect("ml-dsa keygen");
    let mut bad_ix = new_ml_dsa_instruction(&private_key, public_key, message);
    // Flip a byte in the middle of the payload (avoid the unused byte at index 1).
    let tamper_at = bad_ix.data.len() / 2;
    bad_ix.data[tamper_at] ^= 0xff;
    let blockhash = client.get_latest_blockhash().expect("blockhash");
    let bad_tx = Transaction::new_signed_with_payer(
        &[bad_ix],
        Some(&payer.pubkey()),
        &[&payer],
        blockhash,
    );
    match client.send_and_confirm_transaction(&bad_tx) {
        Ok(sig) => {
            eprintln!("❌ tampered tx unexpectedly confirmed: {sig}");
            std::process::exit(1);
        }
        Err(_) => println!("✅ tampered ML-DSA-44 precompile tx correctly rejected"),
    }

    println!("\nDone — ML-DSA-44 precompile verified live on the test validator.");
}
