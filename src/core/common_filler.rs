use crate::core::events::*;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use yellowstone_grpc_proto::prelude::{Transaction, TransactionStatusMeta};

/// `get_fees` / `get_fees_with_quote_mint`: discriminator followed by is_pump_pool.
/// Other instructions in the fee program do not carry this parameter.
#[inline]
pub(crate) fn pumpswap_is_pump_pool_from_fee_instruction(data: &[u8]) -> Option<bool> {
    const GET_FEES: [u8; 8] = [231, 37, 126, 85, 207, 91, 63, 52];
    const GET_FEES_WITH_QUOTE: [u8; 8] = [154, 237, 138, 92, 162, 2, 162, 187];
    let discriminator: [u8; 8] = data.get(..8)?.try_into().ok()?;
    if discriminator != GET_FEES && discriminator != GET_FEES_WITH_QUOTE {
        return None;
    }
    match data.get(8) {
        Some(0) => Some(false),
        Some(1) => Some(true),
        _ => None,
    }
}

#[inline]
fn set_pumpswap_is_pump_pool_from_fees_ix(
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    program_invokes: &HashMap<Pubkey, Vec<(i32, i32)>>,
    pool: Pubkey,
    user: Pubkey,
    buy: bool,
    is_pump_pool: &mut bool,
) {
    // Standalone fee queries carry no pool identity. Only a direct fee CPI
    // belonging to the uniquely matched swap can enrich this event.
    let Some(swaps) = program_invokes.get(&crate::grpc::program_ids::PUMPSWAP_PROGRAM) else {
        return;
    };
    let Some(swap) = crate::core::account_dispatcher::find_pumpswap_trade_invoke(
        swaps,
        meta,
        transaction,
        pool,
        user,
        buy,
    ) else {
        return;
    };
    let Some(group) = meta
        .inner_instructions
        .iter()
        .find(|g| g.index == swap.0 as u32)
    else {
        return;
    };
    let (start, depth) = if swap.1 < 0 {
        (0, 1)
    } else {
        let Some(depth) = group
            .instructions
            .get(swap.1 as usize)
            .and_then(|ix| ix.stack_height)
        else {
            return;
        };
        (swap.1 as usize + 1, depth)
    };
    let Some(fees) = program_invokes.get(&crate::grpc::program_ids::PUMPSWAP_FEES_PROGRAM) else {
        return;
    };
    let Some(child_depth) = depth.checked_add(1) else {
        return;
    };
    let mut flag = None;
    for (inner_index, ix) in group.instructions.iter().enumerate().skip(start) {
        let Some(height) = ix.stack_height else {
            // Without invocation depth, parentage is unknown.
            return;
        };
        if height <= depth {
            break;
        }
        if height == child_depth {
            if let Some(value) = pumpswap_is_pump_pool_from_fee_instruction(&ix.data) {
                if !fees.contains(&(swap.0, inner_index as i32)) {
                    continue;
                }
                if flag.is_some_and(|previous| previous != value) {
                    return;
                }
                flag = Some(value);
            }
        }
    }
    if let Some(flag) = flag {
        *is_pump_pool = flag;
    }
}

#[inline]
pub fn fill_data(
    event: &mut DexEvent,
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    program_invokes: &HashMap<Pubkey, Vec<(i32, i32)>>,
) {
    match event {
        DexEvent::PumpSwapBuy(ref mut e) => {
            set_pumpswap_is_pump_pool_from_fees_ix(
                meta,
                transaction,
                program_invokes,
                e.pool,
                e.user,
                true,
                &mut e.is_pump_pool,
            );
        }
        DexEvent::PumpSwapSell(ref mut e) => {
            set_pumpswap_is_pump_pool_from_fees_ix(
                meta,
                transaction,
                program_invokes,
                e.pool,
                e.user,
                false,
                &mut e.is_pump_pool,
            );
        }
        _ => {}
    }
}

pub fn get_instruction_data<'a>(
    meta: &'a TransactionStatusMeta,
    transaction: &'a Option<Transaction>,
    index: &(i32, i32), // (outer_index, inner_index)
) -> Option<&'a [u8]> {
    let data = if index.1 >= 0 {
        meta.inner_instructions
            .iter()
            .find(|i| i.index == index.0 as u32)?
            .instructions
            .get(index.1 as usize)?
            .data
            .as_slice()
    } else {
        transaction
            .as_ref()?
            .message
            .as_ref()?
            .instructions
            .get(index.0 as usize)?
            .data
            .as_slice()
    };
    Some(data)
}

