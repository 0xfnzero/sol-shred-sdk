//! Public log-to-account enrichment must select a unique compatible instruction.
use sol_shred_sdk::pubkey::Pubkey;
use sol_shred_sdk::{
    core::{account_dispatcher::fill_accounts_with_owned_keys, events::*},
    instr::{
        meteora_amm::discriminators as pools,
        orca_whirlpool::discriminators as orca,
        program_ids::{METEORA_POOLS_PROGRAM_ID, ORCA_WHIRLPOOL_PROGRAM_ID},
    },
};
use std::collections::HashMap;
use yellowstone_grpc_proto::prelude::{
    CompiledInstruction, Message, Transaction, TransactionStatusMeta,
};

fn wire(disc: [u8; 8], body_len: usize) -> Vec<u8> {
    let mut data = [disc.to_vec(), vec![0; body_len]].concat();
    if body_len == 34 && (disc == orca::SWAP || disc == orca::SWAP_V2) {
        data[41] = 1;
    }
    data
}

fn enrich(event: &mut DexEvent, program: Pubkey, layouts: &[(Vec<Pubkey>, Vec<u8>)]) {
    let mut keys = vec![program.to_bytes().to_vec()];
    let instructions = layouts
        .iter()
        .map(|(accounts, data)| {
            let offset = keys.len();
            keys.extend(accounts.iter().map(|key| key.to_bytes().to_vec()));
            CompiledInstruction {
                program_id_index: 0,
                accounts: (offset..keys.len()).map(|i| i as u8).collect(),
                data: data.clone(),
            }
        })
        .collect();
    let tx = Some(Transaction {
        message: Some(Message {
            account_keys: keys,
            instructions,
            ..Default::default()
        }),
        ..Default::default()
    });
    let invokes = HashMap::from([(
        program,
        (0..layouts.len()).map(|i| (i as i32, -1)).collect(),
    )]);
    fill_accounts_with_owned_keys(event, &TransactionStatusMeta::default(), &tx, &invokes);
}

fn keys(count: usize) -> Vec<Pubkey> {
    (0..count).map(|_| Pubkey::new_unique()).collect()
}
fn memo() -> Pubkey {
    "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr"
        .parse()
        .unwrap()
}
fn orca_log(pool: Pubkey) -> DexEvent {
    let mut body = vec![0; 113];
    body[..32].copy_from_slice(pool.as_ref());
    body[32] = 1;
    sol_shred_sdk::logs::orca_whirlpool::parse_traded_from_data(&body, Default::default()).unwrap()
}
fn pools_log() -> DexEvent {
    let mut body = vec![0; 40];
    body[..8].copy_from_slice(&100u64.to_le_bytes());
    sol_shred_sdk::logs::meteora_amm::parse_swap_from_data(&body, Default::default()).unwrap()
}

#[test]
fn orca_swap_context_matches_pool_and_version_instead_of_last_layout() {
    for v2 in [false, true] {
        let pool_index = if v2 { 4 } else { 2 };
        let mut a = keys(if v2 { 15 } else { 11 });
        let mut b = keys(a.len());
        if v2 {
            a[2] = memo();
            b[2] = memo();
        }
        let data = wire(if v2 { orca::SWAP_V2 } else { orca::SWAP }, 34);
        let mut event = orca_log(a[pool_index]);
        enrich(
            &mut event,
            ORCA_WHIRLPOOL_PROGRAM_ID,
            &[(a.clone(), data.clone()), (b, data)],
        );
        let DexEvent::OrcaWhirlpoolSwap(e) = event else {
            panic!()
        };
        assert_eq!(e.token_vault_a, a[if v2 { 8 } else { 4 }]);
        assert_eq!(e.token_owner_account_a, a[if v2 { 7 } else { 3 }]);
        if v2 {
            assert_eq!(e.token_mint_a, a[5]);
        }
    }
}

#[test]
fn orca_ambiguous_or_invalid_swap_context_stays_unknown() {
    let mut a = keys(15);
    a[2] = memo();
    let data = wire(orca::SWAP_V2, 34);
    for layouts in [
        vec![(a.clone(), data.clone()), (a.clone(), data.clone())],
        vec![(a[..14].to_vec(), data.clone())],
        vec![(a.clone(), wire(orca::INCREASE_LIQUIDITY_V2, 32))],
        vec![(a.clone(), wire(orca::SWAP_V2, 33))],
    ] {
        let mut event = orca_log(a[4]);
        enrich(&mut event, ORCA_WHIRLPOOL_PROGRAM_ID, &layouts);
        let DexEvent::OrcaWhirlpoolSwap(e) = event else {
            panic!()
        };
        assert_eq!(e.token_mint_a, Pubkey::default());
    }
    let mut wrong_direction = data.clone();
    wrong_direction[41] = 0;
    let mut event = orca_log(a[4]);
    enrich(
        &mut event,
        ORCA_WHIRLPOOL_PROGRAM_ID,
        &[(a.clone(), wrong_direction)],
    );
    let DexEvent::OrcaWhirlpoolSwap(e) = event else {
        panic!()
    };
    assert_eq!(e.token_mint_a, Pubkey::default());
    let mut event = orca_log(a[4]);
    if let DexEvent::OrcaWhirlpoolSwap(e) = &mut event {
        e.token_authority = Pubkey::new_unique();
    }
    enrich(&mut event, ORCA_WHIRLPOOL_PROGRAM_ID, &[(a, data)]);
    let DexEvent::OrcaWhirlpoolSwap(e) = event else {
        panic!()
    };
    assert_eq!(e.token_mint_a, Pubkey::default());
}

