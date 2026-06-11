//! ML-DSA-44 (FIPS 204) keypair and post-quantum address derivation for Phase 1
//! transaction signing.
//!
//! This is deliberately SEPARATE from the Ed25519 [`crate::signer::keypair::Keypair`]
//! and the [`crate::signer::Signer`] trait, which are left untouched (they still
//! sign votes, gossip, and shreds). An ML-DSA-44 public key is 1312 bytes, so it
//! cannot be a 32-byte account address; instead the address is the SHA-256 hash of
//! the public key, and the full key travels inside the transaction (see
//! [`crate::ml_dsa_transaction`]).

#![cfg(feature = "full")]

use {
    fips204::{
        ml_dsa_44::{self, PrivateKey, PK_LEN, SIG_LEN, SK_LEN},
        traits::{SerDes, Signer as MlDsaSigner},
    },
    solana_program::{hash::hash, pubkey::Pubkey},
    std::{
        error,
        io::{Read, Write},
        path::Path,
    },
};

/// On-disk keypair file size: public key (1312) followed by secret key (2560).
pub const ML_DSA_KEYPAIR_FILE_BYTES: usize = PK_LEN + SK_LEN; // 3872

/// Derive the 32-byte account address from an ML-DSA-44 public key:
/// `address = sha256(public_key_bytes)`. SHA-256 is exactly 32 bytes, so the
/// address is an ordinary [`Pubkey`] and accounts-db / PDAs / base58 display are
/// unchanged. This is the single canonical helper used by the signer, sigverify,
/// and RPC so the binding can never drift.
pub fn ml_dsa_address(public_key_bytes: &[u8]) -> Pubkey {
    Pubkey::from(hash(public_key_bytes).to_bytes())
}

/// An ML-DSA-44 keypair. Stores the raw key bytes (the `fips204` key types do not
/// expose a cheap clone or a "derive public from private", so we keep both).
pub struct MlDsaKeypair {
    public_bytes: [u8; PK_LEN],
    secret_bytes: [u8; SK_LEN],
    address: Pubkey,
}

impl MlDsaKeypair {
    /// Generate a fresh ML-DSA-44 keypair.
    pub fn new() -> Result<Self, Box<dyn error::Error>> {
        let (public, secret) = ml_dsa_44::try_keygen().map_err(|e| e.to_string())?;
        Ok(Self::from_parts(public.into_bytes(), secret.into_bytes()))
    }

    /// Deterministically derive an ML-DSA-44 keypair from a 32-byte seed (the
    /// FIPS 204 key-generation seed ξ, via `KG::keygen_from_seed`). This lets
    /// `solana-keygen --scheme mldsa44` give the same BIP39-mnemonic recovery
    /// story as Ed25519: the same seed always reproduces the same key.
    pub fn from_seed(xi: &[u8; 32]) -> Self {
        use fips204::{ml_dsa_44::KG, traits::KeyGen};
        let (public, secret) = KG::keygen_from_seed(xi);
        Self::from_parts(public.into_bytes(), secret.into_bytes())
    }

    fn from_parts(public_bytes: [u8; PK_LEN], secret_bytes: [u8; SK_LEN]) -> Self {
        let address = ml_dsa_address(&public_bytes);
        Self {
            public_bytes,
            secret_bytes,
            address,
        }
    }

    /// The 32-byte account address = `sha256(public_key)`.
    pub fn address(&self) -> Pubkey {
        self.address
    }

    /// Alias so call sites read like the Ed25519 `Signer::pubkey()`.
    pub fn pubkey(&self) -> Pubkey {
        self.address
    }

    /// The raw 1312-byte ML-DSA public key (carried inside the transaction).
    pub fn public_key_bytes(&self) -> &[u8; PK_LEN] {
        &self.public_bytes
    }

    /// Sign `message` with the ML-DSA-44 secret key, using an EMPTY FIPS 204
    /// context (matching the Phase 0 precompile and the reference JS sample).
    pub fn sign(&self, message: &[u8]) -> Result<[u8; SIG_LEN], Box<dyn error::Error>> {
        let secret = PrivateKey::try_from_bytes(self.secret_bytes).map_err(|e| e.to_string())?;
        secret.try_sign(message, &[]).map_err(|e| e.to_string().into())
    }

    /// Verify `signature` over `message` against this keypair's public key,
    /// using an EMPTY FIPS 204 context (matching [`Self::sign`]). Returns `false`
    /// if the signature does not verify; the "public key failed to parse" arm is
    /// unreachable for keypairs built via [`Self::from_bytes`]/[`Self::new`]/
    /// [`Self::from_seed`], which all validate the public key up front.
    pub fn verify(&self, message: &[u8], signature: &[u8; SIG_LEN]) -> bool {
        use fips204::{ml_dsa_44::PublicKey, traits::Verifier};
        match PublicKey::try_from_bytes(self.public_bytes) {
            Ok(public) => public.verify(message, signature, &[]),
            Err(_) => false,
        }
    }

