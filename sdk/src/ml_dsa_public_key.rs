//! ML-DSA-44 (FIPS 204) public-key newtype and its on-chain address derivation.
//!
//! This is the post-quantum analogue of the 32-byte
//! [`solana_program::pubkey::Pubkey`], kept as a SEPARATE type (like
//! [`crate::ml_dsa_signature::MlDsaSignature`] beside the Ed25519 `Signature`).
//! An ML-DSA-44 public key is 1312 bytes (~41x larger than the 32-byte
//! `Pubkey`), so it cannot BE the account address: addresses must stay 32 bytes
//! for accounts-db, PDAs, and base58 display. Instead the address is the
//! SHA-256 hash of the public key (see [`crate::ml_dsa_keypair::ml_dsa_address`]),
//! and the full key travels inside the transaction body (see
//! [`crate::ml_dsa_transaction`]). This newtype is the typed, base58-displayable,
//! serde-serializable wrapper pairing a 1312-byte key with that derivation.

#![cfg(feature = "full")]

use {
    crate::ml_dsa_keypair::ml_dsa_address,
    fips204::ml_dsa_44::{PK_LEN, SIG_LEN},
    serde::{
        de::{self, SeqAccess, Visitor},
        Deserialize, Deserializer, Serialize, Serializer,
    },
    solana_program::pubkey::Pubkey,
    std::{fmt, str::FromStr},
    thiserror::Error,
};

/// Number of bytes in an ML-DSA-44 (FIPS 204) public key.
pub const ML_DSA_PUBLIC_KEY_BYTES: usize = PK_LEN; // 1312

/// Upper bound on the base58 length of a 1312-byte public key. The standard
/// `len * 138 / 100 + 1` bound (≈1811) sits comfortably above the ~1792 chars a
/// real key encodes to, so a valid key never trips the length guard while an
/// over-long garbage string is rejected before decoding.
const MAX_BASE58_LEN: usize = ML_DSA_PUBLIC_KEY_BYTES * 138 / 100 + 1;

/// An ML-DSA-44 public key: a fixed 1312-byte value that derives a 32-byte
/// on-chain [`Pubkey`] address via SHA-256, with base58 display and
/// bincode/serde serialization. The post-quantum analogue of the 32-byte
/// `Pubkey`, kept separate because a 1312-byte key cannot be an account address.
///
/// Deliberately NOT `Copy` (unlike `Pubkey`) so the 1312-byte payload is never
/// duplicated implicitly.
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct MlDsaPublicKey([u8; ML_DSA_PUBLIC_KEY_BYTES]);

impl MlDsaPublicKey {
    /// Public-key length in bytes (FIPS 204 ML-DSA-44).
    pub const PUBLIC_KEY_BYTES: usize = ML_DSA_PUBLIC_KEY_BYTES;

    /// Wrap raw public-key bytes, validating the length is exactly 1312. The
    /// bytes are NOT checked to be a well-formed ML-DSA-44 key (FIPS 204
    /// `pkDecode` accepts any 1312-byte input); a malformed key simply makes
    /// [`Self::verify`] return `false`. This differs from
    /// [`crate::ml_dsa_keypair::MlDsaKeypair::from_bytes`], which validates the
    /// key up front because it also owns the matching secret.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ParseMlDsaPublicKeyError> {
        let array: [u8; ML_DSA_PUBLIC_KEY_BYTES] = bytes
            .try_into()
            .map_err(|_| ParseMlDsaPublicKeyError::WrongSize)?;
        Ok(Self(array))
    }

    /// The raw 1312-byte public key.
    pub fn to_bytes(&self) -> [u8; ML_DSA_PUBLIC_KEY_BYTES] {
        self.0
    }

    /// Borrow the raw 1312-byte public key.
    pub fn as_bytes(&self) -> &[u8; ML_DSA_PUBLIC_KEY_BYTES] {
        &self.0
    }

    /// The 32-byte on-chain account address, `sha256(public_key)`. Delegates to
    /// the single canonical [`ml_dsa_address`] helper so the binding can never
    /// drift from the signer / sigverify / RPC paths. Recomputed on each call
    /// (not cached) so it can never disagree with the bytes; cache at the call
    /// site if it is ever hot.
    pub fn address(&self) -> Pubkey {
        ml_dsa_address(&self.0)
    }

    /// Verify `signature` over `message` against this public key, using an EMPTY
    /// FIPS 204 context (matching [`crate::ml_dsa_keypair::MlDsaKeypair::sign`]).
    /// Returns `false` if the key fails to parse or the signature does not verify.
    pub fn verify(&self, message: &[u8], signature: &[u8; SIG_LEN]) -> bool {
        use fips204::{
            ml_dsa_44::PublicKey,
            traits::{SerDes, Verifier},
        };
        match PublicKey::try_from_bytes(self.0) {
            Ok(public) => public.verify(message, signature, &[]),
            Err(_) => false,
        }
    }
}

