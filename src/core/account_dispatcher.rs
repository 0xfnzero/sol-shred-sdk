//! 账户填充调度器
//!
//! 主调度器，负责路由所有 DEX 事件到对应的协议填充器。
//! 从指令账户数据填充事件中缺失的账户字段。
//!
//! 各协议的具体填充逻辑在 account_fillers/ 子模块中实现。

use crate::core::account_fillers::{self, AccountGetter};
use crate::core::events::*;
use crate::instr::utils::get_instruction_account_getter;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use yellowstone_grpc_proto::prelude::{Transaction, TransactionStatusMeta};

// ============================================================================
// Helper Functions
// ============================================================================

// Stop after the second match: ambiguous log context must remain unknown.
fn only_match<T>(mut matches: impl Iterator<Item = T>) -> Option<T> {
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

/// Account enrichment is provisional context: never choose an arbitrary invocation.
/// The predicate validates the instruction version/layout and every known identity.
fn find_context_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    matches: impl Fn(&[u8], &AccountGetter<'_>, usize) -> bool,
) -> Option<&'a (i32, i32)> {
    let keys = transaction
        .as_ref()?
        .message
        .as_ref()
        .map(|msg| &msg.account_keys);
    only_match(invokes.iter().filter(|invoke| {
        let Some(data) =
            crate::core::common_filler::get_instruction_data(meta, transaction, invoke)
        else {
            return false;
        };
        get_instruction_account_getter(
            meta,
            transaction,
            keys,
            &meta.loaded_writable_addresses,
            &meta.loaded_readonly_addresses,
            invoke,
        )
        .is_some_and(|get| {
            matches(
                data,
                &get,
                instruction_account_count(meta, transaction, invoke),
            )
        })
    }))
}

fn known_accounts_match(get: &AccountGetter<'_>, known: &[(Pubkey, usize)]) -> bool {
    known
        .iter()
        .all(|(key, index)| *key == Pubkey::default() || get(*index) == *key)
}

fn find_orca_swap_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    event: &OrcaWhirlpoolSwapEvent,
) -> Option<&'a (i32, i32)> {
    find_context_invoke(invokes, meta, transaction, |data, get, count| {
        use crate::instr::orca_whirlpool::discriminators::{SWAP, SWAP_V2};
        let (name, pool, authority, owner_a, owner_b, vault_a, vault_b, minimum) =
            match data.get(..8) {
                Some(d) if d == SWAP => ("swap", 2, 1, 3, 5, 4, 6, 11),
                Some(d) if d == SWAP_V2 => ("swap_v2", 4, 3, 7, 9, 8, 10, 15),
                _ => return false,
            };
        // Both versions have two u64s, a u128, and two strict Borsh bools.
        if count < minimum
            || data.len() < 42
            || data[40] > 1
            || data[41] > 1
            || (data[41] != 0) != event.a_to_b
            || (!event.ix_name.is_empty() && event.ix_name != name)
        {
            return false;
        }
        (name != "swap_v2"
            || get(2) == solana_sdk::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr"))
            && known_accounts_match(
                get,
                &[
                    (event.whirlpool, pool),
                    (event.token_authority, authority),
                    (event.token_owner_account_a, owner_a),
                    (event.token_owner_account_b, owner_b),
                    (event.token_vault_a, vault_a),
                    (event.token_vault_b, vault_b),
                    (event.token_program_a, 0),
                    (event.token_program_b, if minimum == 15 { 1 } else { 0 }),
                    (event.tick_array_0, if minimum == 15 { 11 } else { 7 }),
                    (event.tick_array_1, if minimum == 15 { 12 } else { 8 }),
                    (event.tick_array_2, if minimum == 15 { 13 } else { 9 }),
                    (event.oracle, minimum - 1),
                ],
            )
            && (name != "swap_v2"
                || known_accounts_match(get, &[(event.token_mint_a, 5), (event.token_mint_b, 6)]))
    })
}

fn find_orca_liquidity_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    pool: Pubkey,
    position: Pubkey,
    increase: bool,
) -> Option<&'a (i32, i32)> {
    find_context_invoke(invokes, meta, transaction, |data, get, count| {
        use crate::instr::orca_whirlpool::discriminators::*;
        let (position_index, minimum) = match data.get(..8) {
            Some(d)
                if (increase && d == INCREASE_LIQUIDITY)
                    || (!increase && d == DECREASE_LIQUIDITY) =>
            {
                (3, 11)
            }
            Some(d)
                if (increase && d == INCREASE_LIQUIDITY_V2)
                    || (!increase && d == DECREASE_LIQUIDITY_V2) =>
            {
                (5, 15)
            }
            _ => return false,
        };
        count >= minimum
            && data.len() >= 40
            && (position_index != 5
                || get(3) == solana_sdk::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr"))
            && known_accounts_match(get, &[(pool, 0), (position, position_index)])
    })
}

fn find_pools_swap_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    event: &MeteoraPoolsSwapEvent,
) -> Option<&'a (i32, i32)> {
    find_context_invoke(invokes, meta, transaction, |data, get, count| {
        data.get(..8) == Some(crate::instr::meteora_amm::discriminators::SWAP.as_slice())
            && data.len() >= 24
            && count >= 15
            && (event.ix_name.is_empty() || event.ix_name == "swap")
            && known_accounts_match(
                get,
                &[
                    (event.pool, 0),
                    (event.user, 12),
                    (event.user_source_token, 1),
                    (event.user_destination_token, 2),
                    (event.a_vault, 3),
                    (event.b_vault, 4),
                    (event.a_token_vault, 5),
                    (event.b_token_vault, 6),
                    (event.a_vault_lp_mint, 7),
                    (event.b_vault_lp_mint, 8),
                    (event.a_vault_lp, 9),
                    (event.b_vault_lp, 10),
                    (event.protocol_token_fee, 11),
                    (event.vault_program, 13),
                    (event.token_program, 14),
                ],
            )
    })
}

fn find_pools_add_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    event: &MeteoraPoolsAddLiquidityEvent,
) -> Option<(&'a (i32, i32), &'static str)> {
    let invoke = find_context_invoke(invokes, meta, transaction, |data, get, count| {
        use crate::instr::meteora_amm::discriminators::*;
        let name = match data.get(..8) {
            Some(d) if d == ADD_LIQUIDITY => "add_balance_liquidity",
            Some(d) if d == ADD_IMBALANCE_LIQUIDITY => "add_imbalance_liquidity",
            _ => return false,
        };
        data.len() >= 32
            && count >= 16
            && (event.ix_name.is_empty() || event.ix_name == name)
            && known_accounts_match(
                get,
                &[
                    (event.pool, 0),
                    (event.lp_mint, 1),
                    (event.user_pool_lp, 2),
                    (event.user, 13),
                    (event.user_a_token, 11),
                    (event.user_b_token, 12),
                    (event.a_vault, 5),
                    (event.b_vault, 6),
                    (event.a_vault_lp, 3),
                    (event.b_vault_lp, 4),
                    (event.a_vault_lp_mint, 7),
                    (event.b_vault_lp_mint, 8),
                    (event.a_token_vault, 9),
                    (event.b_token_vault, 10),
                    (event.vault_program, 14),
                    (event.token_program, 15),
                ],
            )
    })?;
    let data = crate::core::common_filler::get_instruction_data(meta, transaction, invoke)?;
    let name = if data[..8] == crate::instr::meteora_amm::discriminators::ADD_LIQUIDITY {
        "add_balance_liquidity"
    } else {
        "add_imbalance_liquidity"
    };
    Some((invoke, name))
}

fn find_cpmm_swap_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    event: &RaydiumCpmmSwapEvent,
) -> Option<&'a (i32, i32)> {
    if event.pool_id == Pubkey::default() {
        return None;
    }
    let account_keys = transaction
        .as_ref()
        .and_then(|tx| tx.message.as_ref())
        .map(|msg| &msg.account_keys);
    only_match(invokes.iter().filter(|invoke| {
        use crate::instr::raydium_cpmm::discriminators::{SWAP_BASE_IN, SWAP_BASE_OUT};
        let Some(data) =
            crate::core::common_filler::get_instruction_data(meta, transaction, invoke)
        else {
            return false;
        };
        let name = match data.get(..8) {
            Some(disc) if disc == SWAP_BASE_IN && event.base_input => "swap_base_input",
            Some(disc) if disc == SWAP_BASE_OUT && !event.base_input => "swap_base_output",
            _ => return false,
        };
        if data.len() < 24
            || instruction_account_count(meta, transaction, invoke) < 13
            || (!event.ix_name.is_empty() && event.ix_name != name)
        {
            return false;
        }
        get_instruction_account_getter(
            meta,
            transaction,
            account_keys,
            &meta.loaded_writable_addresses,
            &meta.loaded_readonly_addresses,
            invoke,
        )
        .is_some_and(|get| {
            let matches = |known: Pubkey, index| known == Pubkey::default() || get(index) == known;
            get(3) == event.pool_id
                && matches(event.payer, 0)
                && matches(event.input_token_account, 4)
                && matches(event.output_token_account, 5)
                && matches(event.input_vault, 6)
                && matches(event.output_vault, 7)
                && matches(event.input_token_mint, 10)
                && matches(event.output_token_mint, 11)
                && matches(event.input_mint, 10)
                && matches(event.output_mint, 11)
        })
    }))
}

fn find_clmm_swap_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    event: &RaydiumClmmSwapEvent,
) -> Option<&'a (i32, i32)> {
    let account_keys = transaction
        .as_ref()?
        .message
        .as_ref()
        .map(|msg| &msg.account_keys);
    only_match(invokes.iter().filter(|invoke| {
        use crate::instr::raydium_clmm::discriminators::{SWAP, SWAP_V2};
        let Some(data) =
            crate::core::common_filler::get_instruction_data(meta, transaction, invoke)
        else {
            return false;
        };
        let name = match data.get(..8) {
            Some(disc) if disc == SWAP => "swap",
            Some(disc) if disc == SWAP_V2 => "swap_v2",
            _ => return false,
        };
        if !event.ix_name.is_empty() && event.ix_name != name {
            return false;
        }
        get_instruction_account_getter(
            meta,
            transaction,
            account_keys,
            &meta.loaded_writable_addresses,
            &meta.loaded_readonly_addresses,
            invoke,
        )
        .is_some_and(|get| {
            let matches = |key: Pubkey, index| key == Pubkey::default() || get(index) == key;
            event.pool_state != Pubkey::default()
                && get(2) == event.pool_state
                && matches(event.sender, 0)
                // Log events use canonical token0/token1 order, which can be
                // reversed relative to instruction input/output accounts.
                && ((matches(event.token_account_0, 3) && matches(event.token_account_1, 4))
                    || (matches(event.token_account_0, 4) && matches(event.token_account_1, 3)))
                && matches(event.input_mint, 11)
                && matches(event.output_mint, 12)
        })
    }))
}