    /// Serialize to the on-disk layout `[public(1312) || secret(2560)]`.
    pub fn to_bytes(&self) -> [u8; ML_DSA_KEYPAIR_FILE_BYTES] {
        let mut out = [0u8; ML_DSA_KEYPAIR_FILE_BYTES];
        out[..PK_LEN].copy_from_slice(&self.public_bytes);
        out[PK_LEN..].copy_from_slice(&self.secret_bytes);
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Box<dyn error::Error>> {
        if bytes.len() != ML_DSA_KEYPAIR_FILE_BYTES {
            return Err(format!(
                "expected {ML_DSA_KEYPAIR_FILE_BYTES}-byte ML-DSA keypair, got {}",
                bytes.len()
            )
            .into());
        }
        let public_bytes: [u8; PK_LEN] = bytes[..PK_LEN].try_into()?;
        let secret_bytes: [u8; SK_LEN] = bytes[PK_LEN..].try_into()?;
        // Validate both halves parse, so a corrupt public half can never later
        // surface as a mere "verification failed" in [`Self::verify`].
        PrivateKey::try_from_bytes(secret_bytes).map_err(|e| e.to_string())?;
        fips204::ml_dsa_44::PublicKey::try_from_bytes(public_bytes).map_err(|e| e.to_string())?;
        Ok(Self::from_parts(public_bytes, secret_bytes))
    }
}

/// Read an ML-DSA-44 keypair from a JSON byte array (same on-disk shape as the
/// Ed25519 keypair file, just 3872 bytes instead of 64).
pub fn read_ml_dsa_keypair<R: Read>(reader: &mut R) -> Result<MlDsaKeypair, Box<dyn error::Error>> {
    let bytes: Vec<u8> = serde_json::from_reader(reader)?;
    MlDsaKeypair::from_bytes(&bytes)
}

pub fn read_ml_dsa_keypair_file<F: AsRef<Path>>(
    path: F,
) -> Result<MlDsaKeypair, Box<dyn error::Error>> {
    let mut file = std::fs::File::open(path.as_ref())?;
    read_ml_dsa_keypair(&mut file)
}

pub fn write_ml_dsa_keypair<W: Write>(
    keypair: &MlDsaKeypair,
    writer: &mut W,
) -> Result<String, Box<dyn error::Error>> {
    let serialized = serde_json::to_string(&keypair.to_bytes().to_vec())?;
    writer.write_all(serialized.as_bytes())?;
    Ok(serialized)
}

pub fn write_ml_dsa_keypair_file<F: AsRef<Path>>(
    keypair: &MlDsaKeypair,
    outfile: F,
) -> Result<String, Box<dyn error::Error>> {
    let outfile = outfile.as_ref();
    if let Some(outdir) = outfile.parent() {
        std::fs::create_dir_all(outdir)?;
    }
    let mut file = std::fs::File::create(outfile)?;
    write_ml_dsa_keypair(keypair, &mut file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_is_hash_of_pubkey() {
        let kp = MlDsaKeypair::new().unwrap();
        assert_eq!(kp.address(), ml_dsa_address(kp.public_key_bytes()));
    }

    #[test]
    fn test_keypair_file_roundtrip() {
        let kp = MlDsaKeypair::new().unwrap();
        let bytes = kp.to_bytes();
        assert_eq!(bytes.len(), ML_DSA_KEYPAIR_FILE_BYTES);
        let restored = MlDsaKeypair::from_bytes(&bytes).unwrap();
        assert_eq!(restored.address(), kp.address());
        assert_eq!(restored.public_key_bytes(), kp.public_key_bytes());
    }

    #[test]
    fn test_sign_is_verifiable() {
        use fips204::{
            ml_dsa_44::PublicKey,
            traits::{SerDes, Verifier},
        };
        let kp = MlDsaKeypair::new().unwrap();
        let msg = b"hello post-quantum";
        let sig = kp.sign(msg).unwrap();
        let pk = PublicKey::try_from_bytes(*kp.public_key_bytes()).unwrap();
        assert!(pk.verify(msg, &sig, &[]));
        assert!(!pk.verify(b"different", &sig, &[]));
    }

    #[test]
    fn test_from_seed_is_deterministic_and_recoverable() {
        let xi = [7u8; 32];
        let a = MlDsaKeypair::from_seed(&xi);
        let b = MlDsaKeypair::from_seed(&xi);
        // Same seed => same key/address (the recovery guarantee).
        assert_eq!(a.address(), b.address());
        assert_eq!(a.public_key_bytes(), b.public_key_bytes());
        // A different seed => a different key.
        let c = MlDsaKeypair::from_seed(&[8u8; 32]);
        assert_ne!(a.address(), c.address());
        // The derived key signs and self-verifies.
        let sig = a.sign(b"recover me").unwrap();
        assert!(a.verify(b"recover me", &sig));
        assert!(!a.verify(b"tampered", &sig));
    }
}
