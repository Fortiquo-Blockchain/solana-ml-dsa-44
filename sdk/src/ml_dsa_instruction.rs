//! Instructions for the [ML-DSA-44 native program][np].
//!
//! This is the post-quantum analogue of [`crate::ed25519_instruction`]. It
//! verifies ML-DSA-44 (NIST FIPS 204) signatures using the `fips204` crate.
//! The on-the-wire layout mirrors the ed25519 precompile exactly, only with
//! larger public-key (1312 B) and signature (2420 B) fields.
//!
//! [np]: https://csrc.nist.gov/pubs/fips/204/final

#![cfg(feature = "full")]

use {
    crate::{feature_set::FeatureSet, instruction::Instruction, precompiles::PrecompileError},
    bytemuck::{bytes_of, Pod, Zeroable},
    fips204::{
        ml_dsa_44::{PrivateKey, PublicKey, PK_LEN, SIG_LEN},
        traits::{SerDes, Signer, Verifier},
    },
};

pub const PUBKEY_SERIALIZED_SIZE: usize = PK_LEN; // 1312
pub const SIGNATURE_SERIALIZED_SIZE: usize = SIG_LEN; // 2420
pub const SIGNATURE_OFFSETS_SERIALIZED_SIZE: usize = 14;
// bytemuck requires structures to be aligned
pub const SIGNATURE_OFFSETS_START: usize = 2;
pub const DATA_START: usize = SIGNATURE_OFFSETS_SERIALIZED_SIZE + SIGNATURE_OFFSETS_START;

// Compile-time guard: the hand-written offsets size must match the struct layout.
const _: () = assert!(
    SIGNATURE_OFFSETS_SERIALIZED_SIZE == core::mem::size_of::<MlDsaSignatureOffsets>(),
);

// FIPS 204 signing/verification context. Kept empty so signatures are
// byte-compatible with `@noble/post-quantum` (the reference sample).
const ML_DSA_CONTEXT: &[u8] = &[];

#[derive(Default, Debug, Copy, Clone, Zeroable, Pod, Eq, PartialEq)]
#[repr(C)]
pub struct MlDsaSignatureOffsets {
    signature_offset: u16,             // offset to ml-dsa signature of 2420 bytes
    signature_instruction_index: u16,  // instruction index to find signature
    public_key_offset: u16,            // offset to public key of 1312 bytes
    public_key_instruction_index: u16, // instruction index to find public key
    message_data_offset: u16,          // offset to start of message data
    message_data_size: u16,            // size of message data
    message_instruction_index: u16,    // index of instruction data to get message data
}

/// Build an ML-DSA-44 precompile instruction that proves `message` was signed
/// by `private_key`. `public_key` is embedded so the precompile can verify it.
pub fn new_ml_dsa_instruction(
    private_key: &PrivateKey,
    public_key: PublicKey,
    message: &[u8],
) -> Instruction {
    // The offsets are u16, so the message must be addressable within that range.
    // (In practice PACKET_DATA_SIZE bounds it far tighter, but fail loudly here.)
    assert!(
        message.len() <= u16::MAX as usize,
        "ML-DSA precompile message too large to address with u16 offsets",
    );
    let signature: [u8; SIG_LEN] = private_key
        .try_sign(message, ML_DSA_CONTEXT)
        .expect("ML-DSA-44 signing should not fail");
    let pubkey: [u8; PK_LEN] = public_key.into_bytes();

    assert_eq!(pubkey.len(), PUBKEY_SERIALIZED_SIZE);
    assert_eq!(signature.len(), SIGNATURE_SERIALIZED_SIZE);

    let mut instruction_data = Vec::with_capacity(
        DATA_START
            .saturating_add(SIGNATURE_SERIALIZED_SIZE)
            .saturating_add(PUBKEY_SERIALIZED_SIZE)
            .saturating_add(message.len()),
    );

    let num_signatures: u8 = 1;
    let public_key_offset = DATA_START;
    let signature_offset = public_key_offset.saturating_add(PUBKEY_SERIALIZED_SIZE);
    let message_data_offset = signature_offset.saturating_add(SIGNATURE_SERIALIZED_SIZE);

    // add padding byte so that offset structure is aligned
    instruction_data.extend_from_slice(bytes_of(&[num_signatures, 0]));

    let offsets = MlDsaSignatureOffsets {
        signature_offset: signature_offset as u16,
        signature_instruction_index: u16::MAX,
        public_key_offset: public_key_offset as u16,
        public_key_instruction_index: u16::MAX,
        message_data_offset: message_data_offset as u16,
        message_data_size: message.len() as u16,
        message_instruction_index: u16::MAX,
    };

    instruction_data.extend_from_slice(bytes_of(&offsets));

    debug_assert_eq!(instruction_data.len(), public_key_offset);

    instruction_data.extend_from_slice(&pubkey);

    debug_assert_eq!(instruction_data.len(), signature_offset);

    instruction_data.extend_from_slice(&signature);

    debug_assert_eq!(instruction_data.len(), message_data_offset);

    instruction_data.extend_from_slice(message);

    Instruction {
        program_id: solana_sdk::ml_dsa_program::id(),
        accounts: vec![],
        data: instruction_data,
    }
}

