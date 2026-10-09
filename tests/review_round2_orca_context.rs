//! Log account context must remain tied to the actual nested swap invocation.
use sol_shred_sdk::{
    core::{account_dispatcher::fill_accounts_with_owned_keys, events::*},
    instr::{orca_whirlpool::discriminators::SWAP_V2, program_ids::ORCA_WHIRLPOOL_PROGRAM_ID},
    pubkey::Pubkey,
};
use std::collections::HashMap;
use yellowstone_grpc_proto::prelude::{
    CompiledInstruction, InnerInstruction, InnerInstructions, Message, Transaction,
    TransactionStatusMeta,
};

fn fixture() -> (
    Vec<Pubkey>,
    Vec<Pubkey>,
    Option<Transaction>,
    TransactionStatusMeta,
) {
    let mut a: Vec<_> = (0..15).map(|_| Pubkey::new_unique()).collect();
    let mut b: Vec<_> = (0..15).map(|_| Pubkey::new_unique()).collect();
    let memo = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr"
        .parse()
        .unwrap();
    a[2] = memo;
    b[2] = memo;
    let keys: Vec<_> = std::iter::once(ORCA_WHIRLPOOL_PROGRAM_ID)
        .chain(a.iter().copied())
        .chain(b.iter().copied())
        .chain(std::iter::once(Pubkey::new_unique()))
        .map(|key| key.to_bytes().to_vec())
        .collect();
    let mut data = SWAP_V2.to_vec();
    data.extend_from_slice(&100u64.to_le_bytes());
    data.extend_from_slice(&80u64.to_le_bytes());
    data.extend_from_slice(&0u128.to_le_bytes());
    data.extend_from_slice(&[1, 1, 0]);
    let transaction = Some(Transaction {
        message: Some(Message {
            account_keys: keys,
            instructions: vec![CompiledInstruction {
                program_id_index: 31,
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    });
    let meta = TransactionStatusMeta {
        inner_instructions: vec![InnerInstructions {
            index: 0,
            instructions: [1u8, 16]
                .into_iter()
                .map(|start| InnerInstruction {
                    program_id_index: 0,
                    accounts: (start..start + 15).collect(),
                    data: data.clone(),
                    stack_height: Some(2),
                })
                .collect(),
        }],
        ..Default::default()
    };
    (a, b, transaction, meta)
}

fn log(pool: Pubkey) -> DexEvent {
    let mut data = vec![0; 113];
    data[..32].copy_from_slice(pool.as_ref());
    data[32] = 1;
    data[65..73].copy_from_slice(&100u64.to_le_bytes());
    data[73..81].copy_from_slice(&80u64.to_le_bytes());
    sol_shred_sdk::logs::orca_whirlpool::parse_traded_from_data(&data, Default::default()).unwrap()
}

#[test]
fn nested_orca_swaps_fill_each_pools_own_accounts_in_either_invoke_order() {
    let (a, b, tx, meta) = fixture();
    for positions in [vec![(0, 0), (0, 1)], vec![(0, 1), (0, 0)]] {
        let invokes = HashMap::from([(ORCA_WHIRLPOOL_PROGRAM_ID, positions)]);
        for accounts in [&a, &b] {
            let mut event = log(accounts[4]);
            fill_accounts_with_owned_keys(&mut event, &meta, &tx, &invokes);
            let DexEvent::OrcaWhirlpoolSwap(event) = event else {
                panic!()
            };
            assert_eq!(event.whirlpool, accounts[4]);
            assert_eq!(event.token_mint_a, accounts[5]);
            assert_eq!(event.token_vault_a, accounts[8]);
            assert_eq!(event.token_authority, accounts[3]);
            assert_eq!((event.input_amount, event.output_amount), (100, 80));
        }
    }
}

#[test]
fn repeated_nested_pool_requires_unique_known_authority() {
    let (a, b, mut tx, meta) = fixture();
    // Both CPIs now reference the same pool, but different users and vault context.
    tx.as_mut().unwrap().message.as_mut().unwrap().account_keys[20] = a[4].to_bytes().to_vec();
    let invokes = HashMap::from([(ORCA_WHIRLPOOL_PROGRAM_ID, vec![(0, 0), (0, 1)])]);
    let mut event = log(a[4]);
    fill_accounts_with_owned_keys(&mut event, &meta, &tx, &invokes);
    let DexEvent::OrcaWhirlpoolSwap(mut unresolved) = event else {
        panic!()
    };
    assert_eq!(unresolved.token_mint_a, Pubkey::default());
    unresolved.token_authority = b[3];
    let mut event = DexEvent::OrcaWhirlpoolSwap(unresolved);
    fill_accounts_with_owned_keys(&mut event, &meta, &tx, &invokes);
    let DexEvent::OrcaWhirlpoolSwap(event) = event else {
        panic!()
    };
    assert_eq!(event.token_mint_a, b[5]);
    assert_eq!(event.token_vault_a, b[8]);
}