impl From<[u8; ML_DSA_PUBLIC_KEY_BYTES]> for MlDsaPublicKey {
    /// Length is guaranteed by the array type; like [`MlDsaPublicKey::from_bytes`]
    /// the bytes are not checked to be a well-formed ML-DSA-44 key.
    fn from(bytes: [u8; ML_DSA_PUBLIC_KEY_BYTES]) -> Self {
        Self(bytes)
    }
}

impl AsRef<[u8]> for MlDsaPublicKey {
    fn as_ref(&self) -> &[u8] {
        &self.0[..]
    }
}

// Excludes signature verification, which is a separate pass; there are no
// indices or bounds to range-check, so the default no-op impl is correct.
impl crate::sanitize::Sanitize for MlDsaPublicKey {}

impl fmt::Debug for MlDsaPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", bs58::encode(self.as_ref()).into_string())
    }
}

impl fmt::Display for MlDsaPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", bs58::encode(self.as_ref()).into_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseMlDsaPublicKeyError {
    #[error("string decoded to wrong size for an ML-DSA-44 public key")]
    WrongSize,
    #[error("failed to decode base58 string to an ML-DSA-44 public key")]
    Invalid,
}

impl FromStr for MlDsaPublicKey {
    type Err = ParseMlDsaPublicKeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > MAX_BASE58_LEN {
            return Err(ParseMlDsaPublicKeyError::WrongSize);
        }
        let bytes = bs58::decode(s)
            .into_vec()
            .map_err(|_| ParseMlDsaPublicKeyError::Invalid)?;
        Self::from_bytes(&bytes)
    }
}

impl Serialize for MlDsaPublicKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0[..])
    }
}

struct MlDsaPublicKeyVisitor;

impl<'de> Visitor<'de> for MlDsaPublicKeyVisitor {
    type Value = MlDsaPublicKey;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{ML_DSA_PUBLIC_KEY_BYTES} public-key bytes")
    }

    // Byte-oriented formats (e.g. bincode) hand us the whole slice at once.
    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<MlDsaPublicKey, E> {
        MlDsaPublicKey::from_bytes(v).map_err(|_| de::Error::invalid_length(v.len(), &self))
    }

    // Formats with no native byte type (e.g. JSON) hand us a sequence of u8.
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<MlDsaPublicKey, A::Error> {
        let mut bytes = [0u8; ML_DSA_PUBLIC_KEY_BYTES];
        for (i, slot) in bytes.iter_mut().enumerate() {
            *slot = seq
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(i, &self))?;
        }
        // Reject an over-long sequence rather than silently dropping the tail,
        // mirroring the exact-length check on the visit_bytes path.
        if seq.next_element::<u8>()?.is_some() {
            return Err(de::Error::invalid_length(
                ML_DSA_PUBLIC_KEY_BYTES + 1,
                &self,
            ));
        }
        Ok(MlDsaPublicKey(bytes))
    }
}

impl<'de> Deserialize<'de> for MlDsaPublicKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_bytes(MlDsaPublicKeyVisitor)
    }
}

#[cfg(test)]
static_assertions::const_assert_eq!(MlDsaPublicKey::PUBLIC_KEY_BYTES, 1312);

#[cfg(test)]
mod tests {
    use {super::*, crate::ml_dsa_keypair::MlDsaKeypair};

    fn sample_keypair() -> MlDsaKeypair {
        MlDsaKeypair::generate().unwrap()
    }

    #[test]
    fn test_public_key_bytes_is_1312() {
        assert_eq!(MlDsaPublicKey::PUBLIC_KEY_BYTES, 1312);
        assert_eq!(ML_DSA_PUBLIC_KEY_BYTES, 1312);
    }

    #[test]
    fn test_address_is_deterministic_sha256() {
        // Address derivation must be deterministic and agree with the single
        // canonical ml_dsa_address helper and the keypair's own address.
        let kp = sample_keypair();
        let pk = MlDsaPublicKey::from(*kp.public_key_bytes());
        assert_eq!(pk.address(), kp.address());
        assert_eq!(pk.address(), ml_dsa_address(kp.public_key_bytes()));
        // Same bytes => same address, every time.
        let pk2 = MlDsaPublicKey::from_bytes(kp.public_key_bytes()).unwrap();
        assert_eq!(pk2.address(), pk.address());
        // Different key => different address (address() actually depends on its
        // input, not a constant).
        let other = MlDsaPublicKey::from(*sample_keypair().public_key_bytes());
        assert_ne!(other.address(), pk.address());
    }