#[test]
fn meteora_swap_ignores_larger_liquidity_and_preserves_executed_amount() {
    let swap = keys(15);
    let liquidity = keys(16);
    let mut event = pools_log();
    enrich(
        &mut event,
        METEORA_POOLS_PROGRAM_ID,
        &[
            (swap.clone(), wire(pools::SWAP, 16)),
            (liquidity, wire(pools::ADD_LIQUIDITY, 24)),
        ],
    );
    let DexEvent::MeteoraPoolsSwap(e) = event else {
        panic!()
    };
    assert_eq!(e.pool, swap[0]);
    assert_eq!(e.user_source_token, swap[1]);
    assert_eq!(e.in_amount, 100);
}

#[test]
fn meteora_repeated_swaps_fail_closed_unless_known_context_is_unique() {
    let a = keys(15);
    let b = keys(15);
    let layouts = vec![
        (a.clone(), wire(pools::SWAP, 16)),
        (b.clone(), wire(pools::SWAP, 16)),
    ];
    let mut event = pools_log();
    enrich(&mut event, METEORA_POOLS_PROGRAM_ID, &layouts);
    let DexEvent::MeteoraPoolsSwap(e) = event else {
        panic!()
    };
    assert_eq!(e.pool, Pubkey::default());
    let mut event = pools_log();
    if let DexEvent::MeteoraPoolsSwap(e) = &mut event {
        e.user = b[12];
    }
    enrich(&mut event, METEORA_POOLS_PROGRAM_ID, &layouts);
    let DexEvent::MeteoraPoolsSwap(e) = event else {
        panic!()
    };
    assert_eq!(e.pool, b[0]);
    let mut event = pools_log();
    enrich(
        &mut event,
        METEORA_POOLS_PROGRAM_ID,
        &[
            (a.clone(), wire(pools::SWAP, 16)),
            (a.clone(), wire(pools::SWAP, 16)),
        ],
    );
    let DexEvent::MeteoraPoolsSwap(e) = event else {
        panic!()
    };
    assert_eq!(e.pool, Pubkey::default());
    for (accounts, data) in [
        (a[..14].to_vec(), wire(pools::SWAP, 16)),
        (a, wire(pools::SWAP, 15)),
    ] {
        let mut event = pools_log();
        enrich(&mut event, METEORA_POOLS_PROGRAM_ID, &[(accounts, data)]);
        let DexEvent::MeteoraPoolsSwap(e) = event else {
            panic!()
        };
        assert_eq!(e.pool, Pubkey::default());
    }
}

#[test]
fn meteora_add_context_discriminates_kind_pool_and_user() {
    for imbalance in [false, true] {
        let a = keys(16);
        let b = keys(16);
        let disc = if imbalance {
            pools::ADD_IMBALANCE_LIQUIDITY
        } else {
            pools::ADD_LIQUIDITY
        };
        let mut event = DexEvent::MeteoraPoolsAddLiquidity(MeteoraPoolsAddLiquidityEvent {
            pool: a[0],
            user: a[13],
            ..Default::default()
        });
        enrich(
            &mut event,
            METEORA_POOLS_PROGRAM_ID,
            &[(a.clone(), wire(disc, 24)), (b, wire(disc, 24))],
        );
        let DexEvent::MeteoraPoolsAddLiquidity(e) = event else {
            panic!()
        };
        assert_eq!(e.lp_mint, a[1]);
        let mut event =
            DexEvent::MeteoraPoolsAddLiquidity(MeteoraPoolsAddLiquidityEvent::default());
        enrich(
            &mut event,
            METEORA_POOLS_PROGRAM_ID,
            &[(a.clone(), wire(disc, 24)), (a, wire(disc, 24))],
        );
        let DexEvent::MeteoraPoolsAddLiquidity(e) = event else {
            panic!()
        };
        assert_eq!(e.lp_mint, Pubkey::default());
    }
}

