//! Phase 1 ML-DSA-44 transaction format — a post-quantum-signed user transaction
//! that coexists with the Ed25519 [`crate::transaction::Transaction`].
//!
//! Wire layout (distinguished by a `0x00` lead byte, which is never a valid
//! Ed25519 transaction since those start with a signature count >= 1):
//!
//! ```text
//! [0x00][shortu16 sig_count][2420 B ml-dsa sigs...]
//!       [shortu16 pk_count][1312 B signer ml-dsa pubkeys...]
//!       [bincode(Message)]
//! ```
//!
//! Binding: for each required signer `i`, `account_keys[i] == sha256(signer_pubkeys[i])`
//! and `signatures[i]` verifies under `signer_pubkeys[i]` over `message.serialize()`.
//!
//! Phase 1 is restricted to a SINGLE required signer (the fee payer) and a legacy
//! [`Message`]; both are enforced in [`MlDsaTransaction::deserialize`].

#![cfg(feature = "full")]

use {
    crate::{
        ml_dsa_keypair::{ml_dsa_address, MlDsaKeypair},
        signature::Signature,
    },
    fips204::{
        ml_dsa_44::{PublicKey, PK_LEN, SIG_LEN},
        traits::{SerDes, Verifier},
    },
    solana_program::{hash::hash, message::Message, sanitize::SanitizeError},
};

/// First byte of an ML-DSA transaction packet. `0x00` decodes to a zero-length
/// signature vector under the Ed25519 parser, which that path always rejects, so
/// it is a collision-free discriminator.
pub const ML_DSA_TX_MARKER: u8 = 0x00;

/// INVARIANTS — enforced at the trust boundary by [`MlDsaTransaction::deserialize`]
/// plus [`MlDsaTransaction::verify_offchain`], NOT by the type (the fields are `pub`
/// for wire (de)serialization):
///   * `signatures.len() == signer_pubkeys.len() == num_required_signatures` (Phase 1: 1)
///   * `account_keys[i] == sha256(signer_pubkeys[i])` (address binding)
///   * `signatures[i]` verifies under `signer_pubkeys[i]` over `message.serialize()`
///
/// An instance is only trustworthy after `verify_offchain()` returns `true`; construct
/// via `sign` or `deserialize`.
#[derive(Clone)]
pub struct MlDsaTransaction {
    pub message: Message,
    pub signatures: Vec<[u8; SIG_LEN]>,
    pub signer_pubkeys: Vec<[u8; PK_LEN]>,
}

/// Append `val` as a compact-u16 (mirrors `short_vec::ShortU16` serialization).
fn encode_shortu16(mut val: u16, out: &mut Vec<u8>) {
    loop {
        let mut elem = (val & 0x7f) as u8;
        val >>= 7;
        if val == 0 {
            out.push(elem);
            break;
        }
        elem |= 0x80;
        out.push(elem);
    }
}