    #[test]
    fn test_verify_matches_keypair() {
        let kp = sample_keypair();
        let pk = MlDsaPublicKey::from(*kp.public_key_bytes());
        let msg = b"epic1-3 public key";
        let sig = kp.sign(msg).unwrap();
        assert!(pk.verify(msg, &sig));
        assert!(!pk.verify(b"tampered", &sig));
        // A valid signature from a DIFFERENT keypair must be rejected — verify
        // proves "this key authorized this message", not just message integrity.
        let other = sample_keypair();
        let other_sig = other.sign(msg).unwrap();
        assert!(!pk.verify(msg, &other_sig));
    }

    #[test]
    fn test_base58_round_trip_no_truncation() {
        let pk = MlDsaPublicKey::from(*sample_keypair().public_key_bytes());
        let encoded = pk.to_string();
        // 1312 bytes base58-encode to ~1792 chars — far past the 32-byte Pubkey
        // cap (44), so this proves there is no truncation/overflow.
        assert!(
            encoded.len() > 1500 && encoded.len() <= MAX_BASE58_LEN,
            "unexpected base58 length {}",
            encoded.len()
        );
        let decoded: MlDsaPublicKey = encoded.parse().unwrap();
        assert_eq!(decoded, pk);
    }

    #[test]
    fn test_bincode_round_trip() {
        let pk = MlDsaPublicKey::from(*sample_keypair().public_key_bytes());
        let bytes = bincode::serialize(&pk).unwrap();
        let restored: MlDsaPublicKey = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored, pk);
    }

    #[test]
    fn test_json_round_trip() {
        let pk = MlDsaPublicKey::from(*sample_keypair().public_key_bytes());
        let json = serde_json::to_string(&pk).unwrap();
        let restored: MlDsaPublicKey = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, pk);
    }

    #[test]
    fn test_fixed_byte_patterns_round_trip() {
        // Attacker-chosen bytes (all-0xFF and all-0x00) instead of a real key,
        // so byte-order / high-bit / base58 leading-zero bugs cannot hide behind
        // structured content. The 0x00 case pins base58 leading-zero handling.
        for pattern in [
            [0xFFu8; ML_DSA_PUBLIC_KEY_BYTES],
            [0x00u8; ML_DSA_PUBLIC_KEY_BYTES],
        ] {
            let pk = MlDsaPublicKey::from(pattern);
            assert_eq!(pk.to_string().parse::<MlDsaPublicKey>().unwrap(), pk);
            let bin = bincode::serialize(&pk).unwrap();
            assert_eq!(bincode::deserialize::<MlDsaPublicKey>(&bin).unwrap(), pk);
            let json = serde_json::to_string(&pk).unwrap();
            assert_eq!(serde_json::from_str::<MlDsaPublicKey>(&json).unwrap(), pk);
        }
    }

    #[test]
    fn test_bincode_framing_is_len_prefixed() {
        // serialize_bytes => bincode writes an 8-byte length prefix + 1312 bytes.
        let pk = MlDsaPublicKey::from(*sample_keypair().public_key_bytes());
        assert_eq!(
            bincode::serialize(&pk).unwrap().len(),
            8 + ML_DSA_PUBLIC_KEY_BYTES
        );
    }

    #[test]
    fn test_from_bytes_rejects_wrong_length() {
        // A 32-byte value (Ed25519 Pubkey size) is the realistic mistake.
        assert_eq!(
            MlDsaPublicKey::from_bytes(&[0u8; 32]),
            Err(ParseMlDsaPublicKeyError::WrongSize)
        );
        assert!(MlDsaPublicKey::from_bytes(&[0u8; ML_DSA_PUBLIC_KEY_BYTES]).is_ok());
    }

    #[test]
    fn test_from_str_rejects_overlong_and_invalid() {
        let too_long = "1".repeat(MAX_BASE58_LEN + 1);
        assert_eq!(
            too_long.parse::<MlDsaPublicKey>(),
            Err(ParseMlDsaPublicKeyError::WrongSize)
        );
        // Valid length budget but not base58 ("0", "O", "I", "l" are excluded).
        assert_eq!(
            "0OIl".parse::<MlDsaPublicKey>(),
            Err(ParseMlDsaPublicKeyError::Invalid)
        );
    }

    #[test]
    fn test_json_sequence_length_enforced() {
        // Neither an over-long nor a short JSON array is accepted (no silent
        // truncation/padding) — exercises both visit_seq length guards.
        let overlong = format!("[{}]", vec!["0"; ML_DSA_PUBLIC_KEY_BYTES + 1].join(","));
        assert!(serde_json::from_str::<MlDsaPublicKey>(&overlong).is_err());
        let short = format!("[{}]", vec!["0"; ML_DSA_PUBLIC_KEY_BYTES - 1].join(","));
        assert!(serde_json::from_str::<MlDsaPublicKey>(&short).is_err());
    }
}
