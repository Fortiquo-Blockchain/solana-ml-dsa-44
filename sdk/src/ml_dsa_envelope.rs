//! Replay-safe post-quantum (ML-DSA-44) transaction signatures.
//!
//! The earlier Phase-1 `0x00` ML-DSA transaction format verified its
//! ML-DSA signature only at TPU ingress and then recorded a transaction whose only
//! signature is a non-verifiable synthetic id — so a *peer* that replays the block
//! cannot re-verify it and marks the slot dead. This module fixes that by keeping
//! the transaction a **standard** [`Transaction`] and carrying the ML-DSA proof
//! *inside* it, as the Phase-0 ML-DSA precompile instruction. Because the proof
//! travels in the transaction, it is recorded in the block and re-verified
//! identically on every node — at TPU ingress (`perf` sigverify) and on replay
//! (`bank.verify_transaction` -> [`crate::transaction`] signature verification).
//!
//! A PQ transaction here is a legacy [`Transaction`] where:
//!   * the sole required signer / fee payer is the ML-DSA address
//!     (`sha256(public_key)`), with a deterministic *placeholder* envelope
//!     signature (used only as the dedup/status id), and
//!   * the **last** instruction is an ML-DSA carrier (precompile) instruction whose
//!     data, after the precompile header (`[num_sig][pad][offsets]`), is
//!     `[public_key || signature || signed_bytes]`, where `signed_bytes`
//!     is the serialization of the transaction's *core* message (every instruction
//!     except the carrier).
//!
//! (Every node re-verifies on **replay** — the universal path. At **TPU ingress**
//! only the leader's sigverify runs, and today only on the CPU path: see the GPU
//! caveat on `verify_ml_dsa_envelope_packet` in `perf::sigverify`.)
//!
//! Verification ([`verify_ml_dsa_envelope`]) accepts the signer iff: the carrier's
//! `sha256(public_key)` equals the signer address, the ML-DSA signature verifies
//! over the embedded bytes, and — the load-bearing anti-lift check — those
//! embedded bytes are *exactly* the serialization of this transaction's core
//! message. The last check binds the proof to this specific transaction (its
//! instructions, accounts, and recent blockhash), so a valid proof cannot be
//! moved onto a different transaction.
//!
//! Scope: a single ML-DSA signer, legacy message only (matching Phase 1). The
//! path is structurally inert for ordinary Ed25519 transactions — one never has a
//! carrier instruction, and `sha256(some_ml_dsa_pubkey) == an_ed25519_signer` is a
//! SHA-256 preimage — so it can only ever *accept* a genuine ML-DSA envelope, never
//! change the outcome for existing traffic.
//!
//! Deployment / extension constraints:
//!   * **No runtime feature gate.** Acceptance is inert for non-carrier traffic, but
//!     every block carrying a PQ vote contains a carrier tx, so a node that has NOT
//!     upgraded would reject those blocks (`SignatureFailure` → dead slot → fork).
//!     Safe on this self-hosted, all-nodes-upgraded-together fork; a heterogeneous or
//!     staged rollout would need a feature gate here first.
//!   * **Dedup id is signature-derived.** `MlDsaKeypair::sign` is hedged (randomized),
//!     so re-signing the same core yields a different [`synthetic_id`]. Benign for
//!     votes (idempotent/monotonic). Before extending this path to user *payments*,
//!     sign deterministically or dedup on the message hash, else the same logical
//!     payment could be admitted twice within one blockhash window.

#![cfg(feature = "full")]

use {
    crate::{
        ml_dsa_instruction::{
            ml_dsa_instruction_from_parts, CARRIER_MESSAGE_OFFSET, CARRIER_PUBKEY_OFFSET,
            CARRIER_SIGNATURE_OFFSET,
        },
        ml_dsa_keypair::{ml_dsa_address, MlDsaKeypair},
        ml_dsa_program,
        signature::Signature,
        transaction::Transaction,
    },
    fips204::{
        ml_dsa_44::{PublicKey, PK_LEN, SIG_LEN},
        traits::{SerDes, Verifier},
    },
    solana_program::{
        hash::{hash, Hash},
        instruction::{AccountMeta, CompiledInstruction, Instruction},
        message::Message,
    },
};

