//! Official pinned CLMM/DAMM management layouts; only unique identity context is filled.
use sol_shred_sdk::{
    core::{account_dispatcher::fill_accounts_with_owned_keys, events::*},
    instr::{
        program_ids::{METEORA_DAMM_V2_PROGRAM_ID, RAYDIUM_CLMM_PROGRAM_ID},
        raydium_clmm::{self, discriminators as c},
    },
    pubkey::Pubkey,
};
use std::collections::HashMap;
use yellowstone_grpc_proto::prelude::{
    CompiledInstruction, InnerInstruction, InnerInstructions, Message, Transaction,
    TransactionStatusMeta,
};
fn keys(n: usize) -> Vec<Pubkey> {
    (0..n).map(|_| Pubkey::new_unique()).collect()
}
fn wire(d: [u8; 8], body: usize) -> Vec<u8> {
    [d.to_vec(), vec![0; body]].concat()
}
fn enrich(
    event: &mut DexEvent,
    program: Pubkey,
    instructions: &[(Vec<Pubkey>, Vec<u8>)],
    nested: bool,
    reverse: bool,
) {
    let mut account_keys = vec![program.to_bytes().to_vec()];
    let mut compiled = vec![];
    for (a, data) in instructions {
        let start = account_keys.len();
        account_keys.extend(a.iter().map(|k| k.to_bytes().to_vec()));
        compiled.push(CompiledInstruction {
            program_id_index: 0,
            accounts: (start..account_keys.len()).map(|v| v as u8).collect(),
            data: data.clone(),
        });
    }
    let mut positions: Vec<_> = (0..compiled.len())
        .map(|i| {
            if nested {
                (0, i as i32)
            } else {
                (i as i32, -1)
            }
        })
        .collect();
    if reverse {
        positions.reverse();
    }
    let mut meta = TransactionStatusMeta::default();
    if nested {
        meta.inner_instructions = vec![InnerInstructions {
            index: 0,
            instructions: compiled
                .iter()
                .map(|ix| InnerInstruction {
                    program_id_index: ix.program_id_index,
                    accounts: ix.accounts.clone(),
                    data: ix.data.clone(),
                    stack_height: Some(2),
                })
                .collect(),
        }];
        compiled = vec![CompiledInstruction::default()];
    }
    let tx = Some(Transaction {
        message: Some(Message {
            account_keys,
            instructions: compiled,
            ..Default::default()
        }),
        ..Default::default()
    });
    fill_accounts_with_owned_keys(event, &meta, &tx, &HashMap::from([(program, positions)]));
}
fn create_event(a: &[Pubkey]) -> DexEvent {
    let mut b = vec![0; 182];
    for (o, k) in [(0, a[3]), (32, a[4]), (66, a[2]), (118, a[5]), (150, a[6])] {
        b[o..o + 32].copy_from_slice(k.as_ref());
    }
    sol_shred_sdk::logs::raydium_clmm::parse_create_pool_from_data(&b, Default::default()).unwrap()
}
#[test]
fn create_pool_ignores_longer_swap_and_conflicting_known_identities_in_both_invoke_orders() {
    let a = keys(13);
    let mut incompatible = a.clone();
    incompatible[0] = Pubkey::new_unique();
    incompatible[3] = Pubkey::new_unique();
    let swap = keys(25);
    for nested in [false, true] {
        for reverse in [false, true] {
            let mut e = create_event(&a);
            enrich(
                &mut e,
                RAYDIUM_CLMM_PROGRAM_ID,
                &[
                    (a.clone(), wire(c::CREATE_POOL, 24)),
                    (swap.clone(), wire(c::SWAP_V2, 33)),
                    (incompatible.clone(), wire(c::CREATE_POOL, 24)),
                ],
                nested,
                reverse,
            );
            let DexEvent::RaydiumClmmCreatePool(e) = e else {
                panic!()
            };
            assert_eq!(e.creator, a[0]);
            assert_eq!(e.pool, a[2]);
        }
    }
}
#[test]
fn ambiguous_or_malformed_create_pool_leaves_unknown_creator() {
    let a = keys(13);
    for instructions in [
        vec![
            (a.clone(), wire(c::CREATE_POOL, 24)),
            (a.clone(), wire(c::CREATE_POOL, 24)),
        ],
        vec![(a.clone(), wire(c::CREATE_POOL, 23))],
        vec![(a[..12].to_vec(), wire(c::CREATE_POOL, 24))],
    ] {
        let mut e = create_event(&a);
        enrich(&mut e, RAYDIUM_CLMM_PROGRAM_ID, &instructions, false, false);
        let DexEvent::RaydiumClmmCreatePool(e) = e else {
            panic!()
        };
        assert_eq!(e.creator, Pubkey::default());
    }
}
fn position(nft: Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"position", nft.as_ref()], &RAYDIUM_CLMM_PROGRAM_ID).0
}
#[test]
fn increase_and_decrease_use_personal_position_not_nft_token_account_and_fail_closed() {
    for increase in [false, true] {
        for legacy in [false, true] {
            let nft = Pubkey::new_unique();
            let count = if legacy {
                12
            } else if increase {
                15
            } else {
                16
            };
            let mut a = keys(count);
            a[if increase { 4 } else { 2 }] = position(nft);
            let disc = match (increase, legacy) {
                (true, true) => c::INCREASE_LIQUIDITY,
                (true, false) => c::INCREASE_LIQUIDITY_V2,
                (false, true) => c::DECREASE_LIQUIDITY,
                (false, false) => c::DECREASE_LIQUIDITY_V2,
            };
            let data = wire(disc, if increase && !legacy { 33 } else { 32 });
            let mut b = vec![0; if increase { 80 } else { 120 }];
            b[..32].copy_from_slice(nft.as_ref());
            let parse = || {
                if increase {
                    sol_shred_sdk::logs::raydium_clmm::parse_increase_liquidity_from_data(
                        &b,
                        Default::default(),
                    )
                    .unwrap()
                } else {
                    sol_shred_sdk::logs::raydium_clmm::parse_decrease_liquidity_from_data(
                        &b,
                        Default::default(),
                    )
                    .unwrap()
                }
            };
            for nested in [false, true] {
                let mut e = parse();
                enrich(
                    &mut e,
                    RAYDIUM_CLMM_PROGRAM_ID,
                    &[(a.clone(), data.clone()), (keys(26), wire(c::SWAP_V2, 33))],
                    nested,
                    true,
                );
                match e {
                    DexEvent::RaydiumClmmIncreaseLiquidity(e) => {
                        assert_eq!(e.pool, a[2]);
                        assert_eq!(e.user, a[0]);
                        assert_eq!(e.position_nft_mint, nft);
                    }
                    DexEvent::RaydiumClmmDecreaseLiquidity(e) => {
                        assert_eq!(e.pool, a[3]);
                        assert_eq!(e.user, a[0]);
                        assert_eq!(e.position_nft_mint, nft);
                    }
                    _ => panic!(),
                }
            }
            let mut unrelated = a.clone();
            unrelated[if increase { 4 } else { 2 }] = Pubkey::new_unique();
            for candidates in [
                vec![(unrelated, data.clone())],
                vec![(a.clone(), data.clone()), (a.clone(), data.clone())],
            ] {
                let mut e = parse();
                enrich(&mut e, RAYDIUM_CLMM_PROGRAM_ID, &candidates, false, false);
                match e {
                    DexEvent::RaydiumClmmIncreaseLiquidity(e) => {
                        assert_eq!(e.pool, Pubkey::default())
                    }
                    DexEvent::RaydiumClmmDecreaseLiquidity(e) => {
                        assert_eq!(e.pool, Pubkey::default())
                    }
                    _ => panic!(),
                }
            }
        }
    }
}
#[test]
fn v2_instruction_and_public_fillers_keep_unavailable_nft_mint_unknown() {
    for increase in [true, false] {
        let a = keys(if increase { 15 } else { 16 });
        let data = wire(
            if increase {
                c::INCREASE_LIQUIDITY_V2
            } else {
                c::DECREASE_LIQUIDITY_V2
            },
            if increase { 33 } else { 32 },
        );
        let mut e =
            raydium_clmm::parse_instruction(&data, &a, Default::default(), 1, 0, None).unwrap();
        enrich(
            &mut e,
            RAYDIUM_CLMM_PROGRAM_ID,
            &[(a.clone(), data)],
            false,
            false,
        );
        match e {
            DexEvent::RaydiumClmmIncreaseLiquidity(mut e) => {
                assert_eq!(e.position_nft_mint, Pubkey::default());
                sol_shred_sdk::core::account_fillers::raydium::fill_clmm_increase_liquidity_accounts(&mut e,&|i|a.get(i).copied().unwrap_or_default());
                assert_eq!(e.position_nft_mint, Pubkey::default());
            }
            DexEvent::RaydiumClmmDecreaseLiquidity(mut e) => {
                assert_eq!(e.position_nft_mint, Pubkey::default());
                sol_shred_sdk::core::account_fillers::raydium::fill_clmm_decrease_liquidity_accounts(&mut e,&|i|a.get(i).copied().unwrap_or_default());
                assert_eq!(e.position_nft_mint, Pubkey::default());
            }
            _ => panic!(),
        }
    }
}
#[test]
fn open_position_layouts_and_close_position_select_exact_nft_context() {
    for (disc, count, pool_index, body) in [
        (c::OPEN_POSITION, 19, 5, 48),
        (c::OPEN_POSITION_V2, 22, 5, 50),
        (c::OPEN_POSITION_WITH_TOKEN_22_NFT, 20, 4, 50),
    ] {
        let a = keys(count);
        let data = wire(disc, body);
        let DexEvent::RaydiumClmmOpenPosition(mut e) =
            raydium_clmm::parse_instruction(&data, &a, Default::default(), 1, 0, None).unwrap()
        else {
            panic!()
        };
        e.pool = Pubkey::default();
        e.user = Pubkey::default();
        let mut event = DexEvent::RaydiumClmmOpenPosition(e);
        enrich(
            &mut event,
            RAYDIUM_CLMM_PROGRAM_ID,
            &[(a.clone(), data), (keys(25), wire(c::SWAP_V2, 33))],
            true,
            true,
        );
        let DexEvent::RaydiumClmmOpenPosition(e) = event else {
            panic!()
        };
        assert_eq!(
            (e.pool, e.user, e.position_nft_mint),
            (a[pool_index], a[1], a[2])
        );
    }
    let mut a = keys(6);
    a[3] = position(a[1]);
    let DexEvent::RaydiumClmmClosePosition(mut e) =
        raydium_clmm::parse_instruction(&c::CLOSE_POSITION, &a, Default::default(), 1, 0, None)
            .unwrap()
    else {
        panic!()
    };
    e.user = Pubkey::default();
    let mut event = DexEvent::RaydiumClmmClosePosition(e);
    enrich(
        &mut event,
        RAYDIUM_CLMM_PROGRAM_ID,
        &[
            (a.clone(), c::CLOSE_POSITION.to_vec()),
            (keys(25), wire(c::SWAP_V2, 33)),
        ],
        false,
        true,
    );
    let DexEvent::RaydiumClmmClosePosition(e) = event else {
        panic!()
    };
    assert_eq!(e.user, a[0]);
    assert_eq!(e.position_nft_mint, a[1]);
    // A non-frozen position may include unused remaining accounts. They must
    // not be interpreted as a mandatory pool address or reject valid context.
    let known_pool = Pubkey::new_unique();
    a.push(Pubkey::new_unique());
    let mut e = e;
    e.pool = known_pool;
    e.user = Pubkey::default();
    let mut event = DexEvent::RaydiumClmmClosePosition(e);
    enrich(
        &mut event,
        RAYDIUM_CLMM_PROGRAM_ID,
        &[(a.clone(), c::CLOSE_POSITION.to_vec())],
        true,
        false,
    );
    let DexEvent::RaydiumClmmClosePosition(e) = event else {
        panic!()
    };
    assert_eq!(e.user, a[0]);
    assert_eq!(e.pool, known_pool);
}
fn damm_event(a: &[Pubkey], dynamic: bool) -> DexEvent {
    let shift = usize::from(dynamic);
    let mut b = vec![0; 332];
    for (o, k) in [
        (0, a[6 + shift]),
        (32, a[8 + shift]),
        (64, a[9 + shift]),
        (96, a[0]),
        (128, a[3]),
    ] {
        b[o..o + 32].copy_from_slice(k.as_ref());
    }
    sol_shred_sdk::logs::meteora_damm::parse_initialize_pool_from_data(&b, Default::default())
        .unwrap()
}
#[test]
fn damm_static_and_dynamic_initialization_select_correct_complete_layout() {
    for dynamic in [false, true] {
        for nested in [false, true] {
            let a = keys(if dynamic { 21 } else { 20 });
            let disc = if dynamic {
                [149, 82, 72, 197, 253, 252, 68, 15]
            } else {
                [95, 180, 10, 172, 84, 174, 232, 40]
            };
            let data = wire(disc, if dynamic { 99 } else { 33 });
            let mut e = damm_event(&a, dynamic);
            enrich(
                &mut e,
                METEORA_DAMM_V2_PROGRAM_ID,
                &[(a.clone(), data.clone()), (keys(26), wire([0; 8], 99))],
                nested,
                true,
            );
            let DexEvent::MeteoraDammV2InitializePool(e) = e else {
                panic!()
            };
            assert_eq!(e.position, a[7 + usize::from(dynamic)]);
            assert_eq!(e.position_nft_mint, a[1]);
            assert_eq!(e.pool, a[6 + usize::from(dynamic)]);
            for candidates in [
                vec![(a.clone(), data[..data.len() - 1].to_vec())],
                vec![(a.clone(), data.clone()), (a.clone(), data.clone())],
            ] {
                let mut e = damm_event(&a, dynamic);
                enrich(
                    &mut e,
                    METEORA_DAMM_V2_PROGRAM_ID,
                    &candidates,
                    nested,
                    false,
                );
                let DexEvent::MeteoraDammV2InitializePool(e) = e else {
                    panic!()
                };
                assert_eq!(e.position, Pubkey::default());
            }
        }
    }
}
#[test]
fn damm_dynamic_optional_fee_activation_and_invalid_borsh_tags_are_checked() {
    let disc = [149, 82, 72, 197, 253, 252, 68, 15];
    let a = keys(21);
    for fee in [false, true] {
        for activation in [false, true] {
            let shift = if fee { 32 } else { 0 };
            let mut data = wire(disc, 99 + shift + if activation { 8 } else { 0 });
            data[38] = u8::from(fee);
            data[71 + shift] = 1;
            data[106 + shift] = u8::from(activation);
            if activation {
                data[107 + shift..].copy_from_slice(&u64::MAX.to_le_bytes());
            }
            let mut e = damm_event(&a, true);
            enrich(
                &mut e,
                METEORA_DAMM_V2_PROGRAM_ID,
                &[(a.clone(), data.clone())],
                true,
                false,
            );
            let DexEvent::MeteoraDammV2InitializePool(e) = e else {
                panic!()
            };
            assert_eq!(e.position, a[8]);
            for offset in [38, 71 + shift, 106 + shift] {
                let mut bad = data.clone();
                bad[offset] = 2;
                let mut e = damm_event(&a, true);
                enrich(
                    &mut e,
                    METEORA_DAMM_V2_PROGRAM_ID,
                    &[(a.clone(), bad)],
                    false,
                    false,
                );
                let DexEvent::MeteoraDammV2InitializePool(e) = e else {
                    panic!()
                };
                assert_eq!(e.position, Pubkey::default());
            }
        }
    }
}
#[test]
fn clmm_option_boolean_tags_and_extra_payload_do_not_supply_liquidity_context() {
    let nft = Pubkey::new_unique();
    let mut a = keys(15);
    a[4] = position(nft);
    let mut body = vec![0; 80];
    body[..32].copy_from_slice(nft.as_ref());
    for option in [None, Some(false), Some(true)] {
        let mut data = wire(
            c::INCREASE_LIQUIDITY_V2,
            if option.is_some() { 34 } else { 33 },
        );
        if let Some(v) = option {
            data[40] = 1;
            data[41] = u8::from(v);
        }
        let parse = || {
            sol_shred_sdk::logs::raydium_clmm::parse_increase_liquidity_from_data(
                &body,
                Default::default(),
            )
            .unwrap()
        };
        let mut e = parse();
        enrich(
            &mut e,
            RAYDIUM_CLMM_PROGRAM_ID,
            &[(a.clone(), data.clone())],
            true,
            false,
        );
        let DexEvent::RaydiumClmmIncreaseLiquidity(e) = e else {
            panic!()
        };
        assert_eq!(e.pool, a[2]);
        let mut bad_tag = data.clone();
        bad_tag[40] = 2;
        let mut trailing = data.clone();
        trailing.push(0);
        let mut invalid_bool = wire(c::INCREASE_LIQUIDITY_V2, 34);
        invalid_bool[40] = 1;
        invalid_bool[41] = 2;
        for bad in [bad_tag, trailing, invalid_bool] {
            let mut e = parse();
            enrich(
                &mut e,
                RAYDIUM_CLMM_PROGRAM_ID,
                &[(a.clone(), bad)],
                false,
                false,
            );
            let DexEvent::RaydiumClmmIncreaseLiquidity(e) = e else {
                panic!()
            };
            assert_eq!(e.pool, Pubkey::default());
        }
    }
}
