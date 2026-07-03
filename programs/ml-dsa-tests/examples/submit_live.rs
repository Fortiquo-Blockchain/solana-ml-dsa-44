//! Live, narrated end-to-end demo of the ML-DSA-44 (post-quantum) precompile.
//!
//! A "precompile" is a built-in checker the validator runs while processing a
//! transaction. This one verifies ML-DSA-44 (post-quantum) signatures. The demo
//! hands the chain a public key, a message, and a signature, and asks: "is this a
//! genuine post-quantum signature?" — the chain accepts a real one and rejects a
//! tampered one. Only the *signature being checked* is post-quantum; the fee payer
//! that pays for the transaction is still an ordinary Ed25519 wallet.
//!
//! Like the transfer demo, it is fully transparent: it prints the full keys, the
//! message and signature, the raw transaction bytes, and — at every chain
//! interaction — the exact JSON-RPC request and the raw result.
//!
//! Easiest way to run it (boots + tears down a fresh validator for you):
//!   bash programs/ml-dsa-tests/demo.sh
//! Or, against a validator you already have running on :8899:
//!   cargo run --release -p solana-ml-dsa-program-tests --example submit_live

use {
    base64::{engine::general_purpose::STANDARD as BASE64, Engine},
    fips204::{
        ml_dsa_44::{PrivateKey, PublicKey},
        traits::SerDes,
    },
    serde_json::{json, Value},
    solana_rpc_client::rpc_client::RpcClient,
    solana_rpc_client_api::request::RpcRequest,
    solana_sdk::{
        commitment_config::CommitmentConfig,
        hash::Hash,
        ml_dsa_instruction::{
            new_ml_dsa_instruction, DATA_START, PUBKEY_SERIALIZED_SIZE, SIGNATURE_SERIALIZED_SIZE,
        },
        ml_dsa_keypair::MlDsaKeypair,
        native_token::LAMPORTS_PER_SOL,
        signature::{Keypair, Signer},
        transaction::Transaction,
    },
    std::{str::FromStr, time::Duration},
};

const RULE: &str = "═══════════════════════════════════════════════════════════════";
const RPC_URL: &str = "http://127.0.0.1:8899";
const PK_LEN: usize = 1312; // ML-DSA-44 public key bytes
const SK_LEN: usize = 2560; // ML-DSA-44 secret key bytes

/// Make a JSON-RPC call, printing the exact request envelope and the raw result.
fn rpc(client: &RpcClient, method: RpcRequest, params: Value) -> Value {
    println!(
        "    → JSON-RPC request:  {}",
        method.build_request_json(1, params.clone())
    );
    match client.send::<Value>(method, params) {
        Ok(result) => {
            println!("    ← raw result:        {result}");
            result
        }
        Err(err) => {
            println!("    ← raw result:        (error) {err}");
            Value::Null
        }
    }
}