/// Build a replay-safe ML-DSA-44 transaction: `core_instructions` signed by
/// `keypair` (its address is fee payer + authority), with the ML-DSA proof carried
/// as an appended precompile instruction. The returned [`Transaction`] is an
/// ordinary legacy transaction — it records, gossips, and replays like any other.
pub fn sign_ml_dsa_transaction(
    core_instructions: &[Instruction],
    keypair: &MlDsaKeypair,
    recent_blockhash: Hash,
) -> Result<Transaction, Box<dyn std::error::Error>> {
    let payer = keypair.address();

    // `signed_bytes` is exactly the core message a peer will rebuild and compare
    // against; sign that, so the ML-DSA signature commits to every core
    // instruction, account, and the recent blockhash.
    let core_message = Message::new_with_blockhash(core_instructions, Some(&payer), &recent_blockhash);
    let signed_bytes = core_message.serialize();
    let signature = keypair.sign(&signed_bytes)?;

    let carrier = ml_dsa_instruction_from_parts(keypair.public_key_bytes(), &signature, &signed_bytes);
    let mut instructions = core_instructions.to_vec();
    instructions.push(carrier);
    let message = Message::new_with_blockhash(&instructions, Some(&payer), &recent_blockhash);

    Ok(Transaction {
        signatures: vec![synthetic_id(&signature, &signed_bytes)],
        message,
    })
}

/// Verify a legacy message that claims to carry a single ML-DSA envelope signer.
/// Returns `true` iff the sole signer is authorized by a genuine, transaction-bound
/// ML-DSA proof. Callers use this as a fallback when ordinary Ed25519 verification
/// of the signatures fails.
pub fn verify_ml_dsa_envelope(message: &Message, signatures: &[Signature]) -> bool {
    // Single ML-DSA signer, legacy message (Phase 1/2 scope).
    if message.header.num_required_signatures != 1 || signatures.len() != 1 {
        return false;
    }
    let Some(signer) = message.account_keys.first().copied() else {
        return false;
    };

    // The carrier must be the last instruction and target the ML-DSA precompile.
    let carrier_index = match message.instructions.len().checked_sub(1) {
        Some(index) => index,
        None => return false,
    };
    let carrier = &message.instructions[carrier_index];
    match message.program_id(carrier_index) {
        Some(program_id) if *program_id == ml_dsa_program::id() => {}
        _ => return false,
    }

    let Some((public_key_bytes, signature_bytes, embedded)) = parse_carrier(&carrier.data) else {
        return false;
    };

    // (a) address binding: the sole signer must be sha256(public_key).
    if ml_dsa_address(&public_key_bytes) != signer {
        return false;
    }
    // (b) ML-DSA signature over the embedded bytes (empty FIPS-204 context).
    let Ok(public_key) = PublicKey::try_from_bytes(public_key_bytes) else {
        return false;
    };
    if !public_key.verify(embedded, &signature_bytes, &[]) {
        return false;
    }
    // (c) anti-lift binding: the embedded bytes must be exactly this transaction's
    // core message, so the proof cannot be attached to a different transaction.
    let Some(core_message_bytes) = rebuild_core_message_bytes(message, carrier_index) else {
        return false;
    };
    if core_message_bytes != embedded {
        return false;
    }
    // (d) dedup/status id must be the deterministic synthetic id for these bytes.
    signatures[0] == synthetic_id(&signature_bytes, embedded)
}

