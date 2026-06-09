//! Live Phase 1 demo: a SOL payment whose sender signs with **ML-DSA-44**
//! (post-quantum), with a post-quantum address (`sha256(public key)`), confirmed
//! on a running `solana-test-validator`. Unlike Phase 0, the transaction itself
//! is post-quantum-signed — there is no Ed25519 fee payer.
//!
//! Written to be readable by non-technical viewers AND fully transparent: it
//! narrates the wallet's creation (full private/public keys), the address
//! reduction, a random starting balance, a random share sent, the raw
//! transaction bytes, and — at every chain interaction — the exact JSON-RPC
//! request it sends plus the raw result the validator returns. Nothing is
//! mathematically faked; every balance is read back from the chain.
//!   bash programs/ml-dsa-tests/demo-transfer.sh

use {
    base64::{engine::general_purpose::STANDARD as BASE64, Engine},
    serde_json::{json, Value},
    solana_rpc_client::rpc_client::RpcClient,
    solana_rpc_client_api::request::RpcRequest,
    solana_sdk::{
        commitment_config::CommitmentConfig, hash::Hash, message::Message,
        ml_dsa_keypair::MlDsaKeypair, ml_dsa_transaction::MlDsaTransaction,
        native_token::LAMPORTS_PER_SOL, system_instruction,
    },
    std::{str::FromStr, time::Duration},
};

const RULE: &str = "═══════════════════════════════════════════════════════════════";
const RPC_URL: &str = "http://127.0.0.1:8899";

// Fixed FIPS 204 ML-DSA-44 sizes (in bytes).
const PK_LEN: usize = 1312; // public key
const SK_LEN: usize = 2560; // secret (private) key
const SIG_LEN: usize = 2420; // signature

/// Make a JSON-RPC call, printing the exact request envelope we send and the raw
/// result the validator returns. Returns the `result` value (or `Null` on error,
/// after printing it — used for the deliberately-rejected tampered transaction).
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

/// Read an address's balance from the chain (getBalance at confirmed commitment).
fn fetch_balance(client: &RpcClient, addr: &str) -> u64 {
    let r = rpc(
        client,
        RpcRequest::GetBalance,
        json!([addr, {"commitment": "confirmed"}]),
    );
    r["value"].as_u64().unwrap_or(0)
}