#[derive(Clone, Copy)]
enum ClmmManagementContext<'a> {
    Create(&'a RaydiumClmmCreatePoolEvent),
    Open(&'a RaydiumClmmOpenPositionEvent),
    Close(&'a RaydiumClmmClosePositionEvent),
    Increase(&'a RaydiumClmmIncreaseLiquidityEvent),
    Decrease(&'a RaydiumClmmDecreaseLiquidityEvent),
}

// Strict wire shapes prevent a discriminator prefix or another version from
// supplying account context. Instruction limits are not executed log quantities.
fn clmm_management_shape(data: &[u8], count: usize) -> Option<(u8, usize)> {
    use crate::instr::raydium_clmm::discriminators::*;
    let disc = data.get(..8)?;
    let valid_option_bool = |offset: usize| match data.get(offset) {
        Some(0) => data.len() == offset + 1,
        Some(1) => data.len() == offset + 2 && data[offset + 1] <= 1,
        _ => false,
    };
    let shape = if disc == CREATE_POOL && count >= 13 && data.len() == 32 {
        (0, 2)
    } else if disc == CREATE_CUSTOMIZABLE_POOL
        && count >= 13
        && data.len() == 26
        && data[24] <= 2
        && data[25] <= 1
    {
        (0, 2)
    } else if disc == OPEN_POSITION && count >= 19 && data.len() == 56 {
        (1, 5)
    } else if disc == OPEN_POSITION_V2
        && count >= 22
        && data.get(56).is_some_and(|v| *v <= 1)
        && valid_option_bool(57)
    {
        (1, 5)
    } else if disc == OPEN_POSITION_WITH_TOKEN_22_NFT
        && count >= 20
        && data.get(56).is_some_and(|v| *v <= 1)
        && valid_option_bool(57)
    {
        (1, 4)
    } else if disc == CLOSE_POSITION && count >= 6 && data.len() == 8 {
        (2, 0)
    } else if (disc == INCREASE_LIQUIDITY && count >= 12 && data.len() == 40)
        || (disc == INCREASE_LIQUIDITY_V2 && count >= 15 && valid_option_bool(40))
    {
        (3, 2)
    } else if (disc == DECREASE_LIQUIDITY && count >= 12 && data.len() == 40)
        || (disc == DECREASE_LIQUIDITY_V2 && count >= 16 && data.len() == 40)
    {
        (4, 3)
    } else {
        return None;
    };
    Some(shape)
}

fn find_clmm_management_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    event: ClmmManagementContext<'_>,
) -> Option<&'a (i32, i32)> {
    let nft = match event {
        ClmmManagementContext::Increase(e) => e.position_nft_mint,
        ClmmManagementContext::Decrease(e) => e.position_nft_mint,
        ClmmManagementContext::Close(e) => e.position_nft_mint,
        _ => Pubkey::default(),
    };
    // nft_account is a token account, not its mint. The official personal
    // position PDA proves which NFT mint a liquidity invocation belongs to.
    let personal = (nft != Pubkey::default()).then(|| {
        Pubkey::find_program_address(
            &[b"position", nft.as_ref()],
            &crate::instr::program_ids::RAYDIUM_CLMM_PROGRAM_ID,
        )
        .0
    });
    find_context_invoke(invokes, meta, transaction, |data, get, count| {
        let Some((kind, pool_index)) = clmm_management_shape(data, count) else {
            return false;
        };
        match event {
            ClmmManagementContext::Create(e) if kind == 0 => known_accounts_match(
                get,
                &[
                    (e.creator, 0),
                    (e.pool, pool_index),
                    (e.token_0_mint, 3),
                    (e.token_1_mint, 4),
                    (e.token_vault_0, 5),
                    (e.token_vault_1, 6),
                ],
            ),
            ClmmManagementContext::Open(e) if kind == 1 => known_accounts_match(
                get,
                &[(e.user, 1), (e.pool, pool_index), (e.position_nft_mint, 2)],
            ),
            ClmmManagementContext::Close(e) if kind == 2 => {
                known_accounts_match(get, &[(e.user, 0), (e.position_nft_mint, 1)])
                    && personal.map_or(true, |key| get(3) == key)
            }
            ClmmManagementContext::Increase(e) if kind == 3 => {
                known_accounts_match(get, &[(e.user, 0), (e.pool, pool_index)])
                    && personal.map_or(true, |key| get(4) == key)
            }
            ClmmManagementContext::Decrease(e) if kind == 4 => {
                known_accounts_match(get, &[(e.user, 0), (e.pool, pool_index)])
                    && personal.map_or(true, |key| get(2) == key)
            }
            _ => false,
        }
    })
}

fn damm_initialize_shape(data: &[u8], count: usize) -> Option<bool> {
    use crate::instr::meteora_damm::discriminators::INITIALIZE_POOL;
    const INITIALIZE_POOL_WITH_DYNAMIC_CONFIG: [u8; 8] = [149, 82, 72, 197, 253, 252, 68, 15];
    let disc = data.get(..8)?;
    if disc == INITIALIZE_POOL && count >= 20 {
        return match data.get(40) {
            Some(0) if data.len() == 41 => Some(false),
            Some(1) if data.len() == 49 => Some(false),
            _ => None,
        };
    }
    if disc != INITIALIZE_POOL_WITH_DYNAMIC_CONFIG || count < 21 {
        return None;
    }
    // PoolFeeParameters = 27 base-fee bytes + u16 + u8 + Option<32-byte dynamic fee>.
    let shift = match data.get(38) {
        Some(0) => 0,
        Some(1) => 32,
        _ => return None,
    };
    if *data.get(71 + shift)? > 1 {
        return None;
    } // has_alpha_vault
    match data.get(106 + shift) {
        Some(0) if data.len() == 107 + shift => Some(true),
        Some(1) if data.len() == 115 + shift => Some(true),
        _ => None,
    }
}

fn find_damm_initialize_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    event: &MeteoraDammV2InitializePoolEvent,
) -> Option<&'a (i32, i32)> {
    find_context_invoke(invokes, meta, transaction, |data, get, count| {
        let Some(dynamic) = damm_initialize_shape(data, count) else {
            return false;
        };
        let shift = usize::from(dynamic);
        known_accounts_match(
            get,
            &[
                (event.creator, 0),
                (event.payer, 3),
                (event.position_nft_mint, 1),
                (event.pool, 6 + shift),
                (event.position, 7 + shift),
                (event.token_a_mint, 8 + shift),
                (event.token_b_mint, 9 + shift),
            ],
        )
    })
}

// Match trade layout and mint before filling missing account fields. Creation
// instructions and other pools must never supply a trade's account context.
fn find_pumpfun_trade_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    event: &PumpFunTradeEvent,
) -> Option<&'a (i32, i32)> {
    let account_keys = transaction
        .as_ref()?
        .message
        .as_ref()
        .map(|msg| &msg.account_keys);
    only_match(invokes.iter().filter(|invoke| {
        let data = if invoke.1 >= 0 {
            meta.inner_instructions
                .iter()
                .find(|g| g.index == invoke.0 as u32)
                .and_then(|g| g.instructions.get(invoke.1 as usize))
                .map(|ix| ix.data.as_slice())
        } else {
            transaction
                .as_ref()
                .and_then(|tx| tx.message.as_ref())
                .and_then(|msg| msg.instructions.get(invoke.0 as usize))
                .map(|ix| ix.data.as_slice())
        };
        use crate::instr::pump::discriminators::*;
        let (mint_idx, user_idx, is_buy, minimum) = match data.and_then(|data| data.get(..8)) {
            Some(disc) if disc == BUY || disc == BUY_EXACT_SOL_IN => (2, 6, true, 16),
            Some(disc) if disc == SELL => (2, 6, false, 14),
            Some(disc) if disc == BUY_V2 || disc == BUY_EXACT_QUOTE_IN_V2 => (1, 13, true, 27),
            Some(disc) if disc == SELL_V2 => (1, 13, false, 26),
            Some(disc) if disc == BUY_V3 || disc == BUY_EXACT_QUOTE_IN_V3 => (1, 8, true, 17),
            Some(disc) if disc == SELL_V3 => (1, 8, false, 17),
            _ => return false,
        };
        instruction_account_count(meta, transaction, invoke) >= minimum
            && is_buy == event.is_buy
            && get_instruction_account_getter(
                meta,
                transaction,
                account_keys,
                &meta.loaded_writable_addresses,
                &meta.loaded_readonly_addresses,
                invoke,
            )
            .is_some_and(|get| {
                event.mint != Pubkey::default()
                    && get(mint_idx) == event.mint
                    && (event.user == Pubkey::default() || get(user_idx) == event.user)
            })
    }))
}

fn find_pumpfun_create_invoke<'a>(
    invokes: &'a [(i32, i32)],
    transaction: &Option<Transaction>,
    v2_only: bool,
    meta: &TransactionStatusMeta,
    mint: Pubkey,
) -> Option<(&'a (i32, i32), bool)> {
    let keys = transaction
        .as_ref()?
        .message
        .as_ref()
        .map(|msg| &msg.account_keys);
    only_match(invokes.iter().filter_map(|invoke| {
        let (data, count) = if invoke.1 >= 0 {
            let ix = meta
                .inner_instructions
                .iter()
                .find(|g| g.index == invoke.0 as u32)?
                .instructions
                .get(invoke.1 as usize)?;
            (ix.data.as_slice(), ix.accounts.len())
        } else {
            let ix = transaction
                .as_ref()?
                .message
                .as_ref()?
                .instructions
                .get(invoke.0 as usize)?;
            (ix.data.as_slice(), ix.accounts.len())
        };
        use crate::instr::pump::discriminators::{CREATE, CREATE_V2};
        let v2 = data.get(..8)? == CREATE_V2;
        if (!v2 && (v2_only || data.get(..8)? != CREATE)) || count < if v2 { 16 } else { 14 } {
            return None;
        }
        let get = get_instruction_account_getter(
            meta,
            transaction,
            keys,
            &meta.loaded_writable_addresses,
            &meta.loaded_readonly_addresses,
            invoke,
        )?;
        if mint != Pubkey::default() && get(0) != mint {
            return None;
        }
        Some((invoke, v2))
    }))
}

// Select the actual liquidity instruction by discriminator and known pool.
// Account count alone can select an unrelated swap/deposit in the same transaction.
fn find_pools_liquidity_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    pool: Pubkey,
    bootstrap: bool,
    ix_name: &str,
) -> Option<(&'a (i32, i32), &'static str)> {
    let keys = transaction
        .as_ref()?
        .message
        .as_ref()
        .map(|msg| &msg.account_keys);
    only_match(invokes.iter().filter_map(|invoke| {
        let data = if invoke.1 >= 0 {
            meta.inner_instructions
                .iter()
                .find(|g| g.index == invoke.0 as u32)
                .and_then(|g| g.instructions.get(invoke.1 as usize))
                .map(|ix| ix.data.as_slice())
        } else {
            transaction
                .as_ref()
                .and_then(|tx| tx.message.as_ref())
                .and_then(|msg| msg.instructions.get(invoke.0 as usize))
                .map(|ix| ix.data.as_slice())
        }?;
        use crate::instr::meteora_amm::discriminators::*;
        let name = match data.get(..8)? {
            d if bootstrap && d == BOOTSTRAP_LIQUIDITY => "bootstrap_liquidity",
            d if !bootstrap && d == REMOVE_LIQUIDITY => "remove_balance_liquidity",
            d if !bootstrap && d == REMOVE_LIQUIDITY_SINGLE_SIDE => "remove_liquidity_single_side",
            _ => return None,
        };
        if !ix_name.is_empty() && ix_name != name {
            return None;
        }
        let get = get_instruction_account_getter(
            meta,
            transaction,
            keys,
            &meta.loaded_writable_addresses,
            &meta.loaded_readonly_addresses,
            invoke,
        )?;
        if pool != Pubkey::default() && get(0) != pool {
            return None;
        }
        Some((invoke, name))
    }))
}

fn find_pools_management_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    pool: Pubkey,
    creation: bool,
    ix_name: &str,
) -> Option<(&'a (i32, i32), &'static str)> {
    let keys = transaction
        .as_ref()?
        .message
        .as_ref()
        .map(|msg| &msg.account_keys);
    only_match(invokes.iter().filter_map(|invoke| {
        let data = if invoke.1 >= 0 {
            meta.inner_instructions
                .iter()
                .find(|g| g.index == invoke.0 as u32)
                .and_then(|g| g.instructions.get(invoke.1 as usize))
                .map(|ix| ix.data.as_slice())
        } else {
            transaction
                .as_ref()
                .and_then(|tx| tx.message.as_ref())
                .and_then(|msg| msg.instructions.get(invoke.0 as usize))
                .map(|ix| ix.data.as_slice())
        }?;
        use crate::instr::meteora_amm::discriminators::*;
        let name = match data.get(..8)? {
            d if creation && d == CREATE_POOL => {
                "initialize_permissionless_constant_product_pool_with_config"
            }
            d if creation && d == CREATE_POOL_WITH_CONFIG2 => {
                "initialize_permissionless_constant_product_pool_with_config2"
            }
            d if creation && d == INITIALIZE_PERMISSIONED_POOL => "initialize_permissioned_pool",
            d if creation && d == INITIALIZE_PERMISSIONLESS_POOL => {
                "initialize_permissionless_pool"
            }
            d if creation && d == INITIALIZE_PERMISSIONLESS_POOL_WITH_FEE_TIER => {
                "initialize_permissionless_pool_with_fee_tier"
            }
            d if creation && d == INITIALIZE_CUSTOMIZABLE_POOL => {
                "initialize_customizable_permissionless_constant_product_pool"
            }
            d if !creation && d == SET_POOL_FEES => "set_pool_fees",
            _ => return None,
        };
        if !ix_name.is_empty() && ix_name != name {
            return None;
        }
        let get = get_instruction_account_getter(
            meta,
            transaction,
            keys,
            &meta.loaded_writable_addresses,
            &meta.loaded_readonly_addresses,
            invoke,
        )?;
        if pool != Pubkey::default() && get(0) != pool {
            return None;
        }
        Some((invoke, name))
    }))
}