/// Parse a carrier instruction's data into `(public_key, signature, embedded_message)`
/// by the fixed layout produced by `ml_dsa_instruction_from_parts`. Rejects anything
/// that is not a well-formed single-signature carrier.
fn parse_carrier(data: &[u8]) -> Option<([u8; PK_LEN], [u8; SIG_LEN], &[u8])> {
    if data.len() < CARRIER_MESSAGE_OFFSET || data.first() != Some(&1u8) {
        return None;
    }
    let public_key: [u8; PK_LEN] = data[CARRIER_PUBKEY_OFFSET..CARRIER_SIGNATURE_OFFSET]
        .try_into()
        .ok()?;
    let signature: [u8; SIG_LEN] = data[CARRIER_SIGNATURE_OFFSET..CARRIER_MESSAGE_OFFSET]
        .try_into()
        .ok()?;
    Some((public_key, signature, &data[CARRIER_MESSAGE_OFFSET..]))
}

/// Rebuild the serialization of the transaction's core message (all instructions
/// except the carrier) from the outer message, so it can be compared byte-for-byte
/// against the ML-DSA-signed `embedded` bytes. Returns `None` on any malformed
/// account/program index.
fn rebuild_core_message_bytes(message: &Message, carrier_index: usize) -> Option<Vec<u8>> {
    let payer = *message.account_keys.first()?;
    let num_required = message.header.num_required_signatures as usize;
    let readonly_signed = message.header.num_readonly_signed_accounts as usize;
    let readonly_unsigned = message.header.num_readonly_unsigned_accounts as usize;
    let total = message.account_keys.len();

    // Writability straight from the header layout (NOT `is_maybe_writable`, which
    // demotes sysvars/programs): this reproduces the per-account flags the signer's
    // `AccountMeta`s carried, so the rebuilt message matches byte-for-byte.
    let is_writable = |index: usize| -> bool {
        if index < num_required {
            index < num_required.saturating_sub(readonly_signed)
        } else {
            index < total.saturating_sub(readonly_unsigned)
        }
    };

    let mut core = Vec::with_capacity(carrier_index);
    for compiled in message.instructions.iter().take(carrier_index) {
        core.push(decompile_instruction(message, compiled, num_required, is_writable)?);
    }
    Some(Message::new_with_blockhash(&core, Some(&payer), &message.recent_blockhash).serialize())
}

fn decompile_instruction(
    message: &Message,
    compiled: &CompiledInstruction,
    num_required: usize,
    is_writable: impl Fn(usize) -> bool,
) -> Option<Instruction> {
    let program_id = *message.account_keys.get(compiled.program_id_index as usize)?;
    let mut accounts = Vec::with_capacity(compiled.accounts.len());
    for &index in &compiled.accounts {
        let index = index as usize;
        accounts.push(AccountMeta {
            pubkey: *message.account_keys.get(index)?,
            is_signer: index < num_required,
            is_writable: is_writable(index),
        });
    }
    Some(Instruction {
        program_id,
        accounts,
        data: compiled.data.clone(),
    })
}