/// Poll getSignatureStatuses until the signature is confirmed; print the request
/// once and the raw confirming result.
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
                println!("    ✅ confirmed");
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
    println!("  POST-QUANTUM PAYMENT — live on a Solana validator");
    println!("{RULE}\n");
    println!("  In plain terms: one wallet sends some SOL to another. The only twist is");
    println!("  that the sender's signature is *post-quantum* (ML-DSA-44) — built to stay");
    println!("  secure even against a future quantum computer — instead of today's Ed25519.");
    println!("  Every chain interaction below shows the exact JSON-RPC call and raw reply.\n");

    let client =
        RpcClient::new_with_commitment(RPC_URL.to_string(), CommitmentConfig::confirmed());

    // ── Create Alice's wallet, step by step ───────────────────────────
    println!("  ══ CREATING ALICE'S POST-QUANTUM WALLET ═════════════════════\n");
    println!("  Step A — generate an ML-DSA-44 keypair (a matched private + public key).");
    let alice = MlDsaKeypair::new().unwrap();
    let alice_bytes = alice.to_bytes(); // [ public (1312) || secret (2560) ]
    let (alice_pub, alice_sec) = alice_bytes.split_at(PK_LEN);
    println!();
    println!("  PRIVATE KEY — {SK_LEN} bytes (kept secret; this is what signs payments)");
    println!("  [shown in full for the demo only — you would NEVER print a real one]:");
    print_hex_block(alice_sec, "    ");
    println!();
    println!("  PUBLIC KEY — {PK_LEN} bytes (shared; used to verify Alice's signature):");
    print_hex_block(alice_pub, "    ");
    println!();
    println!("  Step B — reduce that public key down to a short wallet address.");
    println!("    BEFORE:  public key  = {PK_LEN} bytes   (the block just above)");
    println!("    AFTER:   address     =   32 bytes   = sha256(public key)");
    println!("             {}", alice.address());
    println!("    The address is the short name people send funds to. The full {PK_LEN}-byte");
    println!("    public key still travels inside every transaction, so the validator can");
    println!("    re-hash it and confirm it matches this address.\n");

    // ── Bob (receiver) — created the same way, address shown ──────────
    let bob = MlDsaKeypair::new().unwrap();
    println!("  ══ BOB'S WALLET (the receiver) ══════════════════════════════");
    println!("  Created the same way (private + public key, then hashed). His address:");
    println!("    {}\n", bob.address());
    let alice_addr = alice.address().to_string();
    let bob_addr = bob.address().to_string();

    // ── Sizes at a glance: what is (and isn't) reduced ────────────────
    println!("  ── Sizes at a glance: what is (and isn't) reduced ───────────");
    println!("  public key:  Ed25519   32 B  ->  ML-DSA-44 {PK_LEN} B   (sent FULL-SIZE; NOT reduced)");
    println!("  signature:   Ed25519   64 B  ->  ML-DSA-44 {SIG_LEN} B   (sent FULL-SIZE; NOT reduced)");
    println!("  address:     Ed25519   32 B  ->  ML-DSA-44   32 B   <== the ONLY thing made small\n");

    // ── Step 1: give Alice a (random) starting balance ────────────────
    let mut rng = Rng::new();
    let initial = rng.range(10 * LAMPORTS_PER_SOL, 50 * LAMPORTS_PER_SOL);
    println!("  ══ STEP 1: give Alice a (random) starting balance ═══════════");
    println!("  Airdropping {} to Alice (requestAirdrop):", sol(initial));
    let airdrop = rpc(
        &client,
        RpcRequest::RequestAirdrop,
        json!([alice_addr, initial]),
    );
    let airdrop_sig = airdrop.as_str().expect("airdrop should return a signature");
    confirm(&client, airdrop_sig);
    println!();
    println!("  Reading the BEFORE balances straight from the chain:");
    let a0 = fetch_balance(&client, &alice_addr);
    let b0 = fetch_balance(&client, &bob_addr);
    println!("  = Alice {}   |   Bob {}\n", sol(a0), sol(b0));

    // ── Step 2: Alice sends a random share to Bob ─────────────────────
    let pct = rng.range(20, 80);
    let amount = a0 / 100 * pct; // pct% of Alice's (fetched) balance
    println!("  ══ STEP 2: Alice sends a (random) share to Bob ══════════════");
    println!("  Sending {pct}% of Alice's balance  =  {}", sol(amount));
    println!("  Fetching a recent blockhash to anchor the transaction:");
    let bh = rpc(
        &client,
        RpcRequest::GetLatestBlockhash,
        json!([{"commitment": "confirmed"}]),
    );
    let blockhash = Hash::from_str(bh["value"]["blockhash"].as_str().expect("blockhash"))
        .expect("parse blockhash");
    let mut message = Message::new(
        &[system_instruction::transfer(
            &alice.address(),
            &bob.address(),
            amount,
        )],
        Some(&alice.address()),
    );
    message.recent_blockhash = blockhash;
    let mltx = MlDsaTransaction::sign(message, &[&alice]).expect("ml-dsa sign");
    let wire = mltx.serialize();
    let sig_preview = if wire.len() >= 2 + 16 {
        hex_preview(&wire[2..2 + 16], 16)
    } else {
        "..".to_string()
    };
    println!("\n  Signing the payment with ML-DSA-44 (post-quantum):");
    println!("    signature size:        {SIG_LEN} bytes");
    println!("    signature (first 16 B): {sig_preview}…");
    println!("    whole signed payment:  {} bytes  (an Ed25519 one is ~250)", wire.len());
    println!("  Submitting to the validator (sendTransaction):");
    let send = rpc(
        &client,
        RpcRequest::SendTransaction,
        json!([BASE64.encode(&wire), {"encoding": "base64", "skipPreflight": false}]),
    );
    let sig_str = send
        .as_str()
        .unwrap_or_else(|| {
            eprintln!("        UNEXPECTED: a valid payment was rejected");
            std::process::exit(1);
        })
        .to_string();
    confirm(&client, &sig_str);
    println!();

    // ── The raw transaction bytes that were actually sent ─────────────
    let header = 2 + SIG_LEN + 1 + PK_LEN; // marker(1)+sigcount(1)+sig+pkcount(1)+pubkey
    let msg_len = wire.len().saturating_sub(header);
    println!("  ── The transaction data that was sent ───────────────────────");
    println!("  Total size: {} bytes. A post-quantum transaction is laid out as:", wire.len());
    println!("    • byte 0:          0x00  — marker that flags this as a post-quantum tx");
    println!("    • byte 1:          count = 1 signature");
    println!("    • next {SIG_LEN} bytes:  the ML-DSA-44 signature");
    println!("    • 1 byte:          count = 1 public key");
    println!("    • next {PK_LEN} bytes:  Alice's full public key (the SAME bytes printed above —");
    println!("                       the whole key rides inside every transaction)");
    println!("    • last {msg_len} bytes:    the message (the transfer: from, to, amount, blockhash)");
    println!("  Full transaction, in hex:");
    print_hex_block(&wire, "    ");
    println!();

    // ── Step 3: balances AFTER the payment (read from the chain) ──────
    println!("  ══ STEP 3: balances AFTER the payment (read from the chain) ═");
    let a1 = fetch_balance(&client, &alice_addr);
    let b1 = fetch_balance(&client, &bob_addr);
    println!();
    println!(
        "  Alice:  {:>14}  →  {:>14}   (sent {}, minus a tiny network fee)",
        sol(a0),
        sol(a1),
        sol(amount)
    );
    println!(
        "  Bob:    {:>14}  →  {:>14}   (received {})",
        sol(b0),
        sol(b1),
        sol(amount)
    );
    assert_eq!(b1 - b0, amount, "Bob should have received exactly the sent amount");
    println!("  ✅ The post-quantum payment went through (numbers fetched, not calculated).\n");

    // ── Step 4: can a forged signature get through? ───────────────────
    println!("  ══ STEP 4: can a FORGED signature get through? ══════════════");
    let mut bad = wire.clone();
    bad[2 + 50] ^= 0xff; // flip one byte inside the ML-DSA signature
    println!("  Flipping ONE byte of the signature and resubmitting (sendTransaction):");
    let forged = rpc(
        &client,
        RpcRequest::SendTransaction,
        json!([BASE64.encode(&bad), {"encoding": "base64", "skipPreflight": false}]),
    );
    if forged.is_null() {
        println!("    ✅ REJECTED — the validator caught the forgery.");
    } else {
        eprintln!("    UNEXPECTED: a tampered payment was accepted: {forged}");
        std::process::exit(1);
    }

    println!("\n{RULE}");
    println!("  Result: Alice paid Bob with a post-quantum signature; the validator");
    println!("  verified, executed, and confirmed it — and rejected a forged copy.");
    println!("  No Ed25519 anywhere in this payment.");
    println!("{RULE}\n");
}

/// Lamports → a friendly "12.3456 SOL" string.
fn sol(lamports: u64) -> String {
    format!("{:.4} SOL", lamports as f64 / LAMPORTS_PER_SOL as f64)
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

/// Tiny dependency-free PRNG (splitmix64 seed from the clock → xorshift64* stream),
/// just to give the demo a random starting balance and a random send percentage.
struct Rng(u64);
impl Rng {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        let mut z = nanos.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Self((z ^ (z >> 31)) | 1) // ensure non-zero for xorshift
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in the inclusive range [lo, hi].
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}