pub fn verify(
    data: &[u8],
    instruction_datas: &[&[u8]],
    _feature_set: &FeatureSet,
) -> Result<(), PrecompileError> {
    if data.len() < SIGNATURE_OFFSETS_START {
        return Err(PrecompileError::InvalidInstructionDataSize);
    }
    let num_signatures = data[0] as usize;
    if num_signatures == 0 && data.len() > SIGNATURE_OFFSETS_START {
        return Err(PrecompileError::InvalidInstructionDataSize);
    }
    let expected_data_size = num_signatures
        .saturating_mul(SIGNATURE_OFFSETS_SERIALIZED_SIZE)
        .saturating_add(SIGNATURE_OFFSETS_START);
    // We do not check or use the byte at data[1]
    if data.len() < expected_data_size {
        return Err(PrecompileError::InvalidInstructionDataSize);
    }
    for i in 0..num_signatures {
        let start = i
            .saturating_mul(SIGNATURE_OFFSETS_SERIALIZED_SIZE)
            .saturating_add(SIGNATURE_OFFSETS_START);
        let end = start.saturating_add(SIGNATURE_OFFSETS_SERIALIZED_SIZE);

        // bytemuck wants structures aligned
        let offsets: &MlDsaSignatureOffsets = bytemuck::try_from_bytes(&data[start..end])
            .map_err(|_| PrecompileError::InvalidDataOffsets)?;

        // Parse out signature
        let signature = get_data_slice(
            data,
            instruction_datas,
            offsets.signature_instruction_index,
            offsets.signature_offset,
            SIGNATURE_SERIALIZED_SIZE,
        )?;
        let signature: &[u8; SIG_LEN] = signature
            .try_into()
            .map_err(|_| PrecompileError::InvalidSignature)?;

        // Parse out pubkey
        let pubkey = get_data_slice(
            data,
            instruction_datas,
            offsets.public_key_instruction_index,
            offsets.public_key_offset,
            PUBKEY_SERIALIZED_SIZE,
        )?;
        let pubkey: [u8; PK_LEN] = pubkey
            .try_into()
            .map_err(|_| PrecompileError::InvalidPublicKey)?;
        let public_key =
            PublicKey::try_from_bytes(pubkey).map_err(|_| PrecompileError::InvalidPublicKey)?;

        // Parse out message
        let message = get_data_slice(
            data,
            instruction_datas,
            offsets.message_instruction_index,
            offsets.message_data_offset,
            offsets.message_data_size as usize,
        )?;

        if !public_key.verify(message, signature, ML_DSA_CONTEXT) {
            return Err(PrecompileError::InvalidSignature);
        }
    }
    Ok(())
}