#[test]
fn orca_liquidity_context_requires_direction_version_and_unique_identity() {
    for v2 in [false, true] {
        for increase in [false, true] {
            let mut a = keys(if v2 { 15 } else { 11 });
            if v2 {
                a[3] = memo();
            }
            let disc = match (v2, increase) {
                (false, true) => orca::INCREASE_LIQUIDITY,
                (true, true) => orca::INCREASE_LIQUIDITY_V2,
                (false, false) => orca::DECREASE_LIQUIDITY,
                (true, false) => orca::DECREASE_LIQUIDITY_V2,
            };
            let make = || {
                if increase {
                    DexEvent::OrcaWhirlpoolLiquidityIncreased(
                        OrcaWhirlpoolLiquidityIncreasedEvent {
                            metadata: Default::default(),
                            whirlpool: a[0],
                            position: Pubkey::default(),
                            tick_lower_index: 0,
                            tick_upper_index: 0,
                            liquidity: 1,
                            token_a_amount: 2,
                            token_b_amount: 3,
                            token_a_transfer_fee: 0,
                            token_b_transfer_fee: 0,
                        },
                    )
                } else {
                    DexEvent::OrcaWhirlpoolLiquidityDecreased(
                        OrcaWhirlpoolLiquidityDecreasedEvent {
                            metadata: Default::default(),
                            whirlpool: a[0],
                            position: Pubkey::default(),
                            tick_lower_index: 0,
                            tick_upper_index: 0,
                            liquidity: 1,
                            token_a_amount: 2,
                            token_b_amount: 3,
                            token_a_transfer_fee: 0,
                            token_b_transfer_fee: 0,
                        },
                    )
                }
            };
            let position = |e: DexEvent| match e {
                DexEvent::OrcaWhirlpoolLiquidityIncreased(e) => e.position,
                DexEvent::OrcaWhirlpoolLiquidityDecreased(e) => e.position,
                _ => panic!(),
            };
            let mut event = make();
            enrich(
                &mut event,
                ORCA_WHIRLPOOL_PROGRAM_ID,
                &[
                    (a.clone(), wire(disc, 32)),
                    (keys(a.len()), wire(disc, 32)),
                    (a.clone(), wire(orca::SWAP_V2, 34)),
                ],
            );
            assert_eq!(position(event), a[if v2 { 5 } else { 3 }]);
            for layouts in [
                vec![(a.clone(), wire(disc, 32)), (a.clone(), wire(disc, 32))],
                vec![(a[..a.len() - 1].to_vec(), wire(disc, 32))],
                vec![(a.clone(), wire(disc, 31))],
                vec![(
                    a.clone(),
                    wire(
                        if increase {
                            orca::DECREASE_LIQUIDITY
                        } else {
                            orca::INCREASE_LIQUIDITY
                        },
                        32,
                    ),
                )],
            ] {
                let mut event = make();
                enrich(&mut event, ORCA_WHIRLPOOL_PROGRAM_ID, &layouts);
                assert_eq!(position(event), Pubkey::default());
            }
        }
    }
}

#[test]
fn pumpswap_public_wrappers_support_compact_and_legacy_resolved_layouts() {
    use sol_shred_sdk::core::account_fillers::pumpswap::*;
    use sol_shred_sdk::instr::program_ids::PUMPSWAP_PROGRAM_ID;
    for compact in [false, true] {
        let mut accounts = keys(if compact { 17 } else { 23 });
        // The marker alone must not misclassify a longer legacy layout.
        accounts[16] = PUMPSWAP_PROGRAM_ID;
        let get = |i: usize| accounts.get(i).copied().unwrap_or_default();
        let mut buy = PumpSwapBuyEvent::default();
        let mut counted = buy.clone();
        fill_buy_accounts(&mut buy, &get);
        fill_buy_accounts_with_count(&mut counted, &get, accounts.len());
        assert_eq!(
            buy.base_token_program,
            accounts[if compact { 9 } else { 11 }]
        );
        assert_eq!(buy.base_token_program, counted.base_token_program);
        assert_eq!(
            buy.protocol_fee_recipient,
            if compact {
                Pubkey::default()
            } else {
                accounts[9]
            }
        );
        let mut sell = PumpSwapSellEvent::default();
        let mut counted = sell.clone();
        fill_sell_accounts(&mut sell, &get);
        fill_sell_accounts_with_count(&mut counted, &get, accounts.len());
        assert_eq!(
            sell.base_token_program,
            accounts[if compact { 9 } else { 11 }]
        );
        assert_eq!(sell.base_token_program, counted.base_token_program);
    }
}

#[test]
fn pumpswap_partial_account_count_does_not_guess_compact_layout() {
    use sol_shred_sdk::core::account_fillers::pumpswap::*;
    let mut accounts = keys(23);
    accounts[16] = sol_shred_sdk::instr::program_ids::PUMPSWAP_PROGRAM_ID;
    accounts[17] = Pubkey::default(); // unresolved legacy ALT slot, not a shorter instruction
    let get = |i: usize| accounts.get(i).copied().unwrap_or_default();
    let mut buy = PumpSwapBuyEvent::default();
    fill_buy_accounts_with_count(&mut buy, &get, 23);
    assert_eq!(buy.base_token_program, accounts[11]);
    assert_eq!(buy.coin_creator_vault_ata, Pubkey::default());
}