/// Match one instruction's opcode and all known account identities; emitted
/// event-CPI payloads never match the expected instruction opcode.
fn find_amm_swap_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    e: &RaydiumAmmV4SwapEvent,
) -> Option<&'a (i32, i32)> {
    use crate::instr::raydium_amm::discriminators::*;
    find_context_invoke(invokes, meta, transaction, |data, get, count| {
        let (name, source, destination, owner, vault) = match data.first() {
            Some(&SWAP_BASE_IN) if count >= 17 => (
                "swap_base_in",
                15 - usize::from(count == 17),
                16 - usize::from(count == 17),
                17 - usize::from(count == 17),
                5 - usize::from(count == 17),
            ),
            Some(&SWAP_BASE_OUT) if count >= 17 => (
                "swap_base_out",
                15 - usize::from(count == 17),
                16 - usize::from(count == 17),
                17 - usize::from(count == 17),
                5 - usize::from(count == 17),
            ),
            Some(&SWAP_BASE_IN_V2) if count >= 8 => ("swap_base_in_v2", 5, 6, 7, 3),
            Some(&SWAP_BASE_OUT_V2) if count >= 8 => ("swap_base_out_v2", 5, 6, 7, 3),
            _ => return false,
        };
        data.len() >= 17
            && (e.ix_name.is_empty() || e.ix_name == name)
            && known_accounts_match(
                get,
                &[
                    (e.amm, 1),
                    (e.token_program, 0),
                    (e.amm_authority, 2),
                    (e.user_source_owner, owner),
                    (e.user_source_token_account, source),
                    (e.user_destination_token_account, destination),
                    (e.pool_coin_token_account, vault),
                    (e.pool_pc_token_account, vault + 1),
                ],
            )
    })
}

fn find_pumpswap_create_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    e: &PumpSwapCreatePoolEvent,
) -> Option<&'a (i32, i32)> {
    find_identity_invoke(
        invokes,
        meta,
        transaction,
        &crate::instr::pump_amm::discriminators::CREATE_POOL,
        58,
        18,
        &[
            (e.pool, 0),
            (e.creator, 2),
            (e.base_mint, 3),
            (e.quote_mint, 4),
            (e.lp_mint, 5),
            (e.user_base_token_account, 6),
            (e.user_quote_token_account, 7),
        ],
    )
}

fn find_launchlab_trade_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    e: &RaydiumLaunchlabTradeEvent,
) -> Option<&'a (i32, i32)> {
    use crate::instr::raydium_launchlab::discriminators::*;
    find_context_invoke(invokes, meta, transaction, |data, get, count| {
        let is_buy = match data.get(..8) {
            Some(d) if d == BUY_EXACT_IN || d == BUY_EXACT_OUT => true,
            Some(d) if d == SELL_EXACT_IN || d == SELL_EXACT_OUT => false,
            _ => return false,
        };
        data.len() >= 24
            && count >= 15
            && is_buy == e.is_buy
            && known_accounts_match(
                get,
                &[
                    (e.pool_state, 4),
                    (e.user, 0),
                    (e.base_mint, 9),
                    (e.quote_mint, 10),
                    (e.user_base_token, 5),
                    (e.user_quote_token, 6),
                    (e.base_vault, 7),
                    (e.quote_vault, 8),
                    (e.global_config, 2),
                    (e.platform_config, 3),
                    (e.base_token_program, 11),
                    (e.quote_token_program, 12),
                    (e.system_program, 15),
                    (e.platform_associated_account, 16),
                    (e.creator_associated_account, 17),
                ],
            )
    })
}

// Validate borrowed Borsh strings without allocating decoded event values.
fn launchlab_mint_params_complete(data: &[u8]) -> bool {
    let mut offset = 9usize; // discriminator + decimals
    for _ in 0..3 {
        let Some(prefix_end) = offset.checked_add(4) else {
            return false;
        };
        let Some(length) = data
            .get(offset..prefix_end)
            .and_then(|v| <[u8; 4]>::try_from(v).ok())
            .map(u32::from_le_bytes)
        else {
            return false;
        };
        offset = prefix_end;
        let Some(end) = offset.checked_add(length as usize) else {
            return false;
        };
        let Some(text) = data.get(offset..end) else {
            return false;
        };
        if std::str::from_utf8(text).is_err() {
            return false;
        }
        offset = end;
    }
    true
}

fn find_launchlab_create_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    e: &RaydiumLaunchlabPoolCreateEvent,
) -> Option<&'a (i32, i32)> {
    use crate::instr::raydium_launchlab::discriminators::*;
    find_context_invoke(invokes, meta, transaction, |data, get, count| {
        let minimum = match data.get(..8) {
            Some(d) if d == INITIALIZE || d == INITIALIZE_V2 => 18,
            Some(d) if d == INITIALIZE_WITH_TOKEN_2022 => 15,
            _ => return false,
        };
        count >= minimum
            && launchlab_mint_params_complete(data)
            && known_accounts_match(
                get,
                &[
                    (e.pool_state, 5),
                    (e.payer, 0),
                    (e.creator, 1),
                    (e.base_mint, 6),
                    (e.quote_mint, 7),
                    (e.base_vault, 8),
                    (e.quote_vault, 9),
                    (e.global_config, 2),
                    (e.platform_config, 3),
                    (e.base_token_program, if minimum == 15 { 10 } else { 11 }),
                    (e.quote_token_program, if minimum == 15 { 11 } else { 12 }),
                ],
            )
    })
}

fn find_identity_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    opcode: &[u8],
    minimum_data: usize,
    minimum_accounts: usize,
    known: &[(Pubkey, usize)],
) -> Option<&'a (i32, i32)> {
    find_context_invoke(invokes, meta, transaction, |data, get, count| {
        data.starts_with(opcode)
            && data.len() >= minimum_data
            && count >= minimum_accounts
            && known_accounts_match(get, known)
    })
}

macro_rules! fill_event_accounts_with_invoke {
    ($event:expr, $meta:expr, $tx:expr, $invoke:expr, $filler:expr) => {{
        let account_keys = $tx
            .as_ref()
            .and_then(|tx| tx.message.as_ref())
            .map(|msg| &msg.account_keys);
        if let Some(get_account) = get_instruction_account_getter(
            $meta,
            $tx,
            account_keys,
            &$meta.loaded_writable_addresses,
            &$meta.loaded_readonly_addresses,
            $invoke,
        ) {
            $filler(&get_account);
        }
    }};
}

// ============================================================================
// Public API
// ============================================================================

// PumpSwap events must be enriched from their own direction and user. Pool
// alone is ambiguous when one transaction trades repeatedly in the same pool.
pub(crate) fn find_pumpswap_trade_invoke<'a>(
    invokes: &'a [(i32, i32)],
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    pool: Pubkey,
    user: Pubkey,
    buy: bool,
) -> Option<&'a (i32, i32)> {
    if pool == Pubkey::default() {
        return None;
    }
    let keys = transaction
        .as_ref()?
        .message
        .as_ref()
        .map(|msg| &msg.account_keys);
    let mut matches = invokes.iter().filter(|invoke| {
        let data = if invoke.1 >= 0 {
            meta.inner_instructions
                .iter()
                .find(|group| group.index == invoke.0 as u32)
                .and_then(|group| group.instructions.get(invoke.1 as usize))
                .map(|ix| ix.data.as_slice())
        } else {
            transaction
                .as_ref()
                .and_then(|tx| tx.message.as_ref())
                .and_then(|msg| msg.instructions.get(invoke.0 as usize))
                .map(|ix| ix.data.as_slice())
        };
        use crate::instr::pump_amm::discriminators::{
            BUY, BUY_EXACT_QUOTE_IN, BUY_EXACT_QUOTE_IN_V2, BUY_V2, SELL, SELL_V2,
        };
        let direction_matches = match data.and_then(|data| data.get(..8)) {
            Some(disc) if buy => {
                disc == BUY
                    || disc == BUY_EXACT_QUOTE_IN
                    || disc == BUY_V2
                    || disc == BUY_EXACT_QUOTE_IN_V2
            }
            Some(disc) => disc == SELL || disc == SELL_V2,
            None => false,
        };
        let compact = data
            .and_then(|d| d.get(..8))
            .is_some_and(|disc| disc == BUY_V2 || disc == BUY_EXACT_QUOTE_IN_V2 || disc == SELL_V2);
        if !direction_matches
            || instruction_account_count(meta, transaction, invoke)
                < if compact {
                    17
                } else if buy {
                    23
                } else {
                    21
                }
        {
            return false;
        }
        get_instruction_account_getter(
            meta,
            transaction,
            keys,
            &meta.loaded_writable_addresses,
            &meta.loaded_readonly_addresses,
            invoke,
        )
        .is_some_and(|get| get(0) == pool && (user == Pubkey::default() || get(1) == user))
    });
    let matched = matches.next()?;
    matches.next().is_none().then_some(matched)
}

fn fill_dlmm_swap_event(
    event: &mut MeteoraDlmmSwapEvent,
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    invokes: &[(i32, i32)],
) {
    if event.pool == Pubkey::default() {
        return;
    }
    let keys = transaction
        .as_ref()
        .and_then(|tx| tx.message.as_ref())
        .map(|m| &m.account_keys);
    let matched = only_match(invokes.iter().filter_map(|invoke| {
        let (data, count) = if invoke.1 >= 0 {
            let ix = meta
                .inner_instructions
                .iter()
                .find(|g| g.index == invoke.0 as u32)?
                .instructions
                .get(invoke.1 as usize)?;
            (ix.data.as_slice(), ix.accounts.len())
        } else {
            let ix = transaction
                .as_ref()?
                .message
                .as_ref()?
                .instructions
                .get(invoke.0 as usize)?;
            (ix.data.as_slice(), ix.accounts.len())
        };
        let start = crate::instr::meteora_dlmm::validate_swap_layout(data, count)?;
        let get = get_instruction_account_getter(
            meta,
            transaction,
            keys,
            &meta.loaded_writable_addresses,
            &meta.loaded_readonly_addresses,
            invoke,
        )?;
        let matches = |key: Pubkey, index| key == Pubkey::default() || get(index) == key;
        if get(0) != event.pool
            || !matches(event.from, 10)
            || !matches(event.user_token_in, 4)
            || !matches(event.user_token_out, 5)
        {
            return None;
        }
        use crate::instr::meteora_dlmm::discriminators::*;
        let v2 = data
            .get(..8)
            .is_some_and(|d| d == SWAP2 || d == SWAP_EXACT_OUT2 || d == SWAP_WITH_PRICE_IMPACT2);
        Some((invoke, start, count, v2))
    }));
    if let Some((invoke, start, count, v2)) = matched {
        if let Some(get) = get_instruction_account_getter(
            meta,
            transaction,
            keys,
            &meta.loaded_writable_addresses,
            &meta.loaded_readonly_addresses,
            invoke,
        ) {
            account_fillers::meteora::fill_dlmm_swap_accounts_with_layout(
                event, &get, start, count, v2,
            );
        }
    }
}

fn instruction_account_count(
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    invoke: &(i32, i32),
) -> usize {
    if invoke.1 >= 0 {
        meta.inner_instructions
            .iter()
            .find(|group| group.index == invoke.0 as u32)
            .and_then(|group| group.instructions.get(invoke.1 as usize))
            .map_or(0, |ix| ix.accounts.len())
    } else {
        transaction
            .as_ref()
            .and_then(|tx| tx.message.as_ref())
            .and_then(|msg| msg.instructions.get(invoke.0 as usize))
            .map_or(0, |ix| ix.accounts.len())
    }
}

macro_rules! fill_pumpswap_accounts_anchored {
    ($meta:expr, $tx:expr, $invokes:expr, $program_id:expr, $anchor:expr, $user:expr, $buy:expr, $filler:expr) => {
        if let Some(invokes) = $invokes.get($program_id) {
            let account_keys = $tx
                .as_ref()
                .and_then(|tx| tx.message.as_ref())
                .map(|msg| &msg.account_keys);
            if let Some(invoke) =
                find_pumpswap_trade_invoke(invokes, $meta, $tx, *$anchor, $user, $buy)
            {
                if let Some(get_account) = get_instruction_account_getter(
                    $meta,
                    $tx,
                    account_keys,
                    &$meta.loaded_writable_addresses,
                    &$meta.loaded_readonly_addresses,
                    invoke,
                ) {
                    $filler(&get_account, instruction_account_count($meta, $tx, invoke));
                }
            }
        }
    };
}