fn get_data_slice<'a>(
    data: &'a [u8],
    instruction_datas: &'a [&[u8]],
    instruction_index: u16,
    offset_start: u16,
    size: usize,
) -> Result<&'a [u8], PrecompileError> {
    let instruction = if instruction_index == u16::MAX {
        data
    } else {
        let signature_index = instruction_index as usize;
        if signature_index >= instruction_datas.len() {
            return Err(PrecompileError::InvalidDataOffsets);
        }
        instruction_datas[signature_index]
    };

    let start = offset_start as usize;
    let end = start.saturating_add(size);
    if end > instruction.len() {
        return Err(PrecompileError::InvalidDataOffsets);
    }

    Ok(&instruction[start..end])
}

#[cfg(test)]
pub mod test {
    use {
        super::*,
        crate::{
            feature_set::FeatureSet,
            hash::Hash,
            signature::{Keypair, Signer},
            transaction::Transaction,
        },
        fips204::ml_dsa_44,
        rand0_7::{thread_rng, Rng},
    };

    // Backing instruction buffer large enough to hold a full pubkey + signature
    // region, so the offset/bounds tests exercise get_data_slice rather than
    // tripping on a too-small fixture (ed25519's 100-byte buffer is smaller than a
    // single ML-DSA field).
    const TEST_INSTR_LEN: usize = DATA_START + PUBKEY_SERIALIZED_SIZE + SIGNATURE_SERIALIZED_SIZE;

    fn test_case(
        num_signatures: u16,
        offsets: &MlDsaSignatureOffsets,
    ) -> Result<(), PrecompileError> {
        assert_eq!(
            bytemuck::bytes_of(offsets).len(),
            SIGNATURE_OFFSETS_SERIALIZED_SIZE
        );

        let mut instruction_data = vec![0u8; DATA_START];
        instruction_data[0..SIGNATURE_OFFSETS_START].copy_from_slice(bytes_of(&num_signatures));
        instruction_data[SIGNATURE_OFFSETS_START..DATA_START].copy_from_slice(bytes_of(offsets));

        verify(
            &instruction_data,
            &[&[0u8; TEST_INSTR_LEN]],
            &FeatureSet::all_enabled(),
        )
    }

    #[test]
    fn test_invalid_offsets() {
        solana_logger::setup();

        // Truncated instruction data -> InvalidInstructionDataSize.
        let mut instruction_data = vec![0u8; DATA_START];
        let offsets = MlDsaSignatureOffsets::default();
        instruction_data[0..SIGNATURE_OFFSETS_START].copy_from_slice(bytes_of(&1u16));
        instruction_data[SIGNATURE_OFFSETS_START..DATA_START].copy_from_slice(bytes_of(&offsets));
        instruction_data.truncate(instruction_data.len() - 1);
        assert_eq!(
            verify(
                &instruction_data,
                &[&[0u8; TEST_INSTR_LEN]],
                &FeatureSet::all_enabled()
            ),
            Err(PrecompileError::InvalidInstructionDataSize)
        );

        // *_instruction_index = 1 is out of range for a single instruction.
        let offsets = MlDsaSignatureOffsets {
            signature_instruction_index: 1,
            ..MlDsaSignatureOffsets::default()
        };
        assert_eq!(test_case(1, &offsets), Err(PrecompileError::InvalidDataOffsets));

        let offsets = MlDsaSignatureOffsets {
            message_instruction_index: 1,
            ..MlDsaSignatureOffsets::default()
        };
        assert_eq!(test_case(1, &offsets), Err(PrecompileError::InvalidDataOffsets));

        let offsets = MlDsaSignatureOffsets {
            public_key_instruction_index: 1,
            ..MlDsaSignatureOffsets::default()
        };
        assert_eq!(test_case(1, &offsets), Err(PrecompileError::InvalidDataOffsets));
    }

    #[test]
    fn test_signature_offset() {
        let offsets = MlDsaSignatureOffsets {
            signature_offset: u16::MAX,
            ..MlDsaSignatureOffsets::default()
        };
        assert_eq!(test_case(1, &offsets), Err(PrecompileError::InvalidDataOffsets));

        // Straddling the end of the backing buffer.
        let offsets = MlDsaSignatureOffsets {
            signature_offset: (TEST_INSTR_LEN - SIGNATURE_SERIALIZED_SIZE + 1) as u16,
            ..MlDsaSignatureOffsets::default()
        };
        assert_eq!(test_case(1, &offsets), Err(PrecompileError::InvalidDataOffsets));
    }