/// A deterministic 64-byte id used as the placeholder envelope signature (the
/// runtime keys status/dedup/blockstore on it). It is `sha256(signature) ||
/// sha256(signed_bytes)` — an identifier, not a verifiable signature.
fn synthetic_id(signature: &[u8; SIG_LEN], signed_bytes: &[u8]) -> Signature {
    let mut id = [0u8; 64];
    id[..32].copy_from_slice(hash(signature).as_ref());
    id[32..].copy_from_slice(hash(signed_bytes).as_ref());
    Signature::from(id)
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{signature::Keypair, signer::Signer, system_instruction},
        solana_program::pubkey::Pubkey,
    };

    fn transfer_ixs(from: &Pubkey) -> Vec<Instruction> {
        vec![system_instruction::transfer(from, &Pubkey::new_unique(), 1)]
    }

    #[test]
    fn round_trip_verifies() {
        let kp = MlDsaKeypair::new().unwrap();
        let ixs = transfer_ixs(&kp.address());
        let tx = sign_ml_dsa_transaction(&ixs, &kp, Hash::new_unique()).unwrap();
        assert!(verify_ml_dsa_envelope(&tx.message, &tx.signatures));
        // Ordinary Ed25519 verification cannot pass (placeholder id is not a sig).
        assert!(tx.verify_with_results().iter().any(|ok| !ok));
    }

    #[test]
    fn tampered_instruction_rejected() {
        let kp = MlDsaKeypair::new().unwrap();
        let ixs = transfer_ixs(&kp.address());
        let mut tx = sign_ml_dsa_transaction(&ixs, &kp, Hash::new_unique()).unwrap();
        // Flip a byte in the (non-carrier) transfer instruction's data. This is caught
        // by the anti-lift rebuild check (c) — the ML-DSA signature over the original
        // embedded bytes still verifies, but the rebuilt core no longer matches them.
        tx.message.instructions[0].data[0] ^= 0xff;
        assert!(!verify_ml_dsa_envelope(&tx.message, &tx.signatures));
    }

    #[test]
    fn tampered_blockhash_rejected() {
        let kp = MlDsaKeypair::new().unwrap();
        let mut tx =
            sign_ml_dsa_transaction(&transfer_ixs(&kp.address()), &kp, Hash::new_unique()).unwrap();
        // The proof commits to the recent blockhash; changing it breaks the rebuild.
        tx.message.recent_blockhash = Hash::new_unique();
        assert!(!verify_ml_dsa_envelope(&tx.message, &tx.signatures));
    }

    #[test]
    fn multi_instruction_sysvar_core_round_trips() {
        // The production shape is a *vote* (sysvars + vote program + multiple
        // accounts), not the single system transfer above. Exercise the byte-exact
        // core rebuild for a multi-instruction core that references a sysvar
        // (readonly-unsigned) and shares accounts across instructions.
        use solana_program::{instruction::AccountMeta, sysvar};
        let kp = MlDsaKeypair::new().unwrap();
        let program = Pubkey::new_unique();
        let writable = Pubkey::new_unique();
        let readonly = Pubkey::new_unique();
        let ix0 = Instruction {
            program_id: program,
            accounts: vec![
                AccountMeta::new(kp.address(), true),
                AccountMeta::new(writable, false),
                AccountMeta::new_readonly(sysvar::clock::id(), false),
            ],
            data: vec![1, 2, 3],
        };
        let ix1 = Instruction {
            program_id: program,
            accounts: vec![
                AccountMeta::new(writable, false),
                AccountMeta::new_readonly(readonly, false),
            ],
            data: vec![4, 5],
        };
        let tx = sign_ml_dsa_transaction(&[ix0, ix1], &kp, Hash::new_unique()).unwrap();
        assert!(verify_ml_dsa_envelope(&tx.message, &tx.signatures));
    }

    #[test]
    fn carrier_not_last_rejected() {
        // A carrier that is not the final instruction is not recognized.
        let kp = MlDsaKeypair::new().unwrap();
        let tx =
            sign_ml_dsa_transaction(&transfer_ixs(&kp.address()), &kp, Hash::new_unique()).unwrap();
        let carrier = tx.message.instructions.last().unwrap().clone();
        let carrier_ix = Instruction {
            program_id: ml_dsa_program::id(),
            accounts: vec![],
            data: carrier.data.clone(),
        };
        // carrier first, transfer second -> last instruction is not the carrier.
        let ixs = vec![carrier_ix, system_instruction::transfer(&kp.address(), &Pubkey::new_unique(), 1)];
        let msg = Message::new_with_blockhash(&ixs, Some(&kp.address()), &tx.message.recent_blockhash);
        assert!(!verify_ml_dsa_envelope(&msg, &tx.signatures));
    }

    #[test]
    fn mismatched_dedup_id_rejected() {
        let kp = MlDsaKeypair::new().unwrap();
        let mut tx =
            sign_ml_dsa_transaction(&transfer_ixs(&kp.address()), &kp, Hash::new_unique()).unwrap();
        // A different placeholder signature for the same effective transaction is
        // rejected by check (d), blocking status-cache id malleability.
        tx.signatures[0] = Signature::from([7u8; 64]);
        assert!(!verify_ml_dsa_envelope(&tx.message, &tx.signatures));
    }

    #[test]
    fn multiple_required_signatures_rejected() {
        let kp = MlDsaKeypair::new().unwrap();
        let mut tx =
            sign_ml_dsa_transaction(&transfer_ixs(&kp.address()), &kp, Hash::new_unique()).unwrap();
        // Envelope scope is a single signer; a message claiming two is rejected.
        tx.message.header.num_required_signatures = 2;
        assert!(!verify_ml_dsa_envelope(&tx.message, &tx.signatures));
    }

    #[test]
    fn verifies_through_public_transaction_api() {
        // Drive the actual replay entry points, not just the helper: legacy
        // `Transaction` and `VersionedTransaction` must accept a valid envelope and
        // reject a forged one via `verify_and_hash_message`.
        use crate::transaction::VersionedTransaction;
        let kp = MlDsaKeypair::new().unwrap();
        let tx =
            sign_ml_dsa_transaction(&transfer_ixs(&kp.address()), &kp, Hash::new_unique()).unwrap();
        assert!(tx.verify_and_hash_message().is_ok());
        assert!(VersionedTransaction::from(tx.clone())
            .verify_and_hash_message()
            .is_ok());

        // Forge: swap the signer to an unrelated key so the address binding fails.
        let mut forged = tx;
        forged.message.account_keys[0] = Keypair::new().pubkey();
        assert!(forged.verify_and_hash_message().is_err());
        assert!(VersionedTransaction::from(forged)
            .verify_and_hash_message()
            .is_err());
    }

    #[test]
    fn wrong_signer_address_rejected() {
        let kp = MlDsaKeypair::new().unwrap();
        let ixs = transfer_ixs(&kp.address());
        let mut tx = sign_ml_dsa_transaction(&ixs, &kp, Hash::new_unique()).unwrap();
        // Replace the fee-payer/signer with an unrelated key; the carrier's
        // sha256(pubkey) no longer matches account_keys[0].
        tx.message.account_keys[0] = Keypair::new().pubkey();
        assert!(!verify_ml_dsa_envelope(&tx.message, &tx.signatures));
    }

    #[test]
    fn carrier_lifted_to_other_transaction_rejected() {
        // A valid proof from tx A must not authorize a different tx B.
        let kp = MlDsaKeypair::new().unwrap();
        let tx_a =
            sign_ml_dsa_transaction(&transfer_ixs(&kp.address()), &kp, Hash::new_unique()).unwrap();

        // Build B (different recipient/amount) and splice A's carrier onto it.
        let recipient = Pubkey::new_unique();
        let core_b = vec![system_instruction::transfer(&kp.address(), &recipient, 999)];
        let carrier_a = tx_a.message.instructions.last().unwrap().clone();
        // Reconstruct a full message: core B + A's carrier, same payer.
        let mut b_ixs: Vec<Instruction> = core_b;
        // Rebuild as raw compiled message reusing A's carrier bytes.
        let carrier_ix = Instruction {
            program_id: ml_dsa_program::id(),
            accounts: vec![],
            data: carrier_a.data.clone(),
        };
        b_ixs.push(carrier_ix);
        let msg_b = Message::new_with_blockhash(&b_ixs, Some(&kp.address()), &Hash::new_unique());
        assert!(!verify_ml_dsa_envelope(&msg_b, &tx_a.signatures));
    }

    #[test]
    fn non_ml_dsa_transaction_is_inert() {
        // An ordinary tx (no carrier) is never accepted by the envelope path.
        let payer = Keypair::new();
        let ix = system_instruction::transfer(&payer.pubkey(), &Pubkey::new_unique(), 1);
        let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &Hash::new_unique());
        let tx = Transaction::new(&[&payer], msg.clone(), Hash::new_unique());
        assert!(!verify_ml_dsa_envelope(&tx.message, &tx.signatures));
    }
}
