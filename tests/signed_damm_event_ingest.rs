//! Signed extraction fixtures, not bank execution or authenticated CPI metadata.
use sol_shred_sdk::{
    hash::Hash, message::VersionedMessage, transaction::VersionedTransaction, DexEvent, Pubkey,
};
use solana_instruction::Instruction;
use solana_keypair::{Keypair, Signer};

const DISCS: [[u8; 8]; 4] = [
    [198, 182, 183, 52, 97, 12, 49, 56],
    [129, 91, 188, 3, 246, 52, 185, 249],
    [104, 233, 237, 122, 199, 191, 121, 85],
    [218, 86, 147, 200, 235, 188, 215, 231],
];
fn body(kind: usize) -> Vec<u8> {
    let mut b = vec![];
    for i in 0..if kind == 0 || kind == 2 { 3 } else { 4 } {
        b.extend_from_slice(&[10 + kind as u8 * 4 + i; 32]);
    }
    if kind != 0 {
        b.push(1);
    }
    for i in 0..if kind == 0 {
        2
    } else if kind == 2 {
        3
    } else {
        1
    } {
        b.extend_from_slice(&(u64::MAX - i).to_le_bytes());
    }
    if kind == 2 {
        b.extend_from_slice(&u128::MAX.to_le_bytes());
        b.extend_from_slice(&(u128::MAX - 1).to_le_bytes());
    }
    b
}
fn instruction(kind: usize, body: &[u8]) -> Instruction {
    Instruction {
        program_id: sol_shred_sdk::instr::program_ids::METEORA_DAMM_V2_PROGRAM_ID,
        accounts: vec![],
        data: [
            &[228, 69, 165, 46, 81, 203, 154, 29][..],
            &DISCS[kind],
            body,
        ]
        .concat(),
    }
}
fn signed(ix: &[Instruction], legacy: bool) -> VersionedTransaction {
    let payer = Keypair::new_from_array([37; 32]);
    let message = if legacy {
        VersionedMessage::Legacy(solana_message::Message::new_with_blockhash(
            ix,
            Some(&payer.pubkey()),
            &Hash::new_from_array([38; 32]),
        ))
    } else {
        VersionedMessage::V0(
            sol_shred_sdk::message::v0::Message::try_compile(
                &payer.pubkey(),
                ix,
                &[],
                Hash::new_from_array([38; 32]),
            )
            .unwrap(),
        )
    };
    let tx = VersionedTransaction::try_new(message, &[&payer]).unwrap();
    let wire = wincode::serialize(&tx).unwrap();
    let decoded: VersionedTransaction = wincode::deserialize_exact(&wire).unwrap();
    assert_eq!(decoded.signatures.len(), 1);
    assert_eq!(decoded.message.header().num_required_signatures, 1);
    assert!(decoded.signatures[0].verify(payer.pubkey().as_ref(), &decoded.message.serialize()));
    decoded
}
fn parse(tx: &VersionedTransaction) -> Vec<DexEvent> {
    let mut out = vec![];
    sol_shred_sdk::parse_transaction_dex_events(tx, tx.signatures[0], 42, 8, 999, &mut out);
    out
}

#[test]
fn signed_legacy_and_v0_damm_fee_reward_wire_preserves_full_width_fields() {
    let ix: Vec<_> = (0..4).map(|kind| instruction(kind, &body(kind))).collect();
    for legacy in [true, false] {
        let tx = signed(&ix, legacy);
        let events = parse(&tx);
        assert_eq!(events.len(), 4);
        for (kind, event) in events.iter().enumerate() {
            let key = |i| Pubkey::new_from_array([10 + kind as u8 * 4 + i; 32]);
            match event {
                DexEvent::MeteoraDammV2ClaimPositionFee(e) => {
                    assert_eq!((e.pool, e.position, e.owner), (key(0), key(1), key(2)));
                    assert_eq!((e.fee_a_claimed, e.fee_b_claimed), (u64::MAX, u64::MAX - 1));
                }
                DexEvent::MeteoraDammV2InitializeReward(e) => {
                    assert_eq!(
                        (e.pool, e.reward_mint, e.funder, e.creator),
                        (key(0), key(1), key(2), key(3))
                    );
                    assert_eq!((e.reward_index, e.reward_duration), (1, u64::MAX));
                }
                DexEvent::MeteoraDammV2FundReward(e) => {
                    assert_eq!((e.pool, e.funder, e.mint_reward), (key(0), key(1), key(2)));
                    assert_eq!(
                        (
                            e.amount,
                            e.transfer_fee_excluded_amount_in,
                            e.reward_duration_end
                        ),
                        (u64::MAX, u64::MAX - 1, u64::MAX - 2)
                    );
                    assert_eq!(
                        (e.pre_reward_rate, e.post_reward_rate),
                        (u128::MAX, u128::MAX - 1)
                    );
                }
                DexEvent::MeteoraDammV2ClaimReward(e) => {
                    assert_eq!(
                        (e.pool, e.position, e.owner, e.mint_reward),
                        (key(0), key(1), key(2), key(3))
                    );
                    assert_eq!((e.reward_index, e.total_reward), (1, u64::MAX));
                }
                _ => panic!("unexpected variant"),
            }
            assert_eq!(event.metadata().signature, tx.signatures[0]);
            assert_eq!(
                (
                    event.metadata().slot,
                    event.metadata().tx_index,
                    event.metadata().grpc_recv_us
                ),
                (42, 8, 999)
            );
            assert_eq!(
                event.metadata().recent_blockhash,
                Some(tx.message.recent_blockhash().to_string())
            );
        }
    }
}

#[test]
fn valid_signatures_do_not_make_malformed_or_wrong_program_events_parse() {
    for kind in 0..4 {
        let valid = instruction(kind, &body(kind));
        let mut short = valid.clone();
        short.data.pop();
        let mut long = valid.clone();
        long.data.push(0);
        let mut prefix = valid.clone();
        prefix.data[0] ^= 1;
        let mut wrong = valid.clone();
        wrong.program_id = Pubkey::new_from_array([99; 32]);
        for invalid in [short, long, prefix, wrong] {
            let tx = signed(&[invalid, valid.clone()], false);
            assert_eq!(
                parse(&tx).len(),
                1,
                "invalid event must not hide its valid neighbor"
            );
        }
    }
}