#[cfg(test)]
mod fee_instruction_layout_tests {
    use super::pumpswap_is_pump_pool_from_fee_instruction;

    #[test]
    fn pump_pool_flag_is_independent_of_market_cap_and_other_fee_instructions() {
        for disc in [
            [231, 37, 126, 85, 207, 91, 63, 52],
            [154, 237, 138, 92, 162, 2, 162, 187],
        ] {
            for (flag, market_cap) in [(0u8, 1u128), (1, 0)] {
                let mut data = disc.to_vec();
                data.push(flag);
                data.extend_from_slice(&market_cap.to_le_bytes());
                assert_eq!(
                    pumpswap_is_pump_pool_from_fee_instruction(&data),
                    Some(flag != 0)
                );
            }
            assert_eq!(pumpswap_is_pump_pool_from_fee_instruction(&disc), None);
        }
        let mut unknown = vec![0; 9];
        unknown[8] = 1;
        assert_eq!(pumpswap_is_pump_pool_from_fee_instruction(&unknown), None);
    }
}

#[cfg(test)]
mod review_fee_scope_regressions {
    use super::*;
    use yellowstone_grpc_proto::prelude::{
        CompiledInstruction, InnerInstruction, InnerInstructions, Message,
    };

    #[test]
    fn fees_only_enrich_their_own_swap_and_unknown_parentage_preserves_logs() {
        let pool0 = Pubkey::new_unique();
        let pool1 = Pubkey::new_unique();
        let user = Pubkey::new_unique();
        let swap = |pool_index| CompiledInstruction {
            accounts: std::iter::once(pool_index)
                .chain(std::iter::repeat(2).take(22))
                .collect(),
            data: crate::instr::pump_amm::discriminators::BUY.to_vec(),
            ..Default::default()
        };
        let fee = |flag| {
            let mut data = vec![231, 37, 126, 85, 207, 91, 63, 52];
            data.push(flag);
            data
        };
        let tx = Some(Transaction {
            message: Some(Message {
                account_keys: vec![
                    pool0.to_bytes().to_vec(),
                    pool1.to_bytes().to_vec(),
                    user.to_bytes().to_vec(),
                ],
                instructions: vec![
                    swap(0),
                    swap(1),
                    CompiledInstruction {
                        data: fee(1),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        });
        let mut meta = TransactionStatusMeta {
            inner_instructions: vec![
                InnerInstructions {
                    index: 0,
                    instructions: vec![InnerInstruction {
                        data: fee(1),
                        stack_height: Some(2),
                        ..Default::default()
                    }],
                },
                InnerInstructions {
                    index: 1,
                    instructions: vec![InnerInstruction {
                        data: fee(0),
                        stack_height: Some(2),
                        ..Default::default()
                    }],
                },
            ],
            ..Default::default()
        };
        let invokes = HashMap::from([
            (
                crate::grpc::program_ids::PUMPSWAP_PROGRAM,
                vec![(0, -1), (1, -1)],
            ),
            (
                crate::grpc::program_ids::PUMPSWAP_FEES_PROGRAM,
                vec![(0, 0), (1, 0), (2, -1)],
            ),
        ]);
        let mut flag = false;
        set_pumpswap_is_pump_pool_from_fees_ix(&meta, &tx, &invokes, pool0, user, true, &mut flag);
        assert!(flag);
        set_pumpswap_is_pump_pool_from_fees_ix(&meta, &tx, &invokes, pool1, user, true, &mut flag);
        assert!(
            !flag,
            "last standalone query must not override the other pool"
        );
        meta.inner_instructions.clear();
        set_pumpswap_is_pump_pool_from_fees_ix(&meta, &tx, &invokes, pool0, user, true, &mut flag);
        assert!(!flag, "standalone query cannot be attributed to this pool");
        meta.inner_instructions = vec![InnerInstructions {
            index: 1,
            instructions: vec![InnerInstruction {
                data: fee(0),
                stack_height: None,
                ..Default::default()
            }],
        }];
        flag = true;
        set_pumpswap_is_pump_pool_from_fees_ix(&meta, &tx, &invokes, pool1, user, true, &mut flag);
        assert!(
            flag,
            "preserve a log-derived flag without parentage evidence"
        );
    }
}