impl MlDsaTransaction {
    /// Sign `message` with the ML-DSA signers (Phase 1: exactly one, the fee
    /// payer). The signer at index `i` must equal `account_keys[i]`.
    pub fn sign(
        message: Message,
        signers: &[&MlDsaKeypair],
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let num_required = message.header.num_required_signatures as usize;
        if num_required != 1 {
            return Err("Phase 1 ML-DSA transactions support exactly one signer".into());
        }
        if signers.len() != num_required {
            return Err("number of signers does not match num_required_signatures".into());
        }
        let message_bytes = message.serialize();
        let mut signatures = Vec::with_capacity(num_required);
        let mut signer_pubkeys = Vec::with_capacity(num_required);
        for (i, signer) in signers.iter().enumerate() {
            if message.account_keys.get(i) != Some(&signer.address()) {
                return Err("signer does not match account_keys[i]".into());
            }
            signatures.push(signer.sign(&message_bytes)?);
            signer_pubkeys.push(*signer.public_key_bytes());
        }
        Ok(Self {
            message,
            signatures,
            signer_pubkeys,
        })
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            1 + 3
                + self.signatures.len() * SIG_LEN
                + 3
                + self.signer_pubkeys.len() * PK_LEN
                + 256,
        );
        out.push(ML_DSA_TX_MARKER);
        encode_shortu16(self.signatures.len() as u16, &mut out);
        for sig in &self.signatures {
            out.extend_from_slice(sig);
        }
        encode_shortu16(self.signer_pubkeys.len() as u16, &mut out);
        for pk in &self.signer_pubkeys {
            out.extend_from_slice(pk);
        }
        out.extend_from_slice(&self.message.serialize());
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, SanitizeError> {
        use solana_program::short_vec::decode_shortu16_len;

        if bytes.first() != Some(&ML_DSA_TX_MARKER) {
            return Err(SanitizeError::InvalidValue);
        }
        let mut cursor = 1usize;

        let (sig_count, n) =
            decode_shortu16_len(&bytes[cursor..]).map_err(|_| SanitizeError::InvalidValue)?;
        cursor += n;
        // Phase 1 allows exactly one signer. Reject other counts BEFORE allocating, so a
        // hostile count (up to 65535) on an attacker-controlled packet can't force a large
        // speculative allocation in the pre-verification path (sigverify / banking / RPC).
        if sig_count != 1 {
            return Err(SanitizeError::InvalidValue);
        }
        let mut signatures = Vec::with_capacity(sig_count);
        for _ in 0..sig_count {
            let end = cursor
                .checked_add(SIG_LEN)
                .ok_or(SanitizeError::IndexOutOfBounds)?;
            let sig: [u8; SIG_LEN] = bytes
                .get(cursor..end)
                .ok_or(SanitizeError::IndexOutOfBounds)?
                .try_into()
                .map_err(|_| SanitizeError::InvalidValue)?;
            signatures.push(sig);
            cursor = end;
        }

        let (pk_count, n) =
            decode_shortu16_len(&bytes[cursor..]).map_err(|_| SanitizeError::InvalidValue)?;
        cursor += n;
        if pk_count != 1 {
            return Err(SanitizeError::InvalidValue);
        }
        let mut signer_pubkeys = Vec::with_capacity(pk_count);
        for _ in 0..pk_count {
            let end = cursor
                .checked_add(PK_LEN)
                .ok_or(SanitizeError::IndexOutOfBounds)?;
            let pk: [u8; PK_LEN] = bytes
                .get(cursor..end)
                .ok_or(SanitizeError::IndexOutOfBounds)?
                .try_into()
                .map_err(|_| SanitizeError::InvalidValue)?;
            signer_pubkeys.push(pk);
            cursor = end;
        }

        let remaining = bytes.get(cursor..).ok_or(SanitizeError::IndexOutOfBounds)?;
        let message: Message =
            bincode::deserialize(remaining).map_err(|_| SanitizeError::InvalidValue)?;
        // Reject trailing bytes after the message: it must consume the entire remainder,
        // so two distinct wire encodings can't map to the same transaction (bincode's
        // `deserialize` otherwise silently ignores trailing bytes).
        if message.serialize().len() != remaining.len() {
            return Err(SanitizeError::InvalidValue);
        }

        // Phase 1: legacy message, exactly one required signer. (sig_count and pk_count
        // were already constrained to 1 above, before allocating.)
        if message.header.num_required_signatures as usize != 1 {
            return Err(SanitizeError::InvalidValue);
        }

        Ok(Self {
            message,
            signatures,
            signer_pubkeys,
        })
    }

    /// Verify, for every required signer: (a) the ML-DSA signature is valid, and
    /// (b) `sha256(signer_pubkey) == account_keys[i]`. This single implementation
    /// is shared by the client, sigverify, and RPC preflight.
    pub fn verify_offchain(&self) -> bool {
        let num_required = self.message.header.num_required_signatures as usize;
        if self.signatures.len() != num_required || self.signer_pubkeys.len() != num_required {
            return false;
        }
        if self.message.account_keys.len() < num_required {
            return false;
        }
        let message_bytes = self.message.serialize();
        for i in 0..num_required {
            // (b) address binding.
            if ml_dsa_address(&self.signer_pubkeys[i]) != self.message.account_keys[i] {
                return false;
            }
            // (a) signature.
            let Ok(pubkey) = PublicKey::try_from_bytes(self.signer_pubkeys[i]) else {
                return false;
            };
            if !pubkey.verify(&message_bytes, &self.signatures[i], &[]) {
                return false;
            }
        }
        true
    }

    /// A synthetic 64-byte transaction id used as the `Signature` everywhere the
    /// runtime keys on one (status, blockstore, dedup). It is
    /// `sha256(all_ml_dsa_sigs) || sha256(message_bytes)` — derived via SHA-256, so
    /// distinct (signatures, message) pairs map to distinct ids (a literal replay of the
    /// same bytes intentionally yields the same id). It is an identifier, not a
    /// verifiable signature.
    pub fn synthetic_signature(&self) -> Signature {
        let mut sig_concat = Vec::with_capacity(self.signatures.len() * SIG_LEN);
        for sig in &self.signatures {
            sig_concat.extend_from_slice(sig);
        }
        let sigs_hash = hash(&sig_concat);
        let msg_hash = hash(&self.message.serialize());
        let mut id = [0u8; 64];
        id[..32].copy_from_slice(&sigs_hash.to_bytes());
        id[32..].copy_from_slice(&msg_hash.to_bytes());
        Signature::from(id)
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::ml_dsa_keypair::MlDsaKeypair,
        solana_program::{message::Message, pubkey::Pubkey, system_instruction},
    };

