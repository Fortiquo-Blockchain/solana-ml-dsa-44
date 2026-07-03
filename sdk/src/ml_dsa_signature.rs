//! ML-DSA-44 (FIPS 204) signature newtype.
//!
//! This is the post-quantum analogue of the 64-byte Ed25519
//! [`crate::signature::Signature`], kept as a SEPARATE type for the same reason
//! [`crate::ml_dsa_keypair::MlDsaKeypair`] is separate from the Ed25519
//! `Keypair`: an ML-DSA-44 signature is 2420 bytes (~38x larger than Ed25519's
//! 64), so it cannot BE the universal `Signature`, which is the fixed 64-byte
//! value the runtime, gossip, shreds, and RPC use as a transaction id. The raw
//! 2420-byte signatures still travel inside the transaction body as
//! `[u8; SIG_LEN]` (see [`crate::ml_dsa_envelope`]); this newtype is the
//! typed, base58-displayable, serde-serializable wrapper around one of them.

#![cfg(feature = "full")]

use {
    fips204::ml_dsa_44::SIG_LEN,
    serde::{
        de::{self, SeqAccess, Visitor},
        Deserialize, Deserializer, Serialize, Serializer,
    },
    std::{fmt, str::FromStr},
    thiserror::Error,
};

/// Number of bytes in an ML-DSA-44 (FIPS 204) signature.
pub const ML_DSA_SIGNATURE_BYTES: usize = SIG_LEN; // 2420

/// Upper bound on the base58 length of a 2420-byte signature. The standard
/// `len * 138 / 100 + 1` bound (≈3340) is comfortably above the ~3300 chars a
/// real signature encodes to, so a valid signature never trips the length
/// guard, while a garbage over-long string is rejected before decoding.
const MAX_BASE58_LEN: usize = ML_DSA_SIGNATURE_BYTES * 138 / 100 + 1;

/// An ML-DSA-44 signature: a fixed 2420-byte value with base58 display and
/// bincode/serde serialization, mirroring the Ed25519
/// [`crate::signature::Signature`] API at the post-quantum size.
///
/// Deliberately NOT `Copy` (unlike the 64-byte `Signature`) so the 2420-byte
/// payload is never duplicated implicitly.
#[repr(transparent)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct MlDsaSignature([u8; ML_DSA_SIGNATURE_BYTES]);

impl MlDsaSignature {
    /// Signature length in bytes (FIPS 204 ML-DSA-44).
    pub const SIGNATURE_BYTES: usize = ML_DSA_SIGNATURE_BYTES;

    /// Wrap raw signature bytes, validating the length is exactly 2420.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ParseMlDsaSignatureError> {
        let array: [u8; ML_DSA_SIGNATURE_BYTES] = bytes
            .try_into()
            .map_err(|_| ParseMlDsaSignatureError::WrongSize)?;
        Ok(Self(array))
    }

    /// The raw 2420-byte signature.
    pub fn to_bytes(&self) -> [u8; ML_DSA_SIGNATURE_BYTES] {
        self.0
    }

    /// Borrow the raw 2420-byte signature.
    pub fn as_bytes(&self) -> &[u8; ML_DSA_SIGNATURE_BYTES] {
        &self.0
    }
}

impl From<[u8; ML_DSA_SIGNATURE_BYTES]> for MlDsaSignature {
    fn from(bytes: [u8; ML_DSA_SIGNATURE_BYTES]) -> Self {
        Self(bytes)
    }
}

impl AsRef<[u8]> for MlDsaSignature {
    fn as_ref(&self) -> &[u8] {
        &self.0[..]
    }
}

// Excludes signature verification, which is a separate pass; there are no
// indices or bounds to range-check, so the default no-op impl is correct.
impl crate::sanitize::Sanitize for MlDsaSignature {}

// `[u8; 2420]` is too large to derive AbiExample, so provide it manually (gated
// on specialization like the other hand-written impls, e.g. `Meta`/`FeeStructure`).
// Needed so this type can be embedded in gossip's frozen-abi `CrdsValue`.
#[cfg(RUSTC_WITH_SPECIALIZATION)]
impl ::solana_frozen_abi::abi_example::AbiExample for MlDsaSignature {
    fn example() -> Self {
        Self([0u8; ML_DSA_SIGNATURE_BYTES])
    }
}

impl fmt::Debug for MlDsaSignature {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", bs58::encode(self.as_ref()).into_string())
    }
}

impl fmt::Display for MlDsaSignature {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", bs58::encode(self.as_ref()).into_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseMlDsaSignatureError {
    #[error("string decoded to wrong size for an ML-DSA-44 signature")]
    WrongSize,
    #[error("failed to decode base58 string to an ML-DSA-44 signature")]
    Invalid,
}

impl FromStr for MlDsaSignature {
    type Err = ParseMlDsaSignatureError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > MAX_BASE58_LEN {
            return Err(ParseMlDsaSignatureError::WrongSize);
        }
        let bytes = bs58::decode(s)
            .into_vec()
            .map_err(|_| ParseMlDsaSignatureError::Invalid)?;
        Self::from_bytes(&bytes)
    }
}

impl Serialize for MlDsaSignature {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0[..])
    }
}

struct MlDsaSignatureVisitor;

