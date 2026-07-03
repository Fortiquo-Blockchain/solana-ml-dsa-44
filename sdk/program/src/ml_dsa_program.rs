//! The ML-DSA-44 (FIPS 204) post-quantum signature native program.
//!
//! Mirrors [`crate::ed25519_program`]; the verification logic lives in
//! [`solana_sdk::ml_dsa_instruction`]. This program id is recognized by the
//! runtime as a precompile and is registered as an account at bank init.

crate::declare_id!("6CjqvizoLwkFdkutVsjsXjttQiNVfrZM18MdZw2VbNSs");
