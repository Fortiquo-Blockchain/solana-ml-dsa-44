use {
    assert_matches::assert_matches,
    fips204::ml_dsa_44,
    solana_program_test::*,
    solana_sdk::{
        ml_dsa_instruction::new_ml_dsa_instruction,
        signature::Signer,
        transaction::{Transaction, TransactionError},
    },
};

#[tokio::test]
async fn test_success() {
    let mut context = ProgramTest::default().start_with_context().await;

    let client = &mut context.banks_client;
    let payer = &context.payer;
    let recent_blockhash = context.last_blockhash;

    let (public_key, private_key) = ml_dsa_44::try_keygen().unwrap();
    let message_arr = b"hello";
    let instruction = new_ml_dsa_instruction(&private_key, public_key, message_arr);

    let transaction = Transaction::new_signed_with_payer(
        &[instruction],
        Some(&payer.pubkey()),
        &[payer],
        recent_blockhash,
    );

    assert_matches!(client.process_transaction(transaction).await, Ok(()));
}

#[tokio::test]
async fn test_failure() {
    let mut context = ProgramTest::default().start_with_context().await;

    let client = &mut context.banks_client;
    let payer = &context.payer;
    let recent_blockhash = context.last_blockhash;

    let (public_key, private_key) = ml_dsa_44::try_keygen().unwrap();
    let message_arr = b"hello";
    let mut instruction = new_ml_dsa_instruction(&private_key, public_key, message_arr);

    // Bumping the signature count without supplying the data makes verification fail.
    instruction.data[0] += 1;

    let transaction = Transaction::new_signed_with_payer(
        &[instruction],
        Some(&payer.pubkey()),
        &[payer],
        recent_blockhash,
    );

    assert_matches!(
        client.process_transaction(transaction).await,
        Err(BanksClientError::TransactionError(
            TransactionError::InvalidAccountIndex
        ))
    );
}