impl<'de> Visitor<'de> for MlDsaSignatureVisitor {
    type Value = MlDsaSignature;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{ML_DSA_SIGNATURE_BYTES} signature bytes")
    }

    // Byte-oriented formats (e.g. bincode) hand us the whole slice at once.
    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<MlDsaSignature, E> {
        MlDsaSignature::from_bytes(v).map_err(|_| de::Error::invalid_length(v.len(), &self))
    }

    // Formats with no native byte type (e.g. JSON) hand us a sequence of u8.
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<MlDsaSignature, A::Error> {
        let mut bytes = [0u8; ML_DSA_SIGNATURE_BYTES];
        for (i, slot) in bytes.iter_mut().enumerate() {
            *slot = seq
                .next_element()?
                .ok_or_else(|| de::Error::invalid_length(i, &self))?;
        }
        // Reject an over-long sequence rather than silently dropping the tail,
        // mirroring the exact-length check on the visit_bytes path.
        if seq.next_element::<u8>()?.is_some() {
            return Err(de::Error::invalid_length(ML_DSA_SIGNATURE_BYTES + 1, &self));
        }
        Ok(MlDsaSignature(bytes))
    }
}

impl<'de> Deserialize<'de> for MlDsaSignature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_bytes(MlDsaSignatureVisitor)
    }
}

#[cfg(test)]
static_assertions::const_assert_eq!(MlDsaSignature::SIGNATURE_BYTES, 2420);

#[cfg(test)]
mod tests {
    use {super::*, crate::ml_dsa_keypair::MlDsaKeypair};

    fn sample() -> MlDsaSignature {
        let kp = MlDsaKeypair::generate().unwrap();
        MlDsaSignature::from(kp.sign(b"epic1-2 signature").unwrap())
    }

    #[test]
    fn test_signature_bytes_is_2420() {
        assert_eq!(MlDsaSignature::SIGNATURE_BYTES, 2420);
        assert_eq!(ML_DSA_SIGNATURE_BYTES, 2420);
    }

    #[test]
    fn test_base58_round_trip_no_truncation() {
        let sig = sample();
        let encoded = sig.to_string();
        // 2420 bytes base58-encode to ~3300 chars — nowhere near the 64-byte
        // Ed25519 cap (88), so this proves there is no truncation/overflow.
        assert!(
            encoded.len() > 3000 && encoded.len() <= MAX_BASE58_LEN,
            "unexpected base58 length {}",
            encoded.len()
        );
        let decoded: MlDsaSignature = encoded.parse().unwrap();
        assert_eq!(decoded, sig);
    }

    #[test]
    fn test_bincode_round_trip() {
        let sig = sample();
        let bytes = bincode::serialize(&sig).unwrap();
        let restored: MlDsaSignature = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored, sig);
    }

    #[test]
    fn test_json_round_trip() {
        let sig = sample();
        let json = serde_json::to_string(&sig).unwrap();
        let restored: MlDsaSignature = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, sig);
    }

    #[test]
    fn test_from_bytes_rejects_wrong_length() {
        assert_eq!(
            MlDsaSignature::from_bytes(&[0u8; 64]),
            Err(ParseMlDsaSignatureError::WrongSize)
        );
        assert!(MlDsaSignature::from_bytes(&[0u8; ML_DSA_SIGNATURE_BYTES]).is_ok());
    }

    #[test]
    fn test_from_str_rejects_overlong_and_invalid() {
        let too_long = "1".repeat(MAX_BASE58_LEN + 1);
        assert_eq!(
            too_long.parse::<MlDsaSignature>(),
            Err(ParseMlDsaSignatureError::WrongSize)
        );
        // Valid length budget but not base58 ("0", "O", "I", "l" are not in the alphabet).
        assert_eq!(
            "0OIl".parse::<MlDsaSignature>(),
            Err(ParseMlDsaSignatureError::Invalid)
        );
    }

    #[test]
    fn test_fixed_byte_patterns_round_trip() {
        // Attacker-chosen bytes (all-0xFF and all-0x00) instead of a random
        // signature, so byte-order / high-bit / base58 leading-zero bugs cannot
        // hide behind random content. The 0x00 case specifically pins that
        // base58 round-trips leading-zero bytes without collapsing/truncating.
        for pattern in [[0xFFu8; ML_DSA_SIGNATURE_BYTES], [0x00u8; ML_DSA_SIGNATURE_BYTES]] {
            let sig = MlDsaSignature::from(pattern);
            assert_eq!(sig.to_string().parse::<MlDsaSignature>().unwrap(), sig);
            let bin = bincode::serialize(&sig).unwrap();
            assert_eq!(bincode::deserialize::<MlDsaSignature>(&bin).unwrap(), sig);
            let json = serde_json::to_string(&sig).unwrap();
            assert_eq!(serde_json::from_str::<MlDsaSignature>(&json).unwrap(), sig);
        }
    }

    #[test]
    fn test_bincode_framing_is_len_prefixed() {
        // serialize_bytes => bincode writes an 8-byte length prefix + 2420 bytes.
        // Pinning this locks the on-wire size so a future serialize change cannot
        // silently alter the framing of anything embedding this type.
        let sig = sample();
        assert_eq!(
            bincode::serialize(&sig).unwrap().len(),
            8 + ML_DSA_SIGNATURE_BYTES
        );
    }

    #[test]
    fn test_json_sequence_length_enforced() {
        // Neither an over-long nor a short JSON array is accepted (no silent
        // truncation/padding) — exercises both visit_seq length guards.
        let overlong = format!("[{}]", vec!["0"; ML_DSA_SIGNATURE_BYTES + 1].join(","));
        assert!(serde_json::from_str::<MlDsaSignature>(&overlong).is_err());
        let short = format!("[{}]", vec!["0"; ML_DSA_SIGNATURE_BYTES - 1].join(","));
        assert!(serde_json::from_str::<MlDsaSignature>(&short).is_err());
    }
}