    fn build() -> (MlDsaKeypair, MlDsaTransaction) {
        let payer = MlDsaKeypair::new().unwrap();
        let recipient = Pubkey::new_unique();
        let message = Message::new(
            &[system_instruction::transfer(&payer.address(), &recipient, 1_000)],
            Some(&payer.address()),
        );
        let tx = MlDsaTransaction::sign(message, &[&payer]).unwrap();
        (payer, tx)
    }

    #[test]
    fn test_roundtrip_and_verify() {
        let (_payer, tx) = build();
        let bytes = tx.serialize();
        assert_eq!(bytes[0], ML_DSA_TX_MARKER);
        let decoded = MlDsaTransaction::deserialize(&bytes).unwrap();
        assert!(decoded.verify_offchain());
        // Synthetic id survives the round-trip.
        assert_eq!(decoded.synthetic_signature(), tx.synthetic_signature());
    }

    #[test]
    fn test_tampered_signature_rejected() {
        let (_payer, mut tx) = build();
        tx.signatures[0][100] ^= 0xff;
        assert!(!tx.verify_offchain());
    }

    #[test]
    fn test_address_binding_enforced() {
        let (_payer, mut tx) = build();
        // Swap the fee-payer account key so it no longer equals sha256(pubkey).
        tx.message.account_keys[0] = Pubkey::new_unique();
        assert!(!tx.verify_offchain());
    }

    #[test]
    fn test_wrong_signer_pubkey_rejected() {
        let (_payer, mut tx) = build();
        // A different (valid) pubkey breaks both the signature and the binding.
        let other = MlDsaKeypair::new().unwrap();
        tx.signer_pubkeys[0] = *other.public_key_bytes();
        assert!(!tx.verify_offchain());
    }

    #[test]
    fn test_synthetic_signature_deterministic() {
        let (_payer, tx) = build();
        assert_eq!(tx.synthetic_signature(), tx.synthetic_signature());
    }

    #[test]
    fn test_deserialize_rejects_malformed() {
        // empty, marker-only, wrong marker, truncated, and trailing-garbage inputs
        assert!(MlDsaTransaction::deserialize(&[]).is_err());
        assert!(MlDsaTransaction::deserialize(&[ML_DSA_TX_MARKER]).is_err());

        let (_payer, tx) = build();
        let good = tx.serialize();

        let mut wrong_marker = good.clone();
        wrong_marker[0] = 1;
        assert!(MlDsaTransaction::deserialize(&wrong_marker).is_err());

        assert!(MlDsaTransaction::deserialize(&good[..good.len() - 1]).is_err());

        let mut trailing = good.clone();
        trailing.push(0x42);
        assert!(MlDsaTransaction::deserialize(&trailing).is_err());
    }

    #[test]
    fn test_deserialize_rejects_huge_count_without_allocating() {
        // A 0x00 packet declaring sig_count = 65535 must be rejected immediately (before
        // any large allocation), not OOM. shortu16(65535) = [0xff, 0xff, 0x03].
        let bytes = [ML_DSA_TX_MARKER, 0xff, 0xff, 0x03];
        assert!(matches!(
            MlDsaTransaction::deserialize(&bytes),
            Err(SanitizeError::InvalidValue)
        ));
    }

    #[test]
    fn test_sign_rejects_multi_signer() {
        // Two signers -> num_required_signatures == 2 -> rejected (Phase 1 = single signer).
        let a = MlDsaKeypair::new().unwrap();
        let b = MlDsaKeypair::new().unwrap();
        let recipient = Pubkey::new_unique();
        let message = Message::new(
            &[
                system_instruction::transfer(&a.address(), &recipient, 1),
                system_instruction::transfer(&b.address(), &recipient, 1),
            ],
            Some(&a.address()),
        );
        assert_eq!(message.header.num_required_signatures, 2);
        assert!(MlDsaTransaction::sign(message, &[&a, &b]).is_err());
    }

    #[test]
    fn test_valid_signature_over_wrong_message_rejected() {
        // A structurally valid signature that was produced over a DIFFERENT message must
        // fail — proves the signature is bound to this message, not just that random
        // byte-flips fail. Keep account_keys[0] (the binding) intact.
        let (payer, tx) = build();
        let recipient = Pubkey::new_unique();
        let mut tx2 = tx.clone();
        tx2.message = Message::new(
            &[system_instruction::transfer(&payer.address(), &recipient, 999)],
            Some(&payer.address()),
        );
        assert_eq!(tx2.message.account_keys[0], payer.address()); // binding still holds
        assert!(!tx2.verify_offchain());
    }
}