/// 从交易 meta 将缺失账户填入事件（`program_invokes`: program id → (outer, inner) 索引列表）
pub fn fill_accounts_with_owned_keys(
    event: &mut DexEvent,
    meta: &TransactionStatusMeta,
    transaction: &Option<Transaction>,
    program_invokes: &HashMap<Pubkey, Vec<(i32, i32)>>,
) {
    use crate::grpc::program_ids::*;

    match event {
        // PumpFun
        DexEvent::PumpFunTrade(e)
        | DexEvent::PumpFunBuy(e)
        | DexEvent::PumpFunSell(e)
        | DexEvent::PumpFunBuyExactSolIn(e) => {
            if let Some(invokes) = program_invokes.get(&PUMPFUN_PROGRAM) {
                if let Some(invoke) = find_pumpfun_trade_invoke(invokes, meta, transaction, e) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::pumpfun::fill_trade_accounts(e, get);
                        }
                    );
                }
            }
        }
        DexEvent::PumpFunCreate(e) => {
            if let Some(invokes) = program_invokes.get(&PUMPFUN_PROGRAM) {
                if let Some((invoke, v2)) =
                    find_pumpfun_create_invoke(invokes, transaction, false, meta, e.mint)
                {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            if v2 {
                                account_fillers::pumpfun::fill_create_accounts_from_v2(e, get);
                            } else {
                                account_fillers::pumpfun::fill_create_accounts(e, get);
                            }
                        }
                    );
                }
            }
        }
        DexEvent::PumpFunCreateV2(e) => {
            if let Some(invokes) = program_invokes.get(&PUMPFUN_PROGRAM) {
                if let Some((invoke, _)) =
                    find_pumpfun_create_invoke(invokes, transaction, true, meta, e.mint)
                {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::pumpfun::fill_create_v2_accounts(e, get);
                        }
                    );
                }
            }
        }

        // PumpSwap
        DexEvent::PumpSwapBuy(e) => {
            let pool = e.pool;
            fill_pumpswap_accounts_anchored!(
                meta,
                transaction,
                program_invokes,
                &PUMPSWAP_PROGRAM,
                &pool,
                e.user,
                true,
                |get: &AccountGetter<'_>, count: usize| {
                    account_fillers::pumpswap::fill_buy_accounts_with_count(e, get, count);
                }
            );
        }
        DexEvent::PumpSwapSell(e) => {
            let pool = e.pool;
            fill_pumpswap_accounts_anchored!(
                meta,
                transaction,
                program_invokes,
                &PUMPSWAP_PROGRAM,
                &pool,
                e.user,
                false,
                |get: &AccountGetter<'_>, count: usize| {
                    account_fillers::pumpswap::fill_sell_accounts_with_count(e, get, count);
                }
            );
        }
        DexEvent::PumpSwapCreatePool(e) => {
            if let Some(invokes) = program_invokes.get(&PUMPSWAP_PROGRAM) {
                if let Some(invoke) = find_pumpswap_create_invoke(invokes, meta, transaction, e) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::pumpswap::fill_create_pool_accounts(e, get);
                        }
                    );
                }
            }
        }

        // Raydium CLMM
        DexEvent::RaydiumClmmSwap(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_CLMM_PROGRAM) {
                if let Some(invoke) = find_clmm_swap_invoke(invokes, meta, transaction, e) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium::fill_clmm_swap_accounts_with_count(
                                e,
                                get,
                                instruction_account_count(meta, transaction, invoke),
                            );
                        }
                    );
                }
            }
        }
        DexEvent::RaydiumClmmCreatePool(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_CLMM_PROGRAM) {
                if let Some(invoke) = find_clmm_management_invoke(
                    invokes,
                    meta,
                    transaction,
                    ClmmManagementContext::Create(e),
                ) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium::fill_clmm_create_pool_accounts(e, get);
                        }
                    );
                }
            }
        }
        DexEvent::RaydiumClmmOpenPosition(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_CLMM_PROGRAM) {
                if let Some(invoke) = find_clmm_management_invoke(
                    invokes,
                    meta,
                    transaction,
                    ClmmManagementContext::Open(e),
                ) {
                    if let Some(data) =
                        crate::core::common_filler::get_instruction_data(meta, transaction, invoke)
                    {
                        let token_22_nft = data.get(..8) == Some(&crate::instr::raydium_clmm::discriminators::OPEN_POSITION_WITH_TOKEN_22_NFT);
                        fill_event_accounts_with_invoke!(
                            e,
                            meta,
                            transaction,
                            invoke,
                            |get: &AccountGetter<'_>| {
                                account_fillers::raydium::fill_clmm_open_position_accounts_with_layout(e, get, token_22_nft);
                            }
                        );
                    }
                }
            }
        }
        DexEvent::RaydiumClmmClosePosition(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_CLMM_PROGRAM) {
                if let Some(invoke) = find_clmm_management_invoke(
                    invokes,
                    meta,
                    transaction,
                    ClmmManagementContext::Close(e),
                ) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium::fill_clmm_close_position_accounts(e, get);
                        }
                    );
                }
            }
        }
        DexEvent::RaydiumClmmIncreaseLiquidity(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_CLMM_PROGRAM) {
                if let Some(invoke) = find_clmm_management_invoke(
                    invokes,
                    meta,
                    transaction,
                    ClmmManagementContext::Increase(e),
                ) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium::fill_clmm_increase_liquidity_accounts(e, get);
                        }
                    );
                }
            }
        }
        DexEvent::RaydiumClmmDecreaseLiquidity(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_CLMM_PROGRAM) {
                if let Some(invoke) = find_clmm_management_invoke(
                    invokes,
                    meta,
                    transaction,
                    ClmmManagementContext::Decrease(e),
                ) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium::fill_clmm_decrease_liquidity_accounts(e, get);
                        }
                    );
                }
            }
        }

        // Raydium CPMM
        DexEvent::RaydiumCpmmSwap(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_CPMM_PROGRAM) {
                if let Some(invoke) = find_cpmm_swap_invoke(invokes, meta, transaction, e) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium::fill_cpmm_swap_accounts(e, get);
                        }
                    );
                }
            }
        }
        DexEvent::RaydiumCpmmDeposit(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_CPMM_PROGRAM) {
                if let Some(invoke) = find_identity_invoke(
                    invokes,
                    meta,
                    transaction,
                    &crate::instr::raydium_cpmm::discriminators::DEPOSIT,
                    32,
                    13,
                    &[(e.pool, 2), (e.user, 0)],
                ) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium::fill_cpmm_deposit_accounts(e, get);
                        }
                    );
                }
            }
        }
        DexEvent::RaydiumCpmmWithdraw(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_CPMM_PROGRAM) {
                if let Some(invoke) = find_identity_invoke(
                    invokes,
                    meta,
                    transaction,
                    &crate::instr::raydium_cpmm::discriminators::WITHDRAW,
                    32,
                    14,
                    &[(e.pool, 2), (e.user, 0)],
                ) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium::fill_cpmm_withdraw_accounts(e, get);
                        }
                    );
                }
            }
        }
        DexEvent::RaydiumCpmmInitialize(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_CPMM_PROGRAM) {
                if let Some(invoke) = find_identity_invoke(
                    invokes,
                    meta,
                    transaction,
                    &crate::instr::raydium_cpmm::discriminators::INITIALIZE,
                    32,
                    20,
                    &[(e.pool, 3), (e.creator, 0)],
                ) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium::fill_cpmm_initialize_accounts(e, get);
                        }
                    );
                }
            }
        }

        // Raydium AMM V4
        DexEvent::RaydiumAmmV4Swap(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_AMM_V4_PROGRAM) {
                if let Some(invoke) = find_amm_swap_invoke(invokes, meta, transaction, e) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium::fill_amm_v4_swap_accounts(e, get);
                        }
                    );
                }
            }
        }
        DexEvent::RaydiumAmmV4Deposit(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_AMM_V4_PROGRAM) {
                if let Some(invoke) = find_identity_invoke(
                    invokes,
                    meta,
                    transaction,
                    &[crate::instr::raydium_amm::discriminators::DEPOSIT],
                    25,
                    14,
                    &[(e.amm, 1), (e.user_owner, 12), (e.amm_authority, 2)],
                ) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium::fill_amm_v4_deposit_accounts(e, get);
                        }
                    );
                }
            }
        }
        DexEvent::RaydiumAmmV4Withdraw(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_AMM_V4_PROGRAM) {
                if let Some(invoke) = find_identity_invoke(
                    invokes,
                    meta,
                    transaction,
                    &[crate::instr::raydium_amm::discriminators::WITHDRAW],
                    9,
                    22,
                    &[
                        (e.amm, 1),
                        (e.user_owner, 18),
                        (e.amm_authority, 2),
                        (e.amm_open_orders, 3),
                    ],
                ) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium::fill_amm_v4_withdraw_accounts(e, get);
                        }
                    );
                }
            }
        }

        // Orca Whirlpool
        DexEvent::OrcaWhirlpoolSwap(e) => {
            if let Some(invokes) = program_invokes.get(&ORCA_WHIRLPOOL_PROGRAM) {
                if let Some(invoke) = find_orca_swap_invoke(invokes, meta, transaction, e) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::orca::fill_whirlpool_swap_accounts(e, get)
                        }
                    );
                }
            }
        }
        DexEvent::OrcaWhirlpoolLiquidityIncreased(e) => {
            if let Some(invokes) = program_invokes.get(&ORCA_WHIRLPOOL_PROGRAM) {
                if let Some(invoke) = find_orca_liquidity_invoke(
                    invokes,
                    meta,
                    transaction,
                    e.whirlpool,
                    e.position,
                    true,
                ) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::orca::fill_whirlpool_liquidity_increased_accounts(
                                e, get,
                            )
                        }
                    );
                }
            }
        }
        DexEvent::OrcaWhirlpoolLiquidityDecreased(e) => {
            if let Some(invokes) = program_invokes.get(&ORCA_WHIRLPOOL_PROGRAM) {
                if let Some(invoke) = find_orca_liquidity_invoke(
                    invokes,
                    meta,
                    transaction,
                    e.whirlpool,
                    e.position,
                    false,
                ) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::orca::fill_whirlpool_liquidity_decreased_accounts(
                                e, get,
                            )
                        }
                    );
                }
            }
        }
        DexEvent::MeteoraDammV2InitializePool(e) => {
            if let Some(invokes) = program_invokes.get(&METEORA_DAMM_V2_PROGRAM) {
                if let Some(invoke) = find_damm_initialize_invoke(invokes, meta, transaction, e) {
                    if let Some(dynamic) =
                        crate::core::common_filler::get_instruction_data(meta, transaction, invoke)
                            .and_then(|data| {
                                damm_initialize_shape(
                                    data,
                                    instruction_account_count(meta, transaction, invoke),
                                )
                            })
                    {
                        fill_event_accounts_with_invoke!(
                            e,
                            meta,
                            transaction,
                            invoke,
                            |get: &AccountGetter<'_>| {
                                account_fillers::meteora::fill_damm_v2_initialize_pool_accounts_with_layout(e, get, dynamic);
                            }
                        );
                    }
                }
            }
        }

        // Meteora Pools
        DexEvent::MeteoraPoolsSwap(e) => {
            if let Some(invokes) = program_invokes.get(&METEORA_POOLS_PROGRAM) {
                if let Some(invoke) = find_pools_swap_invoke(invokes, meta, transaction, e) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::meteora::fill_pools_swap_accounts(e, get)
                        }
                    );
                }
            }
        }
        DexEvent::MeteoraPoolsAddLiquidity(e) => {
            if let Some(invokes) = program_invokes.get(&METEORA_POOLS_PROGRAM) {
                if let Some((invoke, name)) = find_pools_add_invoke(invokes, meta, transaction, e) {
                    if e.ix_name.is_empty() {
                        e.ix_name = name.into();
                    }
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::meteora::fill_pools_add_liquidity_accounts(e, get)
                        }
                    );
                }
            }
        }
        DexEvent::MeteoraPoolsRemoveLiquidity(e) => {
            if let Some(invokes) = program_invokes.get(&METEORA_POOLS_PROGRAM) {
                if let Some((invoke, name)) = find_pools_liquidity_invoke(
                    invokes,
                    meta,
                    transaction,
                    e.pool,
                    false,
                    &e.ix_name,
                ) {
                    if e.ix_name.is_empty() {
                        e.ix_name = name.into();
                    }
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::meteora::fill_pools_remove_liquidity_accounts(e, get);
                        }
                    );
                }
            }
        }
        DexEvent::MeteoraPoolsBootstrapLiquidity(e) => {
            if let Some(invokes) = program_invokes.get(&METEORA_POOLS_PROGRAM) {
                if let Some((invoke, name)) = find_pools_liquidity_invoke(
                    invokes,
                    meta,
                    transaction,
                    e.pool,
                    true,
                    &e.ix_name,
                ) {
                    if e.ix_name.is_empty() {
                        e.ix_name = name.into();
                    }
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::meteora::fill_pools_bootstrap_liquidity_accounts(
                                e, get,
                            );
                        }
                    );
                }
            }
        }

        DexEvent::MeteoraPoolsPoolCreated(e) => {
            if let Some(invokes) = program_invokes.get(&METEORA_POOLS_PROGRAM) {
                if let Some((invoke, name)) = find_pools_management_invoke(
                    invokes,
                    meta,
                    transaction,
                    e.pool,
                    true,
                    &e.ix_name,
                ) {
                    if e.ix_name.is_empty() {
                        e.ix_name = name.into();
                    }
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::meteora::fill_pools_pool_created_accounts(e, get);
                        }
                    );
                }
            }
        }
        DexEvent::MeteoraPoolsSetPoolFees(e) => {
            if let Some(invokes) = program_invokes.get(&METEORA_POOLS_PROGRAM) {
                if let Some((invoke, name)) = find_pools_management_invoke(
                    invokes,
                    meta,
                    transaction,
                    e.pool,
                    false,
                    &e.ix_name,
                ) {
                    if e.ix_name.is_empty() {
                        e.ix_name = name.into();
                    }
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::meteora::fill_pools_set_pool_fees_accounts(e, get);
                        }
                    );
                }
            }
        }
        // Meteora DLMM
        DexEvent::MeteoraDlmmSwap(e) => {
            if let Some(invokes) = program_invokes.get(&METEORA_DLMM_PROGRAM) {
                fill_dlmm_swap_event(e, meta, transaction, invokes);
            }
        }

        // RaydiumLaunchlab
        DexEvent::RaydiumLaunchlabTrade(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_LAUNCHLAB_PROGRAM) {
                if let Some(invoke) = find_launchlab_trade_invoke(invokes, meta, transaction, e) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium_launchlab::fill_trade_accounts(e, get);
                        }
                    );
                }
            }
        }
        DexEvent::RaydiumLaunchlabPoolCreate(e) => {
            if let Some(invokes) = program_invokes.get(&RAYDIUM_LAUNCHLAB_PROGRAM) {
                if let Some(invoke) = find_launchlab_create_invoke(invokes, meta, transaction, e) {
                    fill_event_accounts_with_invoke!(
                        e,
                        meta,
                        transaction,
                        invoke,
                        |get: &AccountGetter<'_>| {
                            account_fillers::raydium_launchlab::fill_pool_create_accounts(e, get);
                        }
                    );
                }
            }
        }

        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn create_selector_matches_actual_layout_mint_and_cpi_without_guessing() {
        use yellowstone_grpc_proto::prelude::{InnerInstruction, InnerInstructions};
        for inner in [false, true] {
            for ambiguous in [false, true] {
                let keys: Vec<Pubkey> = (0..40).map(|_| Pubkey::new_unique()).collect();
                let create = CompiledInstruction {
                    accounts: (0..19).collect(),
                    data: crate::instr::pump::discriminators::CREATE_V2.to_vec(),
                    ..Default::default()
                };
                let mut other = create.clone();
                other.accounts[0] = 20;
                let buy = CompiledInstruction {
                    accounts: (0..30).collect(),
                    data: crate::instr::pump::discriminators::BUY.to_vec(),
                    ..Default::default()
                };
                let mut instructions = vec![create.clone(), other, buy];
                if ambiguous {
                    instructions.push(create);
                }
                let invokes: Vec<_> = (0..instructions.len())
                    .map(|i| if inner { (0, i as i32) } else { (i as i32, -1) })
                    .collect();
                let mut meta = TransactionStatusMeta::default();
                if inner {
                    meta.inner_instructions.push(InnerInstructions {
                        index: 0,
                        instructions: instructions
                            .iter()
                            .map(|ix| InnerInstruction {
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
                let selected = find_pumpfun_create_invoke(&invokes, &tx, false, &meta, keys[0]);
                if ambiguous {
                    assert!(selected.is_none());
                } else {
                    assert_eq!(selected, Some((&invokes[0], true)));
                }
                assert!(
                    find_pumpfun_create_invoke(&invokes, &tx, false, &meta, keys[39]).is_none()
                );
            }
        }
    }

    use super::*;
    use crate::core::events::{PumpSwapBuyEvent, PumpSwapSellEvent, RaydiumLaunchlabTradeEvent};
    use crate::grpc::program_ids::{PUMPSWAP_PROGRAM, RAYDIUM_LAUNCHLAB_PROGRAM};
    use yellowstone_grpc_proto::prelude::{
        CompiledInstruction, Message, MessageHeader, Transaction, TransactionStatusMeta,
    };

    #[test]
    fn pumpswap_dispatcher_preserves_actual_account_count_with_missing_alt_tail() {
        for count in [25usize, 27] {
            let mut keys: Vec<Pubkey> = (0..count).map(|_| Pubkey::new_unique()).collect();
            if count == 27 {
                keys[24] = Pubkey::find_program_address(
                    &[b"pool-v2", keys[3].as_ref()],
                    &PUMPSWAP_PROGRAM,
                )
                .0;
            }
            let pool = keys[0];
            let recipient = keys[count - 2];
            let mut accounts: Vec<u8> = (0..count as u8).collect();
            accounts[count - 1] = 200; // unresolved ALT index, not a shorter instruction
            let transaction = Some(Transaction {
                message: Some(Message {
                    account_keys: keys.iter().map(|key| key.to_bytes().to_vec()).collect(),
                    instructions: vec![CompiledInstruction {
                        accounts,
                        data: crate::instr::pump_amm::discriminators::BUY.to_vec(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            });
            let invokes = HashMap::from([(PUMPSWAP_PROGRAM, vec![(0, -1)])]);
            let mut event = DexEvent::PumpSwapBuy(PumpSwapBuyEvent {
                pool,
                ..Default::default()
            });
            fill_accounts_with_owned_keys(
                &mut event,
                &TransactionStatusMeta::default(),
                &transaction,
                &invokes,
            );
            let DexEvent::PumpSwapBuy(event) = event else {
                panic!("expected buy")
            };
            assert_eq!(event.fee_recipient, recipient);
            assert_eq!(event.fee_recipient_quote_token_account, Pubkey::default());
            assert_eq!(
                event.pool_v2,
                if count == 27 {
                    keys[24]
                } else {
                    Pubkey::default()
                }
            );
        }
    }

    struct RouteFixture {
        meta: TransactionStatusMeta,
        transaction: Option<Transaction>,
        invokes: HashMap<Pubkey, Vec<(i32, i32)>>,
        sell_pool: Pubkey,
        buy_pool: Pubkey,
        sell_mint: Pubkey,
        buy_mint: Pubkey,
    }

    /// A Jupiter-style token-to-token route: one outer pAMM sell invoke
    /// (24 accounts, pool/base_mint of leg A) followed by one outer pAMM buy
    /// invoke (26 accounts — the cashback variant is longer — pool/base_mint
    /// of leg B). Mirrors live tx DRCWs7iv… where the sell event's base_mint
    /// was backfilled from the buy leg.
    fn token_to_token_fixture() -> RouteFixture {
        let sell_pool = Pubkey::new_unique();
        let buy_pool = Pubkey::new_unique();
        let sell_mint = Pubkey::new_unique();
        let buy_mint = Pubkey::new_unique();
        let padding = Pubkey::new_unique();

        // static keys: [0]=sell_pool [1]=buy_pool [2]=sell_mint [3]=buy_mint
        // [4]=pumpswap program [5]=padding
        let static_keys: Vec<Vec<u8>> = [
            sell_pool,
            buy_pool,
            sell_mint,
            buy_mint,
            PUMPSWAP_PROGRAM,
            padding,
        ]
        .iter()
        .map(|k| k.to_bytes().to_vec())
        .collect();

        let mut sell_accounts = vec![5u8; 24];
        sell_accounts[0] = 0; // pool
        sell_accounts[3] = 2; // base_mint
        let mut buy_accounts = vec![5u8; 26];
        buy_accounts[0] = 1; // pool
        buy_accounts[3] = 3; // base_mint

        let transaction = Some(Transaction {
            signatures: vec![vec![0u8; 64]],
            message: Some(Message {
                header: Some(MessageHeader::default()),
                account_keys: static_keys,
                recent_blockhash: vec![0u8; 32],
                instructions: vec![
                    CompiledInstruction {
                        program_id_index: 4,
                        accounts: sell_accounts,
                        data: crate::instr::pump_amm::discriminators::SELL.to_vec(),
                    },
                    CompiledInstruction {
                        program_id_index: 4,
                        accounts: buy_accounts,
                        data: crate::instr::pump_amm::discriminators::BUY.to_vec(),
                    },
                ],
                versioned: false,
                address_table_lookups: Vec::new(),
                config: None,
            }),
        });
        let meta = TransactionStatusMeta::default();
        let mut invokes = HashMap::new();
        invokes.insert(PUMPSWAP_PROGRAM, vec![(0i32, -1i32), (1i32, -1i32)]);

        RouteFixture {
            meta,
            transaction,
            invokes,
            sell_pool,
            buy_pool,
            sell_mint,
            buy_mint,
        }
    }

    #[test]
    fn token_to_token_route_backfills_each_leg_from_its_own_invoke() {
        let f = token_to_token_fixture();

        let mut sell = DexEvent::PumpSwapSell(PumpSwapSellEvent {
            pool: f.sell_pool,
            ..Default::default()
        });
        fill_accounts_with_owned_keys(&mut sell, &f.meta, &f.transaction, &f.invokes);
        match sell {
            DexEvent::PumpSwapSell(e) => assert_eq!(
                e.base_mint, f.sell_mint,
                "sell event must backfill from the SELL leg, not the longer buy invoke"
            ),
            _ => unreachable!(),
        }

        let mut buy = DexEvent::PumpSwapBuy(PumpSwapBuyEvent {
            pool: f.buy_pool,
            ..Default::default()
        });
        fill_accounts_with_owned_keys(&mut buy, &f.meta, &f.transaction, &f.invokes);
        match buy {
            DexEvent::PumpSwapBuy(e) => assert_eq!(e.base_mint, f.buy_mint),
            _ => unreachable!(),
        }
    }

    #[test]
    fn anchored_lookup_is_order_independent() {
        let mut f = token_to_token_fixture();
        // Reverse invoke order: the buy leg now comes first.
        f.invokes.get_mut(&PUMPSWAP_PROGRAM).unwrap().reverse();

        let mut sell = DexEvent::PumpSwapSell(PumpSwapSellEvent {
            pool: f.sell_pool,
            ..Default::default()
        });
        fill_accounts_with_owned_keys(&mut sell, &f.meta, &f.transaction, &f.invokes);
        match sell {
            DexEvent::PumpSwapSell(e) => assert_eq!(e.base_mint, f.sell_mint),
            _ => unreachable!(),
        }
    }

    #[test]
    fn repeated_pumpswap_pool_matches_direction_user_and_rejects_ambiguity() {
        use yellowstone_grpc_proto::prelude::{InnerInstruction, InnerInstructions};
        for inner in [false, true] {
            let mut f = token_to_token_fixture();
            let message = f.transaction.as_mut().unwrap().message.as_mut().unwrap();
            message.instructions[1].accounts[0] = 0; // both directions now use the sell pool
            message.instructions[1].accounts[1] = 1; // buy user is a different known key
            let second_user = f.buy_pool;
            let mut duplicate = message.instructions[1].clone();
            duplicate.accounts[1] = 2; // another buy user
            message.instructions.push(duplicate);
            f.invokes
                .insert(PUMPSWAP_PROGRAM, vec![(2, -1), (1, -1), (0, -1)]);
            if inner {
                f.meta.inner_instructions = vec![InnerInstructions {
                    index: 0,
                    instructions: message
                        .instructions
                        .iter()
                        .map(|ix| InnerInstruction {
                            program_id_index: ix.program_id_index,
                            accounts: ix.accounts.clone(),
                            data: ix.data.clone(),
                            ..Default::default()
                        })
                        .collect(),
                }];
                message.instructions.clear();
                f.invokes
                    .insert(PUMPSWAP_PROGRAM, vec![(0, 2), (0, 1), (0, 0)]);
            }
            let mut buy = DexEvent::PumpSwapBuy(PumpSwapBuyEvent {
                pool: f.sell_pool,
                user: second_user,
                ..Default::default()
            });
            fill_accounts_with_owned_keys(&mut buy, &f.meta, &f.transaction, &f.invokes);
            let DexEvent::PumpSwapBuy(buy) = buy else {
                unreachable!()
            };
            assert_eq!(buy.base_mint, f.buy_mint);
            let mut sell = DexEvent::PumpSwapSell(PumpSwapSellEvent {
                pool: f.sell_pool,
                ..Default::default()
            });
            fill_accounts_with_owned_keys(&mut sell, &f.meta, &f.transaction, &f.invokes);
            let DexEvent::PumpSwapSell(sell) = sell else {
                unreachable!()
            };
            assert_eq!(sell.base_mint, f.sell_mint);
            let mut ambiguous = DexEvent::PumpSwapBuy(PumpSwapBuyEvent {
                pool: f.sell_pool,
                ..Default::default()
            });
            fill_accounts_with_owned_keys(&mut ambiguous, &f.meta, &f.transaction, &f.invokes);
            let DexEvent::PumpSwapBuy(ambiguous) = ambiguous else {
                unreachable!()
            };
            assert_eq!(ambiguous.base_mint, Pubkey::default());
            // Neither a different known user nor an unknown pool may select the first invoke.
            let mut unmatched = DexEvent::PumpSwapBuy(PumpSwapBuyEvent {
                pool: f.sell_pool,
                user: Pubkey::new_unique(),
                ..Default::default()
            });
            fill_accounts_with_owned_keys(&mut unmatched, &f.meta, &f.transaction, &f.invokes);
            let DexEvent::PumpSwapBuy(unmatched) = unmatched else {
                unreachable!()
            };
            assert_eq!(unmatched.base_mint, Pubkey::default());
            if inner {
                f.meta.inner_instructions[0].instructions[2].accounts[1] = 1;
            } else {
                f.transaction
                    .as_mut()
                    .unwrap()
                    .message
                    .as_mut()
                    .unwrap()
                    .instructions[2]
                    .accounts[1] = 1;
            }
            let mut same_user = DexEvent::PumpSwapBuy(PumpSwapBuyEvent {
                pool: f.sell_pool,
                user: second_user,
                ..Default::default()
            });
            fill_accounts_with_owned_keys(&mut same_user, &f.meta, &f.transaction, &f.invokes);
            let DexEvent::PumpSwapBuy(same_user) = same_user else {
                unreachable!()
            };
            assert_eq!(same_user.base_mint, Pubkey::default());
        }
    }

    #[test]
    fn unmatched_pumpswap_pool_keeps_missing_accounts_unknown() {
        let f = token_to_token_fixture();
        // An unmatched pool must not inherit another pool's account context.
        let mut sell = DexEvent::PumpSwapSell(PumpSwapSellEvent {
            pool: Pubkey::new_unique(),
            ..Default::default()
        });
        fill_accounts_with_owned_keys(&mut sell, &f.meta, &f.transaction, &f.invokes);
        match sell {
            DexEvent::PumpSwapSell(e) => assert_eq!(
                e.base_mint,
                Pubkey::default(),
                "an unmatched event must not inherit another pool's mint"
            ),
            _ => unreachable!(),
        }
    }

    #[test]
    fn clmm_log_backfill_uses_complete_instruction_account_count() {
        use crate::grpc::program_ids::RAYDIUM_CLMM_PROGRAM;
        let count = 79;
        let mut keys: Vec<_> = (0..=count).map(|_| Pubkey::new_unique()).collect();
        keys[10] = solana_sdk::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
        keys[count] = RAYDIUM_CLMM_PROGRAM;
        let bitmap = account_fillers::raydium::tick_array_bitmap_extension_pda(&keys[2]);
        keys[75] = bitmap;
        keys[25] = Pubkey::default();
        let transaction = Some(Transaction {
            message: Some(Message {
                account_keys: keys.iter().map(|key| key.to_bytes().to_vec()).collect(),
                instructions: vec![CompiledInstruction {
                    program_id_index: count as u32,
                    accounts: (0..count as u8).collect(),
                    data: crate::instr::raydium_clmm::discriminators::SWAP_V2.to_vec(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        });
        let invokes = HashMap::from([(RAYDIUM_CLMM_PROGRAM, vec![(0, -1)])]);
        let mut event = DexEvent::RaydiumClmmSwap(RaydiumClmmSwapEvent {
            pool_state: keys[2],
            sender: keys[0],
            token_account_0: keys[3],
            token_account_1: keys[4],
            tick_array_bitmap_extension: Some(bitmap),
            ..Default::default()
        });
        fill_accounts_with_owned_keys(
            &mut event,
            &TransactionStatusMeta::default(),
            &transaction,
            &invokes,
        );
        let DexEvent::RaydiumClmmSwap(event) = event else {
            panic!("swap")
        };
        assert_eq!(
            event.tick_arrays,
            keys[13..count]
                .iter()
                .copied()
                .filter(|key| *key != bitmap)
                .collect::<Vec<_>>()
        );
        assert_eq!(event.tick_array_bitmap_extension, Some(bitmap));
    }

    #[test]
    fn clmm_same_pool_v2_without_ticks_does_not_inherit_legacy_ticks() {
        use crate::grpc::program_ids::RAYDIUM_CLMM_PROGRAM;
        let mut keys: Vec<_> = (0..14).map(|_| Pubkey::new_unique()).collect();
        keys[9] = solana_sdk::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
        keys[13] = RAYDIUM_CLMM_PROGRAM;
        let transaction = Some(Transaction {
            message: Some(Message {
                account_keys: keys.iter().map(|key| key.to_bytes().to_vec()).collect(),
                instructions: vec![
                    CompiledInstruction {
                        program_id_index: 13,
                        accounts: vec![2, 1, 0, 3, 4, 5, 6, 7, 10, 8],
                        data: crate::instr::raydium_clmm::discriminators::SWAP.to_vec(),
                    },
                    CompiledInstruction {
                        program_id_index: 13,
                        accounts: vec![2, 1, 0, 3, 4, 5, 6, 7, 10, 10, 9, 11, 12],
                        data: crate::instr::raydium_clmm::discriminators::SWAP_V2.to_vec(),
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        });
        let invokes = HashMap::from([(RAYDIUM_CLMM_PROGRAM, vec![(0, -1), (1, -1)])]);
        let mut event = DexEvent::RaydiumClmmSwap(RaydiumClmmSwapEvent {
            pool_state: keys[0],
            sender: keys[2],
            token_account_0: keys[3],
            token_account_1: keys[4],
            input_mint: keys[11],
            output_mint: keys[12],
            ..Default::default()
        });
        fill_accounts_with_owned_keys(
            &mut event,
            &TransactionStatusMeta::default(),
            &transaction,
            &invokes,
        );
        let DexEvent::RaydiumClmmSwap(event) = event else {
            panic!("swap")
        };
        assert_eq!(event.amm_config, keys[1]);
        assert!(event.tick_arrays.is_empty());
        assert!(event.tick_array_bitmap_extension.is_none());
        let mut reversed = DexEvent::RaydiumClmmSwap(RaydiumClmmSwapEvent {
            pool_state: keys[0],
            sender: keys[2],
            token_account_0: keys[4],
            token_account_1: keys[3],
            input_mint: keys[11],
            output_mint: keys[12],
            ..Default::default()
        });
        fill_accounts_with_owned_keys(
            &mut reversed,
            &TransactionStatusMeta::default(),
            &transaction,
            &invokes,
        );
        let DexEvent::RaydiumClmmSwap(reversed) = reversed else {
            panic!("swap")
        };
        assert_eq!(reversed.amm_config, keys[1]);
        assert!(reversed.tick_arrays.is_empty());
    }

    #[test]
    fn launchlab_trade_backfills_from_matching_pool_invoke() {
        let first_pool = Pubkey::new_unique();
        let second_pool = Pubkey::new_unique();
        let first_quote_mint = Pubkey::new_unique();
        let second_quote_mint = Pubkey::new_unique();
        let padding = Pubkey::new_unique();
        let account_keys = [
            first_pool,
            second_pool,
            first_quote_mint,
            second_quote_mint,
            RAYDIUM_LAUNCHLAB_PROGRAM,
            padding,
        ]
        .iter()
        .map(|key| key.to_bytes().to_vec())
        .collect();
        let launchlab_accounts = |pool_index, quote_mint_index| {
            let mut accounts = vec![5u8; 15];
            accounts[4] = pool_index;
            accounts[10] = quote_mint_index;
            accounts[14] = 4;
            accounts
        };
        let transaction = Some(Transaction {
            signatures: vec![vec![0u8; 64]],
            message: Some(Message {
                header: Some(MessageHeader::default()),
                account_keys,
                recent_blockhash: vec![0u8; 32],
                instructions: vec![
                    CompiledInstruction {
                        program_id_index: 4,
                        accounts: launchlab_accounts(0, 2),
                        data: [
                            crate::instr::raydium_launchlab::discriminators::BUY_EXACT_IN
                                .as_slice(),
                            &[0; 16],
                        ]
                        .concat(),
                    },
                    CompiledInstruction {
                        program_id_index: 4,
                        accounts: launchlab_accounts(1, 3),
                        data: [
                            crate::instr::raydium_launchlab::discriminators::BUY_EXACT_IN
                                .as_slice(),
                            &[0; 16],
                        ]
                        .concat(),
                    },
                ],
                versioned: false,
                address_table_lookups: Vec::new(),
                config: None,
            }),
        });
        let meta = TransactionStatusMeta::default();
        let invokes = HashMap::from([(
            RAYDIUM_LAUNCHLAB_PROGRAM,
            vec![(0i32, -1i32), (1i32, -1i32)],
        )]);
        let mut event = DexEvent::RaydiumLaunchlabTrade(RaydiumLaunchlabTradeEvent {
            metadata: EventMetadata::default(),
            pool_state: first_pool,
            amount_in: 1,
            amount_out: 2,
            is_buy: true,
            exact_in: true,
            ..Default::default()
        });

        fill_accounts_with_owned_keys(&mut event, &meta, &transaction, &invokes);

        let DexEvent::RaydiumLaunchlabTrade(event) = event else {
            unreachable!();
        };
        assert_eq!(event.quote_mint, first_quote_mint);
        assert_ne!(event.quote_mint, second_quote_mint);
    }
}

#[cfg(test)]
mod pools_liquidity_dispatch_tests {
    use super::*;
    use crate::instr::meteora_amm::discriminators::*;
    use yellowstone_grpc_proto::prelude::{CompiledInstruction, Message};

    #[test]
    fn liquidity_dispatch_uses_discriminator_and_pool_with_ambiguous_lengths() {
        let keys: Vec<_> = (0..64).map(|_| Pubkey::new_unique()).collect();
        let instructions: Vec<_> = [
            (SWAP, 0u8, 16u8),
            (REMOVE_LIQUIDITY, 16, 16),
            (REMOVE_LIQUIDITY_SINGLE_SIDE, 32, 15),
            (BOOTSTRAP_LIQUIDITY, 48, 16),
        ]
        .into_iter()
        .map(|(disc, start, count)| CompiledInstruction {
            data: disc.to_vec(),
            accounts: (start..start + count).collect(),
            ..Default::default()
        })
        .collect();
        let transaction = Some(Transaction {
            message: Some(Message {
                account_keys: keys.iter().map(|k| k.to_bytes().to_vec()).collect(),
                instructions,
                ..Default::default()
            }),
            ..Default::default()
        });
        let invokes = HashMap::from([(
            crate::grpc::program_ids::METEORA_POOLS_PROGRAM,
            vec![(0, -1), (1, -1), (2, -1), (3, -1)],
        )]);
        let meta = TransactionStatusMeta::default();
        let mut event = DexEvent::MeteoraPoolsRemoveLiquidity(MeteoraPoolsRemoveLiquidityEvent {
            pool: keys[32],
            ..Default::default()
        });
        fill_accounts_with_owned_keys(&mut event, &meta, &transaction, &invokes);
        let DexEvent::MeteoraPoolsRemoveLiquidity(e) = event else {
            panic!("remove")
        };
        assert_eq!(e.ix_name, "remove_liquidity_single_side");
        assert_eq!(e.user_destination_token, keys[43]);
        assert_eq!(e.user, keys[44]);
        assert_eq!(e.vault_program, keys[45]);
        assert_eq!(e.token_program, keys[46]);
        assert_eq!(
            (e.user_a_token, e.user_b_token),
            (Pubkey::default(), Pubkey::default())
        );
        let mut event =
            DexEvent::MeteoraPoolsBootstrapLiquidity(MeteoraPoolsBootstrapLiquidityEvent {
                pool: keys[48],
                ..Default::default()
            });
        fill_accounts_with_owned_keys(&mut event, &meta, &transaction, &invokes);
        let DexEvent::MeteoraPoolsBootstrapLiquidity(e) = event else {
            panic!("bootstrap")
        };
        assert_eq!(e.ix_name, "bootstrap_liquidity");
        assert_eq!(e.user_a_token, keys[59]);
        assert_eq!(e.user_b_token, keys[60]);
        assert_eq!(e.user, keys[61]);
        assert_eq!(e.token_program, keys[63]);
        let invoke_list = [(0, -1), (1, -1), (2, -1), (3, -1)];
        assert!(find_pools_liquidity_invoke(
            &invoke_list,
            &meta,
            &transaction,
            keys[32],
            false,
            "remove_balance_liquidity"
        )
        .is_none());
        assert!(find_pools_liquidity_invoke(
            &invoke_list,
            &meta,
            &transaction,
            Pubkey::new_unique(),
            false,
            ""
        )
        .is_none());
        // Without a pool, type still excludes swaps and deposits.
        assert_eq!(
            find_pools_liquidity_invoke(
                &invoke_list,
                &meta,
                &transaction,
                Pubkey::default(),
                true,
                ""
            )
            .unwrap()
            .0,
            &(3, -1)
        );
    }
}

#[cfg(test)]
mod pools_management_dispatch_tests {
    use super::*;
    use crate::instr::meteora_amm::discriminators::*;
    use yellowstone_grpc_proto::prelude::{CompiledInstruction, Message};

    #[test]
    fn mixed_operations_dispatch_by_type_and_known_pool() {
        let keys: Vec<_> = (0..56).map(|_| Pubkey::new_unique()).collect();
        let instructions: Vec<_> = [
            (SWAP, 0u8, 26u8),
            (CREATE_POOL_WITH_CONFIG2, 26, 26),
            (SET_POOL_FEES, 52, 2),
            (SET_POOL_FEES, 54, 2),
        ]
        .into_iter()
        .map(|(d, start, count)| CompiledInstruction {
            data: d.to_vec(),
            accounts: (start..start + count).collect(),
            ..Default::default()
        })
        .collect();
        let transaction = Some(Transaction {
            message: Some(Message {
                account_keys: keys.iter().map(|k| k.to_bytes().to_vec()).collect(),
                instructions,
                ..Default::default()
            }),
            ..Default::default()
        });
        let invokes = HashMap::from([(
            crate::grpc::program_ids::METEORA_POOLS_PROGRAM,
            vec![(0, -1), (1, -1), (2, -1), (3, -1)],
        )]);
        let meta = TransactionStatusMeta::default();
        let mut event = DexEvent::MeteoraPoolsPoolCreated(MeteoraPoolsPoolCreatedEvent {
            pool: keys[26],
            ..Default::default()
        });
        fill_accounts_with_owned_keys(&mut event, &meta, &transaction, &invokes);
        let DexEvent::MeteoraPoolsPoolCreated(e) = event else {
            panic!("create")
        };
        assert_eq!(
            e.ix_name,
            "initialize_permissionless_constant_product_pool_with_config2"
        );
        assert_eq!(
            (e.config, e.lp_mint, e.token_a_mint, e.token_b_mint),
            (keys[27], keys[28], keys[29], keys[30])
        );
        assert_eq!(
            (e.payer, e.token_program, e.system_program),
            (keys[44], keys[49], keys[51])
        );
        let mut event = DexEvent::MeteoraPoolsSetPoolFees(MeteoraPoolsSetPoolFeesEvent {
            pool: keys[54],
            ..Default::default()
        });
        fill_accounts_with_owned_keys(&mut event, &meta, &transaction, &invokes);
        let DexEvent::MeteoraPoolsSetPoolFees(e) = event else {
            panic!("fees")
        };
        assert_eq!(e.ix_name, "set_pool_fees");
        assert_eq!(e.fee_operator, keys[55]);
        let invoke_list = [(0, -1), (1, -1), (2, -1), (3, -1)];
        assert!(find_pools_management_invoke(
            &invoke_list,
            &meta,
            &transaction,
            keys[54],
            true,
            ""
        )
        .is_none());
        assert!(find_pools_management_invoke(
            &invoke_list,
            &meta,
            &transaction,
            keys[26],
            true,
            "initialize_permissionless_constant_product_pool_with_config"
        )
        .is_none());
    }
}

#[cfg(test)]
mod remaining_creation_dispatch_tests {
    use super::*;
    use crate::instr::meteora_amm::discriminators::*;
    use yellowstone_grpc_proto::prelude::{CompiledInstruction, Message};
    #[test]
    fn every_creation_layout_is_selected_from_its_own_instruction() {
        for (disc, name, count, lp_index, payer_index, token_index) in [
            (
                INITIALIZE_PERMISSIONED_POOL,
                "initialize_permissioned_pool",
                24u8,
                1,
                15,
                21,
            ),
            (
                INITIALIZE_PERMISSIONLESS_POOL,
                "initialize_permissionless_pool",
                26,
                1,
                17,
                23,
            ),
            (
                INITIALIZE_PERMISSIONLESS_POOL_WITH_FEE_TIER,
                "initialize_permissionless_pool_with_fee_tier",
                26,
                1,
                17,
                23,
            ),
            (
                INITIALIZE_CUSTOMIZABLE_POOL,
                "initialize_customizable_permissionless_constant_product_pool",
                25,
                1,
                17,
                22,
            ),
        ] {
            let keys: Vec<_> = (0..count + 27).map(|_| Pubkey::new_unique()).collect();
            let transaction = Some(Transaction {
                message: Some(Message {
                    account_keys: keys.iter().map(|k| k.to_bytes().to_vec()).collect(),
                    instructions: vec![
                        CompiledInstruction {
                            data: CREATE_POOL.to_vec(),
                            accounts: (0..26).collect(),
                            ..Default::default()
                        },
                        CompiledInstruction {
                            data: disc.to_vec(),
                            accounts: (27..27 + count).collect(),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            });
            let invokes = HashMap::from([(
                crate::grpc::program_ids::METEORA_POOLS_PROGRAM,
                vec![(0, -1), (1, -1)],
            )]);
            let mut event = DexEvent::MeteoraPoolsPoolCreated(MeteoraPoolsPoolCreatedEvent {
                pool: keys[27],
                ..Default::default()
            });
            fill_accounts_with_owned_keys(
                &mut event,
                &TransactionStatusMeta::default(),
                &transaction,
                &invokes,
            );
            let DexEvent::MeteoraPoolsPoolCreated(e) = event else {
                panic!("create")
            };
            assert_eq!(e.ix_name, name);
            assert_eq!(e.lp_mint, keys[27 + lp_index]);
            assert_eq!(e.token_program, keys[27 + token_index]);
            if name == "initialize_permissioned_pool" {
                assert_eq!(e.admin, keys[27 + payer_index]);
                assert_eq!(e.payer, Pubkey::default());
            } else {
                assert_eq!(e.payer, keys[27 + payer_index]);
                assert_eq!(e.admin, Pubkey::default());
            }
            assert_eq!(e.config, Pubkey::default());
        }
    }
}

#[cfg(test)]
mod review_matching_regressions {
    use super::*;
    use yellowstone_grpc_proto::prelude::{CompiledInstruction, Message};

    #[test]
    fn known_pool_never_falls_back_to_an_unrelated_or_ambiguous_invocation() {
        let pool = Pubkey::new_unique();
        let foreign = Pubkey::new_unique();
        let tx = Some(Transaction {
            message: Some(Message {
                account_keys: vec![pool.to_bytes().to_vec(), foreign.to_bytes().to_vec()],
                instructions: vec![
                    CompiledInstruction {
                        accounts: vec![1],
                        ..Default::default()
                    },
                    CompiledInstruction {
                        accounts: vec![0],
                        ..Default::default()
                    },
                    CompiledInstruction {
                        accounts: vec![0],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        });
        let meta = TransactionStatusMeta::default();
        let keys = tx
            .as_ref()
            .unwrap()
            .message
            .as_ref()
            .map(|m| &m.account_keys);
        let _ = keys;
        let select = |invokes| find_context_invoke(invokes, &meta, &tx, |_, get, _| get(0) == pool);
        assert!(select(&[(0, -1)]).is_none());
        assert_eq!(select(&[(0, -1), (1, -1)]), Some(&(1, -1)));
        assert!(select(&[(1, -1), (2, -1)]).is_none());
    }

    #[test]
    fn repeated_clmm_swaps_require_unique_context_and_the_right_discriminator() {
        let keys: Vec<_> = (0..13).map(|_| Pubkey::new_unique()).collect();
        let swap = CompiledInstruction {
            accounts: (0..13).collect(),
            data: crate::instr::raydium_clmm::discriminators::SWAP_V2.to_vec(),
            ..Default::default()
        };
        let tx = Some(Transaction {
            message: Some(Message {
                account_keys: keys.iter().map(|k| k.to_bytes().to_vec()).collect(),
                instructions: vec![swap.clone(), swap],
                ..Default::default()
            }),
            ..Default::default()
        });
        let meta = TransactionStatusMeta::default();
        let mut event = RaydiumClmmSwapEvent {
            pool_state: keys[2],
            sender: keys[0],
            ..Default::default()
        };
        assert!(find_clmm_swap_invoke(&[(0, -1), (1, -1)], &meta, &tx, &event).is_none());
        assert_eq!(
            find_clmm_swap_invoke(&[(0, -1)], &meta, &tx, &event),
            Some(&(0, -1))
        );
        event.ix_name = "swap".into();
        assert!(find_clmm_swap_invoke(&[(0, -1)], &meta, &tx, &event).is_none());
    }
}

#[cfg(test)]
mod review_dlmm_context_regressions {
    use super::*;
    use yellowstone_grpc_proto::prelude::{CompiledInstruction, Message};
    #[test]
    fn dlmm_hook_boundaries_are_filled_only_from_unique_matching_swap() {
        let keys: Vec<_> = (0..41).map(|_| Pubkey::new_unique()).collect();
        let accounts_a: Vec<u8> = (1..21).collect();
        let mut accounts_b: Vec<u8> = (21..41).collect();
        accounts_b[0] = accounts_a[0];
        let mut data = crate::instr::meteora_dlmm::discriminators::SWAP2.to_vec();
        data.extend_from_slice(&[0; 16]);
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&[0, 2]);
        let transaction = Some(Transaction {
            message: Some(Message {
                account_keys: keys.iter().map(|k| k.to_bytes().to_vec()).collect(),
                instructions: vec![
                    CompiledInstruction {
                        accounts: accounts_a.clone(),
                        data: data.clone(),
                        ..Default::default()
                    },
                    CompiledInstruction {
                        accounts: accounts_b,
                        data,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }),
            ..Default::default()
        });
        let meta = TransactionStatusMeta::default();
        let mut event = MeteoraDlmmSwapEvent {
            pool: keys[1],
            ..Default::default()
        };
        fill_dlmm_swap_event(&mut event, &meta, &transaction, &[(0, -1), (1, -1)]);
        assert!(event.bin_arrays.is_empty());
        assert_eq!(event.user_token_in, Pubkey::default());
        event.from = keys[11];
        fill_dlmm_swap_event(&mut event, &meta, &transaction, &[(0, -1), (1, -1)]);
        assert_eq!(event.bin_arrays, vec![keys[19], keys[20]]);
        assert_eq!(event.user_token_in, keys[5]);
        // A non-swap invoke sharing the pool cannot supply account context.
        let mut invalid = transaction.clone();
        invalid
            .as_mut()
            .unwrap()
            .message
            .as_mut()
            .unwrap()
            .instructions[0]
            .data
            .truncate(24);
        let mut event = MeteoraDlmmSwapEvent {
            pool: keys[1],
            from: keys[11],
            ..Default::default()
        };
        fill_dlmm_swap_event(&mut event, &meta, &invalid, &[(0, -1)]);
        assert!(event.bin_arrays.is_empty());
    }
}

#[cfg(test)]
mod review_generic_identity_tests {
    use super::*;
    use yellowstone_grpc_proto::prelude::{CompiledInstruction, Message};

    #[test]
    fn cpmm_deposit_cannot_inherit_another_pool_swap_payer() {
        use crate::instr::raydium_cpmm::discriminators::{DEPOSIT, SWAP_BASE_IN};
        let keys: Vec<_> = (0..35).map(|_| Pubkey::new_unique()).collect();
        let mut deposit_data = DEPOSIT.to_vec();
        deposit_data.extend_from_slice(&[0; 24]);
        let mut swap_data = SWAP_BASE_IN.to_vec();
        swap_data.extend_from_slice(&[0; 16]);
        let deposit = CompiledInstruction {
            accounts: (0..13).collect(),
            data: deposit_data,
            ..Default::default()
        };
        let swap = CompiledInstruction {
            accounts: (15..35).collect(),
            data: swap_data,
            ..Default::default()
        };
        let tx = Some(Transaction {
            message: Some(Message {
                account_keys: keys.iter().map(|k| k.to_bytes().to_vec()).collect(),
                instructions: vec![deposit, swap],
                ..Default::default()
            }),
            ..Default::default()
        });
        let mut event = DexEvent::RaydiumCpmmDeposit(RaydiumCpmmDepositEvent {
            metadata: EventMetadata::default(),
            pool: keys[2],
            user: Pubkey::default(),
            token0_amount: 4,
            token1_amount: 5,
            lp_token_amount: 6,
        });
        let invokes = HashMap::from([(
            crate::grpc::program_ids::RAYDIUM_CPMM_PROGRAM,
            vec![(0, -1), (1, -1)],
        )]);
        fill_accounts_with_owned_keys(&mut event, &TransactionStatusMeta::default(), &tx, &invokes);
        let DexEvent::RaydiumCpmmDeposit(event) = event else {
            panic!()
        };
        assert_eq!(
            event.user, keys[0],
            "deposit log must keep the matching instruction's owner"
        );
    }
    fn owner(event: &mut DexEvent) -> &mut Pubkey {
        match event {
            DexEvent::RaydiumCpmmDeposit(e) => &mut e.user,
            DexEvent::RaydiumCpmmWithdraw(e) => &mut e.user,
            DexEvent::RaydiumCpmmInitialize(e) => &mut e.creator,
            DexEvent::RaydiumAmmV4Deposit(e) => &mut e.user_owner,
            DexEvent::RaydiumAmmV4Withdraw(e) => &mut e.user_owner,
            _ => unreachable!(),
        }
    }

    fn missing_context(event: &mut DexEvent) -> &mut Pubkey {
        match event {
            DexEvent::RaydiumAmmV4Deposit(e) => &mut e.amm_authority,
            DexEvent::RaydiumAmmV4Withdraw(e) => &mut e.amm_authority,
            _ => owner(event),
        }
    }

    #[test]
    fn liquidity_context_matches_opcode_identity_and_rejects_ambiguity_outer_and_cpi() {
        use crate::{
            grpc::program_ids::*,
            instr::{raydium_amm as amm, raydium_cpmm as cpmm},
        };
        use solana_sdk::signature::Signature;
        use yellowstone_grpc_proto::prelude::{InnerInstruction, InnerInstructions};
        let cases = [
            (true, cpmm::discriminators::DEPOSIT.to_vec(), 13, 2, 0, 24),
            (true, cpmm::discriminators::WITHDRAW.to_vec(), 14, 2, 0, 24),
            (
                true,
                cpmm::discriminators::INITIALIZE.to_vec(),
                20,
                3,
                0,
                24,
            ),
            (false, vec![amm::discriminators::DEPOSIT], 14, 1, 12, 24),
            (false, vec![amm::discriminators::WITHDRAW], 22, 1, 18, 8),
        ];
        for (is_cpmm, mut data, count, pool_index, owner_index, payload_len) in cases {
            data.extend((0..payload_len).map(|i| (i + 1) as u8));
            let keys: Vec<_> = (0..80).map(|_| Pubkey::new_unique()).collect();
            let parsed = if is_cpmm {
                cpmm::parse_instruction(&data, &keys[..count], Signature::default(), 1, 0, None)
            } else {
                amm::parse_instruction(&data, &keys[..count], Signature::default(), 1, 0, None)
            }
            .unwrap();
            for inner in [false, true] {
                for scenario in 0..4 {
                    let mut event = parsed.clone();
                    *missing_context(&mut event) = Pubkey::default();
                    // Same opcode on another pool, longer than the target.
                    let target = CompiledInstruction {
                        accounts: (0..count as u8).collect(),
                        data: data.clone(),
                        ..Default::default()
                    };
                    let other = CompiledInstruction {
                        accounts: (40..(40 + count + 2) as u8).collect(),
                        data: data.clone(),
                        ..Default::default()
                    };
                    let event_cpi = CompiledInstruction {
                        accounts: (0..(count + 3) as u8).collect(),
                        data: vec![228, 69, 165, 46, 81, 203, 154, 29],
                        ..Default::default()
                    };
                    let mut instructions = vec![target.clone(), other, event_cpi];
                    match scenario {
                        1 => instructions.push(target.clone()), // duplicate matching identity: ambiguous
                        2 => {
                            *owner(&mut event) = keys[70];
                        } // known user conflict
                        3 => {
                            instructions[0].accounts[pool_index] = 71;
                        } // no matching pool
                        _ => {}
                    }
                    let before = serde_json::to_value(&event).unwrap();
                    let invokes: Vec<_> = (0..instructions.len())
                        .map(|i| if inner { (0, i as i32) } else { (i as i32, -1) })
                        .collect();
                    let mut meta = TransactionStatusMeta::default();
                    if inner {
                        meta.inner_instructions.push(InnerInstructions {
                            index: 0,
                            instructions: instructions
                                .iter()
                                .map(|ix| InnerInstruction {
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
                    let program = if is_cpmm {
                        RAYDIUM_CPMM_PROGRAM
                    } else {
                        RAYDIUM_AMM_V4_PROGRAM
                    };
                    fill_accounts_with_owned_keys(
                        &mut event,
                        &meta,
                        &tx,
                        &HashMap::from([(program, invokes)]),
                    );
                    if scenario == 0 {
                        assert_eq!(
                            *missing_context(&mut event),
                            keys[if is_cpmm { owner_index } else { 2 }]
                        );
                        assert_eq!(
                            serde_json::to_value(&event).unwrap(),
                            serde_json::to_value(&parsed).unwrap(),
                            "account enrichment must preserve amounts"
                        );
                    } else {
                        assert_eq!(
                            serde_json::to_value(&event).unwrap(),
                            before,
                            "ambiguous or conflicting context must remain unchanged"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod followup_generic_context_tests {
    use super::*;
    use solana_sdk::signature::Signature;
    use yellowstone_grpc_proto::prelude::{
        CompiledInstruction, InnerInstruction, InnerInstructions, Message,
    };

    fn missing_field(event: &mut DexEvent) -> &mut Pubkey {
        match event {
            DexEvent::RaydiumAmmV4Swap(e) => &mut e.amm,
            DexEvent::PumpSwapCreatePool(e) => &mut e.creator,
            DexEvent::RaydiumLaunchlabTrade(e) => &mut e.quote_mint,
            DexEvent::RaydiumLaunchlabPoolCreate(e) => &mut e.base_mint,
            _ => unreachable!(),
        }
    }

    #[test]
    fn generic_mutating_branches_do_not_attribute_other_operations_or_users() {
        use crate::instr::{pump_amm as pump, raydium_amm as amm, raydium_launchlab as launch};
        let keys: Vec<_> = (0..100).map(|_| Pubkey::new_unique()).collect();
        let mut create_data = pump::discriminators::CREATE_POOL.to_vec();
        create_data.extend_from_slice(&[0; 50]);
        let mut cases = vec![(
            1,
            create_data,
            18,
            crate::grpc::program_ids::PUMPSWAP_PROGRAM,
        )];
        for (tag, count) in [
            (amm::discriminators::SWAP_BASE_IN, 17),
            (amm::discriminators::SWAP_BASE_IN, 18),
            (amm::discriminators::SWAP_BASE_OUT, 17),
            (amm::discriminators::SWAP_BASE_OUT, 18),
            (amm::discriminators::SWAP_BASE_IN_V2, 8),
            (amm::discriminators::SWAP_BASE_OUT_V2, 8),
        ] {
            let mut data = vec![tag];
            data.extend_from_slice(&[0; 16]);
            cases.push((
                0,
                data,
                count,
                crate::grpc::program_ids::RAYDIUM_AMM_V4_PROGRAM,
            ));
        }
        for disc in [
            launch::discriminators::BUY_EXACT_IN,
            launch::discriminators::BUY_EXACT_OUT,
            launch::discriminators::SELL_EXACT_IN,
            launch::discriminators::SELL_EXACT_OUT,
        ] {
            let mut data = disc.to_vec();
            data.extend_from_slice(&[0; 16]);
            cases.push((
                2,
                data,
                18,
                crate::grpc::program_ids::RAYDIUM_LAUNCHLAB_PROGRAM,
            ));
        }
        for (disc, count) in [
            (launch::discriminators::INITIALIZE, 18),
            (launch::discriminators::INITIALIZE_V2, 18),
            (launch::discriminators::INITIALIZE_WITH_TOKEN_2022, 15),
        ] {
            let mut data = disc.to_vec();
            data.extend_from_slice(&[0; 13]);
            cases.push((
                3,
                data,
                count,
                crate::grpc::program_ids::RAYDIUM_LAUNCHLAB_PROGRAM,
            ));
        }
        for (case, data, count, program) in cases {
            let parsed = match case {
                0 => {
                    amm::parse_instruction(&data, &keys[..count], Signature::default(), 1, 0, None)
                }
                1 => {
                    pump::parse_instruction(&data, &keys[..count], Signature::default(), 1, 0, None)
                }
                _ => launch::parse_instruction(
                    &data,
                    &keys[..count],
                    Signature::default(),
                    1,
                    0,
                    None,
                ),
            }
            .unwrap();
            for inner in [false, true] {
                for scenario in 0..5 {
                    let mut event = parsed.clone();
                    *missing_field(&mut event) = Pubkey::default();
                    let target = CompiledInstruction {
                        accounts: (0..count as u8).collect(),
                        data: data.clone(),
                        ..Default::default()
                    };
                    let mut foreign = CompiledInstruction {
                        accounts: (40..(40 + count + 3) as u8).collect(),
                        data: data.clone(),
                        ..Default::default()
                    };
                    if case >= 2 {
                        foreign.accounts[if case == 2 { 4 } else { 5 }] =
                            if case == 2 { 4 } else { 5 };
                    }
                    let mut instructions = vec![target.clone(), foreign];
                    match scenario {
                        1 => instructions.push(target.clone()),
                        2 => {
                            instructions.remove(0);
                        } // only other pool / same pool another user
                        3 => {
                            instructions[0].data = vec![228, 69, 165, 46, 81, 203, 154, 29];
                        } // eventCPI is not an operation
                        4 => {
                            instructions[0].data.truncate(8);
                        }
                        _ => {}
                    }
                    let before = serde_json::to_value(&event).unwrap();
                    let invokes: Vec<_> = (0..instructions.len())
                        .map(|i| if inner { (0, i as i32) } else { (i as i32, -1) })
                        .collect();
                    let mut meta = TransactionStatusMeta::default();
                    if inner {
                        meta.inner_instructions.push(InnerInstructions {
                            index: 0,
                            instructions: instructions
                                .iter()
                                .map(|ix| InnerInstruction {
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
                    fill_accounts_with_owned_keys(
                        &mut event,
                        &meta,
                        &tx,
                        &HashMap::from([(program, invokes)]),
                    );
                    let expected = if scenario == 0 {
                        serde_json::to_value(&parsed).unwrap()
                    } else {
                        before
                    };
                    assert_eq!(
                        serde_json::to_value(&event).unwrap(),
                        expected,
                        "case {case}, inner {inner}, scenario {scenario}"
                    );
                }
            }
        }
    }
}
