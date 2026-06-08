//! Live Phase 1 demo: a SOL transfer whose fee payer signs with **ML-DSA-44**
//! (post-quantum), with a post-quantum address (`sha256(public key)`), confirmed
//! on a running `solana-test-validator`. Unlike Phase 0, the transaction itself
//! is post-quantum-signed — there is no Ed25519 fee payer.
//!
//! Easiest way to run it (boots + tears down a fresh validator):
//!   bash programs/ml-dsa-tests/demo-transfer.sh

use {
    base64::{engine::general_purpose::STANDARD as BASE64, Engine},
    solana_rpc_client::rpc_client::RpcClient,
    solana_rpc_client_api::request::RpcRequest,
    solana_sdk::{
        commitment_config::CommitmentConfig, message::Message, ml_dsa_keypair::MlDsaKeypair,
        ml_dsa_transaction::MlDsaTransaction, native_token::LAMPORTS_PER_SOL, signature::Signature,
        system_instruction,
    },
    std::str::FromStr,
};

const RULE: &str = "═══════════════════════════════════════════════════════════════";

/// Submit raw ML-DSA transaction bytes via the JSON-RPC `sendTransaction`.
fn submit_ml_dsa(client: &RpcClient, wire: &[u8], skip_preflight: bool) -> Result<String, String> {
    let b64 = BASE64.encode(wire);
    client
        .send(
            RpcRequest::SendTransaction,
            serde_json::json!([b64, {"encoding": "base64", "skipPreflight": skip_preflight}]),
        )
        .map_err(|e| e.to_string())
}

fn main() {
    println!("\n{RULE}");
    println!("  Post-quantum TRANSACTION SIGNING on a Solana validator — live demo");
    println!("{RULE}\n");
    println!("  The fee payer signs this SOL transfer with ML-DSA-44, and its");
    println!("  address is sha256(public key). No Ed25519 anywhere in the payment.\n");

    let client = RpcClient::new_with_commitment(
        "http://127.0.0.1:8899".to_string(),
        CommitmentConfig::confirmed(),
    );

    // Two post-quantum accounts.
    let payer = MlDsaKeypair::new().unwrap();
    let recipient = MlDsaKeypair::new().unwrap();
    println!("  payer     (ML-DSA): {}", payer.address());
    println!("  recipient (ML-DSA): {}\n", recipient.address());

    // Airdrop to the post-quantum address (a normal 32-byte Pubkey to accounts-db).
    print!("  [1/3] Airdropping 2 SOL to the post-quantum payer ... ");
    let airdrop = client
        .request_airdrop(&payer.address(), 2 * LAMPORTS_PER_SOL)
        .expect("airdrop request failed — is the validator running on :8899?");
    wait_confirmed(&client, &airdrop);
    println!("done\n");

    // Build + ML-DSA-sign the transfer.
    let amount = LAMPORTS_PER_SOL;
    let mut message = Message::new(
        &[system_instruction::transfer(
            &payer.address(),
            &recipient.address(),
            amount,
        )],
        Some(&payer.address()),
    );
    message.recent_blockhash = client.get_latest_blockhash().expect("blockhash");
    let mltx = MlDsaTransaction::sign(message, &[&payer]).expect("ml-dsa sign");
    let wire = mltx.serialize();

    println!("  [2/3] Submitting a transfer signed with ML-DSA-44 ({} bytes)", wire.len());
    let sig_str = submit_ml_dsa(&client, &wire, false).unwrap_or_else(|e| {
        eprintln!("        UNEXPECTED: valid tx was rejected: {e}");
        std::process::exit(1);
    });
    let sig = Signature::from_str(&sig_str).expect("signature");
    wait_confirmed(&client, &sig);
    let recipient_balance = client.get_balance(&recipient.address()).unwrap_or(0);
    println!("        ✅ CONFIRMED on-chain — id {sig_str}");
    println!("        recipient balance: {recipient_balance} lamports (expected {amount})");
    assert_eq!(recipient_balance, amount, "recipient should have received the transfer");
    println!();

    // Tamper one signature byte → the validator's preflight must reject it.
    println!("  [3/3] Submitting the same transfer with one signature byte flipped");
    let mut bad = wire.clone();
    bad[2 + 50] ^= 0xff; // marker(1) + shortu16 count(1) -> inside the ML-DSA signature
    match submit_ml_dsa(&client, &bad, false) {
        Ok(sig) => {
            eprintln!("        UNEXPECTED: tampered tx was accepted: {sig}");
            std::process::exit(1);
        }
        Err(_) => println!("        ✅ REJECTED — the validator caught the forged signature"),
    }

    println!("\n{RULE}");
    println!("  Result: a post-quantum-signed transfer was verified, executed, and");
    println!("  confirmed; a forged one was rejected. Phase 1 works.");
    println!("{RULE}\n");
}

/// Poll until a signature confirms (bounded, ~30s), failing loudly otherwise.
fn wait_confirmed(client: &RpcClient, sig: &Signature) {
    for _ in 0..60 {
        if client.confirm_transaction(sig).unwrap_or(false) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    eprintln!("        FAILED: {sig} never confirmed after ~30s");
    std::process::exit(1);
}