    #[test]
    fn test_pubkey_offset() {
        let offsets = MlDsaSignatureOffsets {
            public_key_offset: u16::MAX,
            ..MlDsaSignatureOffsets::default()
        };
        assert_eq!(test_case(1, &offsets), Err(PrecompileError::InvalidDataOffsets));

        let offsets = MlDsaSignatureOffsets {
            public_key_offset: (TEST_INSTR_LEN - PUBKEY_SERIALIZED_SIZE + 1) as u16,
            ..MlDsaSignatureOffsets::default()
        };
        assert_eq!(test_case(1, &offsets), Err(PrecompileError::InvalidDataOffsets));
    }

    #[test]
    fn test_message_data_offsets() {
        let offsets = MlDsaSignatureOffsets {
            message_data_offset: u16::MAX,
            message_data_size: 1,
            ..MlDsaSignatureOffsets::default()
        };
        assert_eq!(test_case(1, &offsets), Err(PrecompileError::InvalidDataOffsets));

        let offsets = MlDsaSignatureOffsets {
            message_data_offset: (TEST_INSTR_LEN - 1) as u16,
            message_data_size: 1000,
            ..MlDsaSignatureOffsets::default()
        };
        assert_eq!(test_case(1, &offsets), Err(PrecompileError::InvalidDataOffsets));

        // start + size must not wrap (saturating arithmetic in get_data_slice).
        let offsets = MlDsaSignatureOffsets {
            message_data_offset: u16::MAX,
            message_data_size: u16::MAX,
            ..MlDsaSignatureOffsets::default()
        };
        assert_eq!(test_case(1, &offsets), Err(PrecompileError::InvalidDataOffsets));
    }

    #[test]
    fn test_ml_dsa() {
        solana_logger::setup();

        let (public_key, private_key) = ml_dsa_44::try_keygen().unwrap();
        let message_arr = b"hello";
        let mut instruction = new_ml_dsa_instruction(&private_key, public_key, message_arr);
        let mint_keypair = Keypair::new();
        let feature_set = FeatureSet::all_enabled();

        let tx = Transaction::new_signed_with_payer(
            &[instruction.clone()],
            Some(&mint_keypair.pubkey()),
            &[&mint_keypair],
            Hash::default(),
        );

        assert!(tx.verify_precompiles(&feature_set).is_ok());

        let index = loop {
            let index = thread_rng().gen_range(0, instruction.data.len());
            // byte 1 is not used, so this would not cause the verify to fail
            if index != 1 {
                break index;
            }
        };

        instruction.data[index] = instruction.data[index].wrapping_add(12);
        let tx = Transaction::new_signed_with_payer(
            &[instruction],
            Some(&mint_keypair.pubkey()),
            &[&mint_keypair],
            Hash::default(),
        );
        assert!(tx.verify_precompiles(&feature_set).is_err());
    }

    #[test]
    fn test_ml_dsa_wrong_message() {
        // A signature over message_a must not verify against a different, equal-length
        // message_b (guards the hand-written bool -> Err branch in verify()).
        let (public_key, private_key) = ml_dsa_44::try_keygen().unwrap();
        let message_a = b"correct horse battery staple";
        let message_b = b"Correct horse battery staple";
        assert_eq!(message_a.len(), message_b.len());

        let mut instruction = new_ml_dsa_instruction(&private_key, public_key, message_a);
        let message_offset = DATA_START + PUBKEY_SERIALIZED_SIZE + SIGNATURE_SERIALIZED_SIZE;
        instruction.data[message_offset..message_offset + message_b.len()]
            .copy_from_slice(message_b);

        assert_eq!(
            verify(&instruction.data, &[], &FeatureSet::all_enabled()),
            Err(PrecompileError::InvalidSignature)
        );
    }
}
