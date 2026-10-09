//! Genuine signatures validate the wire, not projected log/CPI payload layouts.
use base64::{engine::general_purpose::STANDARD, Engine};
use sol_shred_sdk::{
    core::events::EventMetadata, hash::Hash, message::VersionedMessage,
    transaction::VersionedTransaction, Pubkey,
};
use solana_instruction::Instruction;
use solana_keypair::{Keypair, Signer};
fn check(body: &[u8], expected: bool) {
    let mut payload = vec![0; 12];
    payload.extend_from_slice(body);
    let payer = Keypair::new_from_array([91; 32]);
    let instruction = Instruction {
        program_id: Pubkey::new_from_array([92; 32]),
        accounts: vec![],
        data: payload,
    };
    for legacy in [false, true] {
        let message = if legacy {
            VersionedMessage::Legacy(solana_message::Message::new_with_blockhash(
                &[instruction.clone()],
                Some(&payer.pubkey()),
                &Hash::new_from_array([93; 32]),
            ))
        } else {
            VersionedMessage::V0(
                sol_shred_sdk::message::v0::Message::try_compile(
                    &payer.pubkey(),
                    &[instruction.clone()],
                    &[],
                    Hash::new_from_array([93; 32]),
                )
                .unwrap(),
            )
        };
        let signed = VersionedTransaction::try_new(message, &[&payer]).unwrap();
        let tx: VersionedTransaction =
            wincode::deserialize_exact(&wincode::serialize(&signed).unwrap()).unwrap();
        assert!(tx.signatures[0].verify(payer.pubkey().as_ref(), &tx.message.serialize()));
        let data = &tx.message.instructions()[0].data;
        let metadata = EventMetadata {
            signature: tx.signatures[0],
            ..Default::default()
        };
        assert_eq!(
            sol_shred_sdk::logs::pump::parse_create_from_data(data, metadata.clone()).is_some(),
            expected
        );
        let disc = sol_shred_sdk::instr::pump_inner::discriminators::CREATE_TOKEN_EVENT;
        assert_eq!(
            sol_shred_sdk::instr::pump_inner::parse_pumpfun_inner_instruction(
                &disc, data, metadata, false
            )
            .is_some(),
            expected
        );
        let mut log_data = sol_shred_sdk::logs::pump::CREATE_EVENT
            .to_le_bytes()
            .to_vec();
        log_data.extend_from_slice(data);
        assert_eq!(
            sol_shred_sdk::logs::pump::parse_log(
                &format!("Program data: {}", STANDARD.encode(log_data)),
                tx.signatures[0],
                1,
                0,
                None,
                0,
                false
            )
            .is_some(),
            expected
        );
    }
}
#[test]
fn signed_create_rejects_partial_fields_and_noncanonical_bools() {
    for len in 0..=253 {
        check(
            &vec![0; len],
            matches!(len, 96 | 201 | 202 | 234 | 242 | 250 | 251 | 252),
        );
    }
    for offset in [200, 201, 250] {
        for invalid in [2, 255] {
            let mut body = vec![0; 252];
            body[offset] = invalid;
            check(&body, false);
        }
        let mut body = vec![0; 252];
        body[offset] = 1;
        check(&body, true);
    }
}
