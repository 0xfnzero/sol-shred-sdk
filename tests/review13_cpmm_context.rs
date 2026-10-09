use sol_shred_sdk::{
    core::{account_dispatcher::fill_accounts_with_owned_keys, events::RaydiumCpmmSwapEvent},
    grpc::program_ids::RAYDIUM_CPMM_PROGRAM,
    instr::raydium_cpmm::discriminators,
    DexEvent, Pubkey,
};
use std::collections::HashMap;
use yellowstone_grpc_proto::prelude::{
    CompiledInstruction, InnerInstruction, InnerInstructions, Message, Transaction,
    TransactionStatusMeta,
};

#[test]
fn cpmm_enrichment_matches_pool_direction_and_unique_account_context() {
    let mut keys: Vec<_> = (0..26).map(|_| Pubkey::new_unique()).collect();
    keys.push(RAYDIUM_CPMM_PROGRAM);
    let swap = |start, disc: [u8; 8]| CompiledInstruction {
        program_id_index: 26,
        accounts: (start..start + 13).collect(),
        data: [
            disc.to_vec(),
            100u64.to_le_bytes().to_vec(),
            1u64.to_le_bytes().to_vec(),
        ]
        .concat(),
    };
    for inner in [false, true] {
        for base_input in [false, true] {
            let disc = if base_input {
                discriminators::SWAP_BASE_IN
            } else {
                discriminators::SWAP_BASE_OUT
            };
            for ambiguous in [false, true] {
                let mut instructions = vec![swap(0, disc), swap(13, disc)];
                // An unrelated instruction sharing the pool must not match a swap.
                instructions.push(swap(0, discriminators::DEPOSIT));
                if ambiguous {
                    instructions.push(swap(0, disc));
                }
                let invokes: Vec<_> = (0..instructions.len())
                    .map(|index| {
                        if inner {
                            (0, index as i32)
                        } else {
                            (index as i32, -1)
                        }
                    })
                    .collect();
                let mut meta = TransactionStatusMeta::default();
                if inner {
                    meta.inner_instructions.push(InnerInstructions {
                        index: 0,
                        instructions: instructions
                            .iter()
                            .map(|ix| InnerInstruction {
                                program_id_index: ix.program_id_index,
                                accounts: ix.accounts.clone(),
                                data: ix.data.clone(),
                                ..Default::default()
                            })
                            .collect(),
                    });
                }
                let tx = Some(Transaction {
                    message: Some(Message {
                        account_keys: keys.iter().map(|k| k.to_bytes().to_vec()).collect(),
                        instructions: if inner { vec![] } else { instructions },
                        ..Default::default()
                    }),
                    ..Default::default()
                });
                let invokes = HashMap::from([(RAYDIUM_CPMM_PROGRAM, invokes)]);
                for pool in [keys[3], Pubkey::new_unique()] {
                    let mut event = DexEvent::RaydiumCpmmSwap(RaydiumCpmmSwapEvent {
                        pool_id: pool,
                        base_input,
                        input_amount: 77,
                        output_amount: 55,
                        ..Default::default()
                    });
                    fill_accounts_with_owned_keys(&mut event, &meta, &tx, &invokes);
                    let DexEvent::RaydiumCpmmSwap(event) = event else {
                        unreachable!()
                    };
                    assert_eq!(event.pool_id, pool);
                    assert_eq!((event.input_amount, event.output_amount), (77, 55));
                    if ambiguous || pool != keys[3] {
                        assert_eq!(event.input_token_mint, Pubkey::default());
                        assert_eq!(event.input_vault, Pubkey::default());
                    } else {
                        assert_eq!(event.input_token_mint, keys[10]);
                        assert_eq!(event.output_token_mint, keys[11]);
                        assert_eq!(event.input_vault, keys[6]);
                        assert_eq!(event.output_vault, keys[7]);
                    }
                }
            }
        }
    }
}

#[test]
fn repeated_cpmm_pool_requires_observed_payer_to_choose_the_right_swap() {
    let keys: Vec<_> = (0..26).map(|_| Pubkey::new_unique()).collect();
    let mut second: Vec<u8> = (13..26).collect();
    second[3] = 3;
    let tx = Some(Transaction {
        message: Some(Message {
            account_keys: keys.iter().map(|k| k.to_bytes().to_vec()).collect(),
            instructions: [(0..13).collect(), second]
                .into_iter()
                .map(|accounts| CompiledInstruction {
                    accounts,
                    data: [
                        discriminators::SWAP_BASE_IN.to_vec(),
                        100u64.to_le_bytes().to_vec(),
                        1u64.to_le_bytes().to_vec(),
                    ]
                    .concat(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    });
    let invokes = HashMap::from([(RAYDIUM_CPMM_PROGRAM, vec![(0, -1), (1, -1)])]);
    for (payer, expected_mint) in [
        (Pubkey::default(), Pubkey::default()),
        (keys[0], keys[10]),
        (keys[13], keys[23]),
    ] {
        let mut event = DexEvent::RaydiumCpmmSwap(RaydiumCpmmSwapEvent {
            pool_id: keys[3],
            payer,
            base_input: true,
            ..Default::default()
        });
        fill_accounts_with_owned_keys(&mut event, &TransactionStatusMeta::default(), &tx, &invokes);
        let DexEvent::RaydiumCpmmSwap(event) = event else {
            unreachable!()
        };
        assert_eq!(event.input_token_mint, expected_mint);
        assert_eq!(event.pool_id, keys[3]);
    }
}