/// Poll getSignatureStatuses until confirmed; print the request once and the raw
/// confirming result (which also proves the slot it landed in).
fn confirm(client: &RpcClient, sig: &str) {
    let params = json!([[sig], {"searchTransactionHistory": false}]);
    println!("  Confirming on-chain (getSignatureStatuses):");
    println!(
        "    → JSON-RPC request:  {}",
        RpcRequest::GetSignatureStatuses.build_request_json(1, params.clone())
    );
    for _ in 0..60 {
        if let Ok(v) = client.send::<Value>(RpcRequest::GetSignatureStatuses, params.clone()) {
            let status = &v["value"][0];
            let cs = status["confirmationStatus"].as_str().unwrap_or("");
            if status["err"].is_null() && (cs == "confirmed" || cs == "finalized") {
                println!("    ← raw result:        {v}");
                println!("    ✅ confirmed in slot {}", status["slot"]);
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    eprintln!("    FAILED: signature {sig} never confirmed after ~30s");
    std::process::exit(1);
}

fn main() {
    println!("\n{RULE}");
    println!("  POST-QUANTUM SIGNATURE CHECK — live on a Solana validator");
    println!("{RULE}\n");
    println!("  A \"precompile\" is a built-in checker the validator runs while processing a");
    println!("  transaction. This one verifies ML-DSA-44 (post-quantum) signatures. We hand");
    println!("  the chain a public key, a message, and a signature, and ask: is the signature");
    println!("  genuine? It accepts a real one and rejects a tampered one. (The fee that pays");
    println!("  for the transaction still comes from an ordinary Ed25519 wallet — only the");
    println!("  signature being *checked* is post-quantum.)\n");
    println!("  Scheme                      Public key     Signature");
    println!("  Ed25519 (today)                   32 B           64 B");
    println!(
        "  ML-DSA-44 (post-quantum)        {PUBKEY_SERIALIZED_SIZE} B         {SIGNATURE_SERIALIZED_SIZE} B   (~38x bigger)\n"
    );
    println!("  Every chain interaction below shows the exact JSON-RPC call and raw reply.\n");

    let client =
        RpcClient::new_with_commitment(RPC_URL.to_string(), CommitmentConfig::confirmed());

    // ── Step 1: fund an ordinary Ed25519 fee payer ────────────────────
    println!("  ══ STEP 1: fund an ordinary (Ed25519) fee payer ════════════");
    println!("  The fee payer just pays the transaction fee — it is NOT the post-quantum part.");
    let payer = Keypair::new();
    let payer_addr = payer.pubkey().to_string();
    println!("  Fee payer (Ed25519) address: {payer_addr}");
    println!("  Airdropping 1 SOL (requestAirdrop):");
    let airdrop = rpc(
        &client,
        RpcRequest::RequestAirdrop,
        json!([payer_addr, LAMPORTS_PER_SOL]),
    );
    let airdrop_sig = airdrop.as_str().expect("airdrop should return a signature");
    confirm(&client, airdrop_sig);
    println!();

    // ── Step 2: the post-quantum keypair whose signature we will check ─
    println!("  ══ STEP 2: the post-quantum keypair we will sign with ══════\n");
    println!("  Generate an ML-DSA-44 keypair (private + public key).");
    let kp = MlDsaKeypair::new().unwrap();
    let kp_bytes = kp.to_bytes(); // [ public (1312) || secret (2560) ]
    let (pub_bytes, sec_bytes) = kp_bytes.split_at(PK_LEN);
    println!();
    println!("  PRIVATE KEY — {SK_LEN} bytes (kept secret; this is what produces the signature)");
    println!("  [shown in full for the demo only — you would NEVER print a real one]:");
    print_hex_block(sec_bytes, "    ");
    println!();
    println!("  PUBLIC KEY — {PK_LEN} bytes (handed to the chain so it can verify the signature):");
    print_hex_block(pub_bytes, "    ");
    println!();

    // Rebuild the fips204 key types from those exact bytes to sign with.
    let private_key = PrivateKey::try_from_bytes(sec_bytes.try_into().unwrap())
        .expect("reconstruct private key");
    let public_key =
        PublicKey::try_from_bytes(pub_bytes.try_into().unwrap()).expect("reconstruct public key");

    // ── Step 3: sign a message and build the "verify this" transaction ─
    println!("  ══ STEP 3: sign a message and ask the chain to verify it ═══");
    let message = b"post-quantum hello from infinia";
    println!("  Message being signed: {:?}", String::from_utf8_lossy(message));
    let ix = new_ml_dsa_instruction(&private_key, public_key, message);
    let ix_for_tamper = ix.clone(); // reuse the SAME signature in step 4, with one byte flipped
    // The signature lives right after the public key inside the instruction data.
    let sig_start = DATA_START + PUBKEY_SERIALIZED_SIZE;
    let signature = &ix.data[sig_start..sig_start + SIGNATURE_SERIALIZED_SIZE];
    println!("  Produced an ML-DSA-44 signature:");
    println!("    signature size:        {SIGNATURE_SERIALIZED_SIZE} bytes");
    println!("    signature (first 16 B): {}…", hex_preview(signature, 16));
    println!("  Fetching a recent blockhash to anchor the transaction:");
    let bh = rpc(
        &client,
        RpcRequest::GetLatestBlockhash,
        json!([{"commitment": "confirmed"}]),
    );
    let blockhash = Hash::from_str(bh["value"]["blockhash"].as_str().expect("blockhash"))
        .expect("parse blockhash");
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[&payer], blockhash);
    let wire = bincode::serialize(&tx).expect("serialize tx");
    println!(
        "\n  Whole transaction: {} bytes  (a stock Solana tx is capped at 1232 — this fork",
        wire.len()
    );
    println!("  widened that ceiling so one ~2.4 KB post-quantum signature fits).");
    println!("  Submitting to the validator (sendTransaction):");
    let send = rpc(
        &client,
        RpcRequest::SendTransaction,
        json!([BASE64.encode(&wire), {"encoding": "base64", "skipPreflight": false, "preflightCommitment": "processed"}]),
    );
    let sig_str = send
        .as_str()
        .unwrap_or_else(|| {
            eprintln!("        UNEXPECTED: the valid signature was rejected");
            std::process::exit(1);
        })
        .to_string();
    confirm(&client, &sig_str);
    println!("  ✅ The chain verified the post-quantum signature and accepted the transaction.\n");

    // ── The raw transaction bytes that were actually sent ─────────────
    println!("  ── The transaction data that was sent ({} bytes) ───────────", wire.len());
    println!("  It carries the fee payer's Ed25519 signature, then a single instruction whose");
    println!("  data = [ public key ({PUBKEY_SERIALIZED_SIZE} B) | signature ({SIGNATURE_SERIALIZED_SIZE} B) | message ] for the");
    println!("  precompile to verify. Full transaction, in hex:");
    print_hex_block(&wire, "    ");
    println!();

    // ── Step 4: can a forged signature get through? ───────────────────
    println!("  ══ STEP 4: can a FORGED signature get through? ══════════════");
    let mut bad_ix = ix_for_tamper;
    bad_ix.data[sig_start + 100] ^= 0xff; // flip one byte inside the signature
    let bad_tx =
        Transaction::new_signed_with_payer(&[bad_ix], Some(&payer.pubkey()), &[&payer], blockhash);
    let bad_wire = bincode::serialize(&bad_tx).expect("serialize tampered tx");
    println!("  Flipping ONE byte of the signature and resubmitting (sendTransaction):");
    let forged = rpc(
        &client,
        RpcRequest::SendTransaction,
        json!([BASE64.encode(&bad_wire), {"encoding": "base64", "skipPreflight": false, "preflightCommitment": "processed"}]),
    );
    if forged.is_null() {
        println!("    ✅ REJECTED — the validator caught the forgery.");
    } else {
        eprintln!("    UNEXPECTED: a tampered signature was accepted: {forged}");
        std::process::exit(1);
    }

    println!("\n{RULE}");
    println!("  Result: the chain verified a genuine quantum-resistant signature and");
    println!("  rejected a tampered one. Phase 0 (the app-level building block) works.");
    println!("{RULE}\n");
}

/// First `n` bytes as lowercase hex (no separators).
fn hex_preview(bytes: &[u8], n: usize) -> String {
    bytes.iter().take(n).map(|b| format!("{b:02x}")).collect()
}

/// Print every byte as lowercase hex, wrapped at 32 bytes (64 hex chars) per line.
fn print_hex_block(bytes: &[u8], indent: &str) {
    for chunk in bytes.chunks(32) {
        let line: String = chunk.iter().map(|b| format!("{b:02x}")).collect();
        println!("{indent}{line}");
    }
}
