//! Partial ALT/account references must not become known economic route legs.
use sol_shred_sdk::{
    analyze_yellowstone_transaction_routes, instr::program_ids::RAYDIUM_CPMM_PROGRAM_ID, Pubkey,
};
use yellowstone_grpc_proto::prelude::{
    CompiledInstruction, Message, Transaction, TransactionStatusMeta,
};

#[test]
fn unresolved_swap_accounts_keep_the_invocation_unknown() {
    let keys: Vec<_> = (0..13)
        .map(|_| Pubkey::new_unique())
        .chain([RAYDIUM_CPMM_PROGRAM_ID])
        .collect();
    let mut data = sol_shred_sdk::instr::raydium_cpmm::discriminators::SWAP_BASE_IN.to_vec();
    data.extend_from_slice(&100u64.to_le_bytes());
    data.extend_from_slice(&50u64.to_le_bytes());
    let make = |accounts| Transaction {
        message: Some(Message {
            account_keys: keys.iter().map(|key| key.to_bytes().to_vec()).collect(),
            instructions: vec![CompiledInstruction {
                program_id_index: 13,
                accounts,
                data: data.clone(),
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    let valid: Vec<_> = (0..13).collect();
    assert_eq!(
        analyze_yellowstone_transaction_routes(
            &make(valid.clone()),
            &TransactionStatusMeta::default(),
            &[]
        )
        .legs
        .len(),
        1
    );
    for index in 0..13 {
        let mut accounts = valid.clone();
        accounts[index] = 255;
        let route = analyze_yellowstone_transaction_routes(
            &make(accounts),
            &TransactionStatusMeta::default(),
            &[],
        );
        assert!(
            route.legs.is_empty(),
            "unresolved account at {index} must not become a known swap"
        );
        assert_eq!(route.unknown_invocations.len(), 1);
    }
}
