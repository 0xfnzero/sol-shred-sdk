//! ShredStream 热路径：DEX **外层**指令解析（无 inner CPI）。
//!
//! - 与 `client.rs` 解耦，便于维护与 `#[inline]` 边界优化。
//! - 避免每笔交易克隆整张 `static_account_keys`、避免 `Vec<IxRef>` 指令副本。
//! - Pump.fun 使用专用外层热路径；其它已支持 DEX 协议走统一指令解析入口。

use crate::instr::pump::create_layout::{
    create as create_accounts, create_v2 as create_v2_accounts,
};
use smallvec::SmallVec;
use solana_sdk::message::VersionedMessage;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::VersionedTransaction;

use crate::accounts::program_ids::SPL_TOKEN_2022_PROGRAM_ID;
use crate::core::events::{
    normalize_pumpfun_quote_mint, DexEvent, EventMetadata, PumpFunCreateTokenEvent,
    PumpFunMigrateBondingCurveCreatorEvent, PumpFunTradeEvent, PUMPFUN_SOLSCAN_SOL_QUOTE_MINT,
};
use crate::grpc::types::EventTypeFilter;
use crate::instr::program_ids::{
    METEORA_DAMM_V2_PROGRAM_ID, METEORA_DLMM_PROGRAM_ID, METEORA_POOLS_PROGRAM_ID,
    ORCA_WHIRLPOOL_PROGRAM_ID, PUMPSWAP_PROGRAM_ID, PUMP_FEES_PROGRAM_ID,
    RAYDIUM_AMM_V4_PROGRAM_ID, RAYDIUM_CLMM_PROGRAM_ID, RAYDIUM_CPMM_PROGRAM_ID,
    RAYDIUM_LAUNCHLAB_PROGRAM_ID,
};
use crate::instr::pump::discriminators;
use crate::instr::pump::PROGRAM_ID_PUBKEY;
use crate::instr::utils::{read_option_bool_idl, read_pubkey, read_str_unchecked, read_u64_le};

type PumpMintSet = SmallVec<[Pubkey; 4]>;
type ShredIxAccounts = SmallVec<[Pubkey; 64]>;

#[inline(always)]
fn account_from_static_or_default(static_keys: &[Pubkey], idx: u8) -> Pubkey {
    static_keys.get(idx as usize).copied().unwrap_or_default()
}

#[inline(always)]
fn program_id_references_loaded_key(program_id_index: u8, static_key_len: usize) -> bool {
    program_id_index as usize >= static_key_len
}

#[inline(always)]
fn supported_unified_outer_programs() -> &'static [Pubkey] {
    &[
        PUMPSWAP_PROGRAM_ID,
        PUMP_FEES_PROGRAM_ID,
        RAYDIUM_LAUNCHLAB_PROGRAM_ID,
        RAYDIUM_CPMM_PROGRAM_ID,
        RAYDIUM_CLMM_PROGRAM_ID,
        RAYDIUM_AMM_V4_PROGRAM_ID,
        ORCA_WHIRLPOOL_PROGRAM_ID,
        METEORA_POOLS_PROGRAM_ID,
        METEORA_DAMM_V2_PROGRAM_ID,
        METEORA_DLMM_PROGRAM_ID,
    ]
}

#[inline(always)]
fn build_shred_ix_accounts(static_keys: &[Pubkey], ix_accounts: &[u8]) -> ShredIxAccounts {
    let mut accounts = SmallVec::with_capacity(ix_accounts.len());
    for &idx in ix_accounts {
        accounts.push(account_from_static_or_default(static_keys, idx));
    }
    accounts
}

#[inline(always)]
fn instruction_keys_resolved(program_id_index: u8, accounts: &[u8], keys_len: usize) -> bool {
    (program_id_index as usize) < keys_len
        && accounts.iter().all(|&index| (index as usize) < keys_len)
}

#[inline(always)]
fn disc8(data: &[u8]) -> Option<[u8; 8]> {
    data.get(..8)?.try_into().ok()
}

#[inline(always)]
fn pumpfun_outer_data_may_parse(data: &[u8]) -> bool {
    let Some(disc) = disc8(data) else {
        return false;
    };
    matches!(
        disc,
        discriminators::CREATE
            | discriminators::CREATE_V2
            | discriminators::BUY
            | discriminators::SELL
            | discriminators::BUY_EXACT_SOL_IN
            | discriminators::BUY_V3
            | discriminators::SELL_V3
            | discriminators::BUY_EXACT_QUOTE_IN_V3
            | discriminators::BUY_V2
            | discriminators::BUY_EXACT_QUOTE_IN_V2
            | discriminators::SELL_V2
            | discriminators::MIGRATE_BONDING_CURVE_CREATOR
    )
}

#[inline(always)]
fn unified_outer_data_may_parse(program_id: Pubkey, data: &[u8]) -> bool {
    crate::instr::instruction_data_may_parse(&program_id, data)
}

#[inline(always)]
fn unknown_outer_data_may_parse(data: &[u8], filter: Option<&EventTypeFilter>) -> bool {
    if filter.is_none_or(|f| f.includes_pumpfun()) && pumpfun_outer_data_may_parse(data) {
        return true;
    }
    supported_unified_outer_programs().iter().any(|program_id| {
        is_supported_unified_outer_program(program_id, filter)
            && unified_outer_data_may_parse(*program_id, data)
    })
}

#[inline(always)]
fn push_unique_mint(mints: &mut PumpMintSet, mint: Pubkey) {
    if !mints.contains(&mint) {
        mints.push(mint);
    }
}

#[inline(always)]
fn token_program_or_default(token_program: Pubkey) -> Pubkey {
    if token_program == Pubkey::default() {
        SPL_TOKEN_2022_PROGRAM_ID
    } else {
        token_program
    }
}

#[inline(always)]
fn quote_mint_from_shred_v2_account(quote_mint: Option<Pubkey>) -> Pubkey {
    normalize_pumpfun_quote_mint(quote_mint.unwrap_or_default())
}

#[inline]
fn scan_create_mint_from_ix(
    program_id_index: u8,
    ix_accounts: &[u8],
    data: &[u8],
    static_keys: &[Pubkey],
    created_mints: &mut PumpMintSet,
    mayhem_mints: &mut PumpMintSet,
) {
    if data.len() < 8 {
        return;
    }
    if program_id_references_loaded_key(program_id_index, static_keys.len()) {
        scan_create_mint_from_unknown_program_ix(
            ix_accounts,
            data,
            static_keys,
            created_mints,
            mayhem_mints,
        );
        return;
    }
    let Some(program_id) = static_keys.get(program_id_index as usize) else {
        return;
    };
    if *program_id != PROGRAM_ID_PUBKEY {
        return;
    }
    let disc: [u8; 8] = data[0..8].try_into().unwrap_or_default();
    if disc != discriminators::CREATE && disc != discriminators::CREATE_V2 {
        return;
    }
    let Some(&mint_idx) = ix_accounts.first() else {
        return;
    };
    let Some(&mint) = static_keys.get(mint_idx as usize) else {
        return;
    };
    push_unique_mint(created_mints, mint);
    if disc == discriminators::CREATE_V2 {
        let is_mayhem = crate::instr::utils::parse_create_v2_tail_fields(&data[8..])
            .map(|(_, m, _, _, _)| m)
            .unwrap_or(false);
        if is_mayhem {
            push_unique_mint(mayhem_mints, mint);
        }
    }
}

#[inline]
fn scan_create_mint_from_unknown_program_ix(
    ix_accounts: &[u8],
    data: &[u8],
    static_keys: &[Pubkey],
    created_mints: &mut PumpMintSet,
    mayhem_mints: &mut PumpMintSet,
) {
    if data.len() < 8 {
        return;
    }
    let disc: [u8; 8] = data[0..8].try_into().unwrap_or_default();
    if disc != discriminators::CREATE && disc != discriminators::CREATE_V2 {
        return;
    }
    let Some(&mint_idx) = ix_accounts.first() else {
        return;
    };
    let Some(&mint) = static_keys.get(mint_idx as usize) else {
        return;
    };
    push_unique_mint(created_mints, mint);
    if disc == discriminators::CREATE_V2 {
        let is_mayhem = crate::instr::utils::parse_create_v2_tail_fields(&data[8..])
            .map(|(_, m, _, _, _)| m)
            .unwrap_or(false);
        if is_mayhem {
            push_unique_mint(mayhem_mints, mint);
        }
    }
}

/// 第一遍：收集本笔交易内 Pump Create/CreateV2 的 mint（**零指令副本**，直接引用 message 内 `CompiledInstruction`）。
#[inline]
fn detect_pumpfun_create_mints(
    message: &VersionedMessage,
    static_keys: &[Pubkey],
    best_effort: bool,
) -> (PumpMintSet, PumpMintSet) {
    let mut created_mints = PumpMintSet::new();
    let mut mayhem_mints = PumpMintSet::new();
    for ix in message.instructions() {
        if !best_effort && program_id_references_loaded_key(ix.program_id_index, static_keys.len())
        {
            continue;
        }
        scan_create_mint_from_ix(
            ix.program_id_index,
            &ix.accounts,
            &ix.data,
            static_keys,
            &mut created_mints,
            &mut mayhem_mints,
        );
    }
    (created_mints, mayhem_mints)
}

/// DEX 外层指令解析，保持与交易内 ix 顺序一致。
#[inline]
fn dispatch_shred_outer(
    program_id_index: u8,
    ix_accounts: &[u8],
    data: &[u8],
    static_keys: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    filter: Option<&EventTypeFilter>,
    created_mints: &PumpMintSet,
    mayhem_mints: &PumpMintSet,
    events: &mut Vec<DexEvent>,
) {
    if data.is_empty() {
        return;
    }
    if program_id_references_loaded_key(program_id_index, static_keys.len()) {
        log::trace!(
            target: "sol_parser_sdk::shredstream",
            "program_id_index references an ALT-loaded account; trying discriminator-only best-effort parse"
        );
        parse_unknown_program_outer(
            ix_accounts,
            data,
            static_keys,
            signature,
            slot,
            tx_index,
            recv_us,
            filter,
            created_mints,
            mayhem_mints,
            events,
        );
        return;
    }
    let Some(program_id) = static_keys.get(program_id_index as usize) else {
        return;
    };
    if *program_id == PROGRAM_ID_PUBKEY {
        if filter.is_some_and(|f| !f.includes_pumpfun()) || !pumpfun_outer_data_may_parse(data) {
            return;
        }
        let accounts = build_shred_ix_accounts(static_keys, ix_accounts);
        if let Some(ev) = parse_pumpfun_instruction(
            data,
            &accounts,
            signature,
            slot,
            tx_index,
            recv_us,
            created_mints,
            mayhem_mints,
        ) {
            push_filtered_shred_event(events, ev, filter);
        }
        return;
    }
    if !is_supported_unified_outer_program(program_id, filter) {
        return;
    }
    if !unified_outer_data_may_parse(*program_id, data) {
        return;
    }
    let accounts = build_shred_ix_accounts(static_keys, ix_accounts);
    if let Some(ev) = parse_non_pump_dex_outer(
        *program_id,
        data,
        &accounts,
        signature,
        slot,
        tx_index,
        recv_us,
        filter,
    ) {
        push_filtered_shred_event(events, ev, filter);
    }
}

#[inline]
fn push_filtered_shred_event(
    events: &mut Vec<DexEvent>,
    event: DexEvent,
    filter: Option<&EventTypeFilter>,
) -> bool {
    let Some(filter) = filter else {
        events.push(event);
        return true;
    };
    if filter.should_include_dex_event(&event) {
        events.push(filter.normalize_dex_event(event));
        true
    } else {
        false
    }
}

#[inline]
fn parse_unknown_program_outer(
    ix_accounts: &[u8],
    data: &[u8],
    static_keys: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    filter: Option<&EventTypeFilter>,
    created_mints: &PumpMintSet,
    mayhem_mints: &PumpMintSet,
    events: &mut Vec<DexEvent>,
) {
    if !unknown_outer_data_may_parse(data, filter) {
        return;
    }
    let accounts = build_shred_ix_accounts(static_keys, ix_accounts);

    if filter.is_none_or(|f| f.includes_pumpfun()) && pumpfun_outer_data_may_parse(data) {
        if let Some(ev) = parse_pumpfun_instruction(
            data,
            &accounts,
            signature,
            slot,
            tx_index,
            recv_us,
            created_mints,
            mayhem_mints,
        ) {
            if push_filtered_shred_event(events, ev, filter) {
                log::trace!(
                    target: "sol_parser_sdk::shredstream",
                    "unknown-program shred ix parsed as first matching candidate; use a narrow filter to avoid discriminator collisions"
                );
                return;
            }
        }
    }

    for &program_id in supported_unified_outer_programs() {
        if !is_supported_unified_outer_program(&program_id, filter) {
            continue;
        }
        if let Some(ev) = parse_non_pump_dex_outer(
            program_id, data, &accounts, signature, slot, tx_index, recv_us, filter,
        ) {
            push_filtered_shred_event(events, ev, filter);
            log::trace!(
                target: "sol_parser_sdk::shredstream",
                "unknown-program shred ix parsed as first matching candidate; use a narrow filter to avoid discriminator collisions"
            );
            return;
        }
    }
}

#[inline(always)]
fn is_supported_unified_outer_program(
    program_id: &Pubkey,
    filter: Option<&EventTypeFilter>,
) -> bool {
    let Some(f) = filter else {
        return supported_unified_outer_programs().contains(program_id);
    };

    if *program_id == PUMPSWAP_PROGRAM_ID {
        f.includes_pumpswap()
    } else if *program_id == PUMP_FEES_PROGRAM_ID {
        f.includes_pump_fees()
    } else if *program_id == RAYDIUM_LAUNCHLAB_PROGRAM_ID {
        f.includes_raydium_launchlab()
    } else if *program_id == RAYDIUM_CPMM_PROGRAM_ID {
        f.includes_raydium_cpmm()
    } else if *program_id == RAYDIUM_CLMM_PROGRAM_ID {
        f.includes_raydium_clmm()
    } else if *program_id == RAYDIUM_AMM_V4_PROGRAM_ID {
        f.includes_raydium_amm_v4()
    } else if *program_id == ORCA_WHIRLPOOL_PROGRAM_ID {
        f.includes_orca_whirlpool()
    } else if *program_id == METEORA_POOLS_PROGRAM_ID {
        f.includes_meteora_pools()
    } else if *program_id == METEORA_DAMM_V2_PROGRAM_ID {
        f.includes_meteora_damm_v2()
    } else if *program_id == METEORA_DLMM_PROGRAM_ID {
        f.includes_meteora_dlmm()
    } else {
        false
    }
}

#[inline]
fn parse_non_pump_dex_outer(
    program_id: Pubkey,
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    filter: Option<&EventTypeFilter>,
) -> Option<DexEvent> {
    crate::instr::parse_instruction_unified(
        data,
        accounts,
        signature,
        slot,
        tx_index,
        None,
        recv_us,
        filter,
        &program_id,
    )
}

/// Parse outer instructions whose program and account keys are fully resolved.
/// Instructions referring to unresolved ALT keys are skipped. Resolve their ALT
/// addresses with `parse_transaction_dex_events_with_loaded_addresses` to parse them.
#[inline]
pub fn parse_transaction_dex_events(
    transaction: &VersionedTransaction,
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    events: &mut Vec<DexEvent>,
) {
    parse_transaction_dex_events_with_filter(
        transaction,
        signature,
        slot,
        tx_index,
        recv_us,
        None,
        events,
    );
}

/// Filtered version of `parse_transaction_dex_events`, with the same resolved-key requirement.
#[inline]
pub fn parse_transaction_dex_events_with_filter(
    transaction: &VersionedTransaction,
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    filter: Option<&EventTypeFilter>,
    events: &mut Vec<DexEvent>,
) {
    parse_transaction_with_account_keys(
        transaction,
        transaction.message.static_account_keys(),
        signature,
        slot,
        tx_index,
        recv_us,
        filter,
        events,
        false,
    );
}

/// Compatibility parser that guesses unresolved program IDs by discriminator and
/// substitutes missing account keys with defaults. Results are provisional and
/// can be ambiguous; use the default parser or resolved ALT entry for parity.
#[inline]
pub fn parse_transaction_dex_events_best_effort(
    transaction: &VersionedTransaction,
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    filter: Option<&EventTypeFilter>,
    events: &mut Vec<DexEvent>,
) {
    parse_transaction_with_account_keys(
        transaction,
        transaction.message.static_account_keys(),
        signature,
        slot,
        tx_index,
        recv_us,
        filter,
        events,
        true,
    );
}

/// Parse outer instructions with addresses resolved from this transaction's ALT lookups.
///
/// Supply all writable addresses followed by all readonly addresses, in message lookup order.
/// Resolve them using a caller-managed cache or RPC before calling this function. No network
/// access occurs here. Wrong address counts or out-of-range instruction indices return an
/// error without appending events. Address identity remains the resolver's responsibility.
#[inline]
pub fn parse_transaction_dex_events_with_loaded_addresses(
    transaction: &VersionedTransaction,
    loaded_writable: &[Pubkey],
    loaded_readonly: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    filter: Option<&EventTypeFilter>,
    events: &mut Vec<DexEvent>,
) -> anyhow::Result<()> {
    let (writable_count, readonly_count) = transaction
        .message
        .address_table_lookups()
        .unwrap_or_default()
        .iter()
        .fold((0usize, 0usize), |(w, r), lookup| {
            (
                w + lookup.writable_indexes.len(),
                r + lookup.readonly_indexes.len(),
            )
        });
    anyhow::ensure!(
        loaded_writable.len() == writable_count && loaded_readonly.len() == readonly_count,
        "resolved ALT address counts do not match transaction lookups"
    );
    let mut keys: SmallVec<[Pubkey; 64]> = SmallVec::new();
    keys.extend_from_slice(transaction.message.static_account_keys());
    keys.extend_from_slice(loaded_writable);
    keys.extend_from_slice(loaded_readonly);
    anyhow::ensure!(
        transaction
            .message
            .instructions()
            .iter()
            .all(|ix| (ix.program_id_index as usize) < keys.len()
                && ix.accounts.iter().all(|&idx| (idx as usize) < keys.len())),
        "instruction index exceeds resolved transaction account keys"
    );
    parse_transaction_with_account_keys(
        transaction,
        &keys,
        signature,
        slot,
        tx_index,
        recv_us,
        filter,
        events,
        false,
    );
    Ok(())
}

#[inline]
fn parse_transaction_with_account_keys(
    transaction: &VersionedTransaction,
    account_keys: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    filter: Option<&EventTypeFilter>,
    events: &mut Vec<DexEvent>,
    best_effort: bool,
) {
    let start = events.len();
    parse_transaction_pump_events_with_filter(
        transaction,
        account_keys,
        signature,
        slot,
        tx_index,
        recv_us,
        filter,
        events,
        best_effort,
    );
    let appended = &mut events[start..];
    crate::core::pumpfun_fee_enrich::enrich_pumpfun_same_tx_post_merge(appended);
    let mut metadata = appended.iter_mut().filter_map(DexEvent::metadata_mut);
    if let Some(first) = metadata.next() {
        let blockhash = transaction.message.recent_blockhash().to_string();
        for item in metadata {
            item.grpc_recv_us = recv_us;
            item.recent_blockhash = Some(blockhash.clone());
        }
        first.grpc_recv_us = recv_us;
        first.recent_blockhash = Some(blockhash);
    }
}

#[inline]
fn parse_transaction_pump_events_with_filter(
    transaction: &VersionedTransaction,
    static_keys: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    filter: Option<&EventTypeFilter>,
    events: &mut Vec<DexEvent>,
    best_effort: bool,
) {
    let (created_mints, mayhem_mints) = if filter.is_none_or(|f| f.includes_pumpfun()) {
        detect_pumpfun_create_mints(&transaction.message, static_keys, best_effort)
    } else {
        (PumpMintSet::new(), PumpMintSet::new())
    };
    for ix in transaction.message.instructions() {
        if !best_effort
            && !instruction_keys_resolved(ix.program_id_index, &ix.accounts, static_keys.len())
        {
            continue;
        }
        if !best_effort
            && static_keys.get(ix.program_id_index as usize) == Some(&PROGRAM_ID_PUBKEY)
            && ix.data.len() >= 8
        {
            let disc = &ix.data[..8];
            let minimum = if disc == discriminators::BUY_V2
                || disc == discriminators::BUY_EXACT_QUOTE_IN_V2
            {
                27
            } else if disc == discriminators::SELL_V2 {
                26
            } else {
                0
            };
            if ix.accounts.len() < minimum {
                continue;
            }
        }
        dispatch_shred_outer(
            ix.program_id_index,
            &ix.accounts,
            &ix.data,
            static_keys,
            signature,
            slot,
            tx_index,
            recv_us,
            filter,
            &created_mints,
            &mayhem_mints,
            events,
        );
    }
}

// --- 单条 outer ix 解析（由原 `client.rs` 迁入） ---

#[inline]
fn parse_pumpfun_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    created_mints: &PumpMintSet,
    mayhem_mints: &PumpMintSet,
) -> Option<DexEvent> {
    if data.len() < 8 {
        return None;
    }
    let disc: [u8; 8] = data[0..8].try_into().ok()?;
    let ix_data = &data[8..];

    let mut event = match disc {
        d if d == discriminators::CREATE => {
            parse_create_instruction(data, accounts, signature, slot, tx_index, recv_us)
        }
        d if d == discriminators::CREATE_V2 => {
            parse_create_v2_instruction(data, accounts, signature, slot, tx_index, recv_us)
        }
        d if d == discriminators::BUY => parse_buy_instruction(
            ix_data,
            accounts,
            signature,
            slot,
            tx_index,
            recv_us,
            created_mints,
            mayhem_mints,
        ),
        d if d == discriminators::SELL => {
            parse_sell_instruction(ix_data, accounts, signature, slot, tx_index, recv_us)
        }
        d if d == discriminators::BUY_EXACT_SOL_IN => parse_buy_exact_sol_in_instruction(
            ix_data,
            accounts,
            signature,
            slot,
            tx_index,
            recv_us,
            created_mints,
            mayhem_mints,
        ),
        d if d == discriminators::BUY_V2 => parse_buy_v2_instruction(
            ix_data,
            accounts,
            signature,
            slot,
            tx_index,
            recv_us,
            created_mints,
            mayhem_mints,
        ),
        d if d == discriminators::BUY_EXACT_QUOTE_IN_V2 => parse_buy_exact_quote_in_v2_instruction(
            ix_data,
            accounts,
            signature,
            slot,
            tx_index,
            recv_us,
            created_mints,
            mayhem_mints,
        ),
        d if d == discriminators::SELL_V2 => {
            parse_sell_v2_instruction(ix_data, accounts, signature, slot, tx_index, recv_us)
        }
        discriminators::BUY_V3
        | discriminators::BUY_EXACT_QUOTE_IN_V3
        | discriminators::SELL_V3 => crate::instr::pump::parse_instruction(
            data, accounts, signature, slot, tx_index, None, recv_us,
        ),
        d if d == discriminators::MIGRATE_BONDING_CURVE_CREATOR => {
            parse_migrate_bonding_curve_creator_shred(accounts, signature, slot, tx_index, recv_us)
        }
        _ => None,
    }?;
    if matches!(
        disc,
        discriminators::BUY_V3 | discriminators::BUY_EXACT_QUOTE_IN_V3
    ) {
        if let DexEvent::PumpFunBuy(trade) = &mut event {
            trade.is_created_buy = created_mints.contains(&trade.mint);
            trade.mayhem_mode = mayhem_mints.contains(&trade.mint);
        }
    }
    if let DexEvent::PumpFunCreate(create) = &mut event {
        let get = |i| accounts.get(i).copied().unwrap_or_default();
        if disc == discriminators::CREATE {
            crate::core::account_fillers::pumpfun::fill_create_accounts(create, &get);
        } else if disc == discriminators::CREATE_V2 {
            crate::core::account_fillers::pumpfun::fill_create_accounts_from_v2(create, &get);
        }
    }
    Some(event)
}

/// `migrate_bonding_curve_creator` 外层 ix（`idls/pumpfun.json`）；无链上事件体时 `timestamp=0`，
/// `old_creator` 未知则填默认，`new_creator` 依赖执行或账户状态，不能用 `sharing_config` 地址代替。
#[inline]
fn parse_migrate_bonding_curve_creator_shred(
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
) -> Option<DexEvent> {
    const MIN_ACC: usize = 5;
    if accounts.len() < MIN_ACC {
        return None;
    }
    let get_account = |idx: usize| -> Option<Pubkey> { accounts.get(idx).copied() };
    let mint = get_account(0)?;
    let bonding_curve = get_account(1).unwrap_or_default();
    let sharing_config = get_account(2).unwrap_or_default();
    let metadata = EventMetadata {
        signature,
        slot,
        tx_index,
        block_time_us: 0,
        grpc_recv_us: recv_us,
        recent_blockhash: None,
    };
    Some(DexEvent::PumpFunMigrateBondingCurveCreator(
        PumpFunMigrateBondingCurveCreatorEvent {
            metadata,
            timestamp: 0,
            mint,
            bonding_curve,
            sharing_config,
            old_creator: Pubkey::default(),
            new_creator: Pubkey::default(),
        },
    ))
}

#[inline]
fn parse_create_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
) -> Option<DexEvent> {
    if accounts.len() < 10 {
        return None;
    }

    let get_account = |idx: usize| -> Option<Pubkey> { accounts.get(idx).copied() };

    let mut offset = 8;

    let (name, len) = read_str_unchecked(data, offset)?;
    offset += len;
    let name = name.to_string();

    let (symbol, len) = read_str_unchecked(data, offset)?;
    offset += len;
    let symbol = symbol.to_string();

    let (uri, len) = read_str_unchecked(data, offset)?;
    offset += len;
    let uri = uri.to_string();

    // Historical create instructions had no creator argument; partial keys are invalid.
    let creator = if offset == data.len() {
        Pubkey::default()
    } else {
        read_pubkey(data, offset)?
    };

    let mint = get_account(create_accounts::MINT)?;
    let bonding_curve = get_account(create_accounts::BONDING_CURVE).unwrap_or_default();
    let user = get_account(create_accounts::USER).unwrap_or_default();

    let metadata = EventMetadata {
        signature,
        slot,
        tx_index,
        block_time_us: 0,
        grpc_recv_us: recv_us,
        recent_blockhash: None,
    };

    Some(DexEvent::PumpFunCreate(PumpFunCreateTokenEvent {
        metadata,
        name,
        symbol,
        uri,
        mint,
        bonding_curve,
        user,
        creator,
        token_program: get_account(create_accounts::TOKEN_PROGRAM).unwrap_or_default(),
        quote_mint: PUMPFUN_SOLSCAN_SOL_QUOTE_MINT,
        ix_name: "create".to_string(),
        ..Default::default()
    }))
}

#[inline]
fn parse_create_v2_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
) -> Option<DexEvent> {
    const CREATE_V2_MIN_ACCOUNTS: usize = create_v2_accounts::LEN;
    if accounts.len() < CREATE_V2_MIN_ACCOUNTS {
        return None;
    }

    let get_account = |idx: usize| -> Option<Pubkey> { accounts.get(idx).copied() };

    let payload = &data[8..];
    let mut offset = 0usize;
    let (name, len) = read_str_unchecked(payload, offset)?;
    offset += len;
    let (symbol, len) = read_str_unchecked(payload, offset)?;
    offset += len;
    let (uri, len) = read_str_unchecked(payload, offset)?;
    offset += len;
    if payload.len() < offset + 32 + 1 {
        return None;
    }
    let creator = read_pubkey(payload, offset).unwrap_or_default();
    offset += 32;
    let is_mayhem_mode = read_option_bool_idl(payload, offset)?;
    offset += 1;
    let (is_cashback_enabled, creator_fee_bps, is_holder_reward) =
        crate::instr::utils::parse_create_v2_optional_tail(&payload[offset..])?;

    let mint = get_account(create_v2_accounts::MINT)?;
    let bonding_curve = get_account(create_v2_accounts::BONDING_CURVE).unwrap_or_default();
    let user = get_account(create_v2_accounts::USER).unwrap_or_default();

    let metadata = EventMetadata {
        signature,
        slot,
        tx_index,
        block_time_us: 0,
        grpc_recv_us: recv_us,
        recent_blockhash: None,
    };

    let mayhem_program_id = get_account(create_v2_accounts::MAYHEM_PROGRAM_ID).unwrap_or_default();
    let (quote_mint, quote_vault, quote_token_program) =
        crate::instr::pump::create_v2_quote_accounts(accounts.len(), get_account);

    Some(DexEvent::PumpFunCreate(PumpFunCreateTokenEvent {
        metadata,
        name: name.to_string(),
        symbol: symbol.to_string(),
        uri: uri.to_string(),
        mint,
        bonding_curve,
        user,
        creator,
        mint_authority: get_account(create_v2_accounts::MINT_AUTHORITY).unwrap_or_default(),
        associated_bonding_curve: get_account(create_v2_accounts::ASSOCIATED_BONDING_CURVE)
            .unwrap_or_default(),
        global: get_account(create_v2_accounts::GLOBAL).unwrap_or_default(),
        system_program: get_account(create_v2_accounts::SYSTEM_PROGRAM).unwrap_or_default(),
        token_program: get_account(create_v2_accounts::TOKEN_PROGRAM).unwrap_or_default(),
        associated_token_program: get_account(create_v2_accounts::ASSOCIATED_TOKEN_PROGRAM)
            .unwrap_or_default(),
        mayhem_program_id,
        global_params: get_account(create_v2_accounts::GLOBAL_PARAMS).unwrap_or_default(),
        sol_vault: get_account(create_v2_accounts::SOL_VAULT).unwrap_or_default(),
        mayhem_state: get_account(create_v2_accounts::MAYHEM_STATE).unwrap_or_default(),
        mayhem_token_vault: get_account(create_v2_accounts::MAYHEM_TOKEN_VAULT).unwrap_or_default(),
        event_authority: get_account(create_v2_accounts::EVENT_AUTHORITY).unwrap_or_default(),
        program: get_account(create_v2_accounts::PROGRAM).unwrap_or_default(),
        is_mayhem_mode,
        is_cashback_enabled,
        creator_fee_bps,
        is_holder_reward,
        quote_mint,
        quote_vault,
        quote_token_program,
        ix_name: "create_v2".to_string(),
        ..Default::default()
    }))
}

#[inline]
fn parse_buy_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    created_mints: &PumpMintSet,
    mayhem_mints: &PumpMintSet,
) -> Option<DexEvent> {
    const LEGACY_BUY_ACCOUNTS: usize = 16;
    if accounts.len() < LEGACY_BUY_ACCOUNTS {
        return None;
    }

    if data.get(16).is_some_and(|value| *value > 1) {
        return None;
    }

    let get_account = |idx: usize| -> Option<Pubkey> { accounts.get(idx).copied() };

    let (token_amount, sol_amount) = (read_u64_le(data, 0)?, read_u64_le(data, 8)?);

    let mint = get_account(2)?;
    let is_created_buy = created_mints.contains(&mint);
    let is_mayhem_mode = mayhem_mints.contains(&mint);

    let metadata = EventMetadata {
        signature,
        slot,
        tx_index,
        block_time_us: 0,
        grpc_recv_us: recv_us,
        recent_blockhash: None,
    };

    Some(DexEvent::PumpFunBuy(PumpFunTradeEvent {
        metadata,
        mint,
        quote_mint: PUMPFUN_SOLSCAN_SOL_QUOTE_MINT,
        global: get_account(0).unwrap_or_default(),
        bonding_curve: get_account(3).unwrap_or_default(),
        bonding_curve_v2: get_account(16).unwrap_or_default(),
        associated_bonding_curve: get_account(4).unwrap_or_default(),
        associated_user: get_account(5).unwrap_or_default(),
        user: get_account(6).unwrap_or_default(),
        system_program: get_account(7).unwrap_or_default(),
        sol_amount,
        token_amount,
        amount: token_amount,
        max_sol_cost: sol_amount,
        min_sol_output: 0,
        spendable_sol_in: 0,
        spendable_quote_in: 0,
        min_tokens_out: 0,
        fee_recipient: get_account(1).unwrap_or_default(),
        token_program: token_program_or_default(get_account(8).unwrap_or_default()),
        creator_vault: get_account(9).unwrap_or_default(),
        event_authority: get_account(10).unwrap_or_default(),
        program: get_account(11).unwrap_or_default(),
        global_volume_accumulator: get_account(12).unwrap_or_default(),
        user_volume_accumulator: get_account(13).unwrap_or_default(),
        fee_config: get_account(14).unwrap_or_default(),
        fee_program: get_account(15).unwrap_or_default(),
        is_buy: true,
        is_created_buy,
        timestamp: 0,
        virtual_sol_reserves: 0,
        virtual_token_reserves: 0,
        real_sol_reserves: 0,
        real_token_reserves: 0,
        fee_basis_points: 0,
        fee: 0,
        creator: Pubkey::default(),
        creator_fee_basis_points: 0,
        creator_fee: 0,
        track_volume: data.get(16).copied().map(|b| b != 0).unwrap_or(false),
        total_unclaimed_tokens: 0,
        total_claimed_tokens: 0,
        current_sol_volume: 0,
        last_update_timestamp: 0,
        ix_name: "buy".to_string(),
        mayhem_mode: is_mayhem_mode,
        cashback_fee_basis_points: 0,
        cashback: 0,
        is_cashback_coin: false,
        buyback_fee_recipient: get_account(17).unwrap_or_default(),
        account: get_account(17).filter(|pk| *pk != Pubkey::default()),
        ..Default::default()
    }))
}

#[inline]
fn parse_sell_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
) -> Option<DexEvent> {
    const LEGACY_SELL_ACCOUNTS: usize = 14;
    if accounts.len() < LEGACY_SELL_ACCOUNTS {
        return None;
    }

    let get_account = |idx: usize| -> Option<Pubkey> { accounts.get(idx).copied() };

    let (token_amount, sol_amount) = (read_u64_le(data, 0)?, read_u64_le(data, 8)?);

    let mint = get_account(2)?;
    let metadata = EventMetadata {
        signature,
        slot,
        tx_index,
        block_time_us: 0,
        grpc_recv_us: recv_us,
        recent_blockhash: None,
    };

    Some(DexEvent::PumpFunSell(PumpFunTradeEvent {
        metadata,
        mint,
        quote_mint: PUMPFUN_SOLSCAN_SOL_QUOTE_MINT,
        global: get_account(0).unwrap_or_default(),
        bonding_curve: get_account(3).unwrap_or_default(),
        bonding_curve_v2: if accounts.len() >= 17 {
            get_account(15).unwrap_or_default()
        } else {
            get_account(14).unwrap_or_default()
        },
        associated_bonding_curve: get_account(4).unwrap_or_default(),
        associated_user: get_account(5).unwrap_or_default(),
        user: get_account(6).unwrap_or_default(),
        system_program: get_account(7).unwrap_or_default(),
        sol_amount,
        token_amount,
        amount: token_amount,
        max_sol_cost: 0,
        min_sol_output: sol_amount,
        spendable_sol_in: 0,
        spendable_quote_in: 0,
        min_tokens_out: 0,
        fee_recipient: get_account(1).unwrap_or_default(),
        token_program: token_program_or_default(get_account(9).unwrap_or_default()),
        creator_vault: get_account(8).unwrap_or_default(),
        event_authority: get_account(10).unwrap_or_default(),
        program: get_account(11).unwrap_or_default(),
        global_volume_accumulator: Pubkey::default(),
        user_volume_accumulator: if accounts.len() >= 17 {
            get_account(14).unwrap_or_default()
        } else {
            Pubkey::default()
        },
        fee_config: get_account(12).unwrap_or_default(),
        fee_program: get_account(13).unwrap_or_default(),
        is_buy: false,
        is_created_buy: false,
        timestamp: 0,
        virtual_sol_reserves: 0,
        virtual_token_reserves: 0,
        real_sol_reserves: 0,
        real_token_reserves: 0,
        fee_basis_points: 0,
        fee: 0,
        creator: Pubkey::default(),
        creator_fee_basis_points: 0,
        creator_fee: 0,
        track_volume: false,
        total_unclaimed_tokens: 0,
        total_claimed_tokens: 0,
        current_sol_volume: 0,
        last_update_timestamp: 0,
        ix_name: "sell".to_string(),
        mayhem_mode: false,
        cashback_fee_basis_points: 0,
        cashback: 0,
        is_cashback_coin: false,
        buyback_fee_recipient: if accounts.len() >= 17 {
            get_account(16).unwrap_or_default()
        } else {
            get_account(15).unwrap_or_default()
        },
        account: if accounts.len() >= 17 {
            get_account(16).filter(|pk| *pk != Pubkey::default())
        } else {
            get_account(15).filter(|pk| *pk != Pubkey::default())
        },
        ..Default::default()
    }))
}

#[inline]
fn parse_buy_exact_sol_in_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    created_mints: &PumpMintSet,
    mayhem_mints: &PumpMintSet,
) -> Option<DexEvent> {
    const LEGACY_BUY_ACCOUNTS: usize = 16;
    if accounts.len() < LEGACY_BUY_ACCOUNTS {
        return None;
    }

    if data.get(16).is_some_and(|value| *value > 1) {
        return None;
    }

    let get_account = |idx: usize| -> Option<Pubkey> { accounts.get(idx).copied() };

    let (sol_amount, token_amount) = (read_u64_le(data, 0)?, read_u64_le(data, 8)?);

    let mint = get_account(2)?;
    let is_created_buy = created_mints.contains(&mint);
    let is_mayhem_mode = mayhem_mints.contains(&mint);

    let metadata = EventMetadata {
        signature,
        slot,
        tx_index,
        block_time_us: 0,
        grpc_recv_us: recv_us,
        recent_blockhash: None,
    };

    Some(DexEvent::PumpFunBuyExactSolIn(PumpFunTradeEvent {
        metadata,
        mint,
        quote_mint: PUMPFUN_SOLSCAN_SOL_QUOTE_MINT,
        global: get_account(0).unwrap_or_default(),
        bonding_curve: get_account(3).unwrap_or_default(),
        bonding_curve_v2: get_account(16).unwrap_or_default(),
        associated_bonding_curve: get_account(4).unwrap_or_default(),
        associated_user: get_account(5).unwrap_or_default(),
        user: get_account(6).unwrap_or_default(),
        system_program: get_account(7).unwrap_or_default(),
        sol_amount,
        token_amount,
        amount: token_amount,
        max_sol_cost: sol_amount,
        min_sol_output: 0,
        spendable_sol_in: sol_amount,
        spendable_quote_in: 0,
        min_tokens_out: token_amount,
        fee_recipient: get_account(1).unwrap_or_default(),
        token_program: token_program_or_default(get_account(8).unwrap_or_default()),
        creator_vault: get_account(9).unwrap_or_default(),
        event_authority: get_account(10).unwrap_or_default(),
        program: get_account(11).unwrap_or_default(),
        global_volume_accumulator: get_account(12).unwrap_or_default(),
        user_volume_accumulator: get_account(13).unwrap_or_default(),
        fee_config: get_account(14).unwrap_or_default(),
        fee_program: get_account(15).unwrap_or_default(),
        is_buy: true,
        is_created_buy,
        timestamp: 0,
        virtual_sol_reserves: 0,
        virtual_token_reserves: 0,
        real_sol_reserves: 0,
        real_token_reserves: 0,
        fee_basis_points: 0,
        fee: 0,
        creator: Pubkey::default(),
        creator_fee_basis_points: 0,
        creator_fee: 0,
        track_volume: data.get(16).copied().map(|b| b != 0).unwrap_or(false),
        total_unclaimed_tokens: 0,
        total_claimed_tokens: 0,
        current_sol_volume: 0,
        last_update_timestamp: 0,
        ix_name: "buy_exact_sol_in".to_string(),
        mayhem_mode: is_mayhem_mode,
        cashback_fee_basis_points: 0,
        cashback: 0,
        is_cashback_coin: false,
        buyback_fee_recipient: get_account(17).unwrap_or_default(),
        account: get_account(17).filter(|pk| *pk != Pubkey::default()),
        ..Default::default()
    }))
}

/// `buy_v2`：27 个固定账户（IDL `buy_v2`）；mint=#1 bonding_curve=#10 user=#13 fee=#6 base_token_program=#3。
/// ShredStream can see shortened account lists, so only `mint` is required and all later accounts
/// are filled best-effort.
#[inline]
fn parse_buy_v2_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    created_mints: &PumpMintSet,
    mayhem_mints: &PumpMintSet,
) -> Option<DexEvent> {
    let get_account = |idx: usize| -> Option<Pubkey> { accounts.get(idx).copied() };

    let (token_amount, sol_amount) = (read_u64_le(data, 0)?, read_u64_le(data, 8)?);

    let mint = get_account(1)?;
    let is_created_buy = created_mints.contains(&mint);
    let is_mayhem_mode = mayhem_mints.contains(&mint);

    let metadata = EventMetadata {
        signature,
        slot,
        tx_index,
        block_time_us: 0,
        grpc_recv_us: recv_us,
        recent_blockhash: None,
    };

    Some(DexEvent::PumpFunBuy(PumpFunTradeEvent {
        metadata,
        mint,
        quote_mint: quote_mint_from_shred_v2_account(get_account(2)),
        global: get_account(0).unwrap_or_default(),
        bonding_curve: get_account(10).unwrap_or_default(),
        associated_bonding_curve: get_account(11).unwrap_or_default(),
        associated_quote_bonding_curve: get_account(12).unwrap_or_default(),
        associated_user: get_account(14).unwrap_or_default(),
        associated_quote_user: get_account(15).unwrap_or_default(),
        user: get_account(13).unwrap_or_default(),
        system_program: get_account(24).unwrap_or_default(),
        sol_amount,
        token_amount,
        amount: token_amount,
        max_sol_cost: sol_amount,
        min_sol_output: 0,
        spendable_sol_in: 0,
        spendable_quote_in: 0,
        min_tokens_out: 0,
        fee_recipient: get_account(6).unwrap_or_default(),
        token_program: token_program_or_default(get_account(3).unwrap_or_default()),
        quote_token_program: token_program_or_default(get_account(4).unwrap_or_default()),
        associated_token_program: get_account(5).unwrap_or_default(),
        creator_vault: get_account(16).unwrap_or_default(),
        associated_quote_fee_recipient: get_account(7).unwrap_or_default(),
        buyback_fee_recipient: get_account(8).unwrap_or_default(),
        associated_quote_buyback_fee_recipient: get_account(9).unwrap_or_default(),
        associated_creator_vault: get_account(17).unwrap_or_default(),
        sharing_config: get_account(18).unwrap_or_default(),
        event_authority: get_account(25).unwrap_or_default(),
        program: get_account(26).unwrap_or_default(),
        global_volume_accumulator: get_account(19).unwrap_or_default(),
        user_volume_accumulator: get_account(20).unwrap_or_default(),
        associated_user_volume_accumulator: get_account(21).unwrap_or_default(),
        fee_config: get_account(22).unwrap_or_default(),
        fee_program: get_account(23).unwrap_or_default(),
        is_buy: true,
        is_created_buy,
        timestamp: 0,
        virtual_sol_reserves: 0,
        virtual_token_reserves: 0,
        real_sol_reserves: 0,
        real_token_reserves: 0,
        fee_basis_points: 0,
        fee: 0,
        creator: Pubkey::default(),
        creator_fee_basis_points: 0,
        creator_fee: 0,
        track_volume: false,
        total_unclaimed_tokens: 0,
        total_claimed_tokens: 0,
        current_sol_volume: 0,
        last_update_timestamp: 0,
        ix_name: "buy_v2".to_string(),
        mayhem_mode: is_mayhem_mode,
        cashback_fee_basis_points: 0,
        cashback: 0,
        is_cashback_coin: false,
        account: None,
        ..Default::default()
    }))
}

#[inline]
fn parse_buy_exact_quote_in_v2_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
    created_mints: &PumpMintSet,
    mayhem_mints: &PumpMintSet,
) -> Option<DexEvent> {
    let get_account = |idx: usize| -> Option<Pubkey> { accounts.get(idx).copied() };

    let (sol_amount, token_amount) = (read_u64_le(data, 0)?, read_u64_le(data, 8)?);

    let mint = get_account(1)?;
    let is_created_buy = created_mints.contains(&mint);
    let is_mayhem_mode = mayhem_mints.contains(&mint);

    let metadata = EventMetadata {
        signature,
        slot,
        tx_index,
        block_time_us: 0,
        grpc_recv_us: recv_us,
        recent_blockhash: None,
    };

    Some(DexEvent::PumpFunBuy(PumpFunTradeEvent {
        metadata,
        mint,
        quote_mint: quote_mint_from_shred_v2_account(get_account(2)),
        global: get_account(0).unwrap_or_default(),
        bonding_curve: get_account(10).unwrap_or_default(),
        associated_bonding_curve: get_account(11).unwrap_or_default(),
        associated_quote_bonding_curve: get_account(12).unwrap_or_default(),
        associated_user: get_account(14).unwrap_or_default(),
        associated_quote_user: get_account(15).unwrap_or_default(),
        user: get_account(13).unwrap_or_default(),
        system_program: get_account(24).unwrap_or_default(),
        sol_amount,
        token_amount,
        amount: token_amount,
        max_sol_cost: 0,
        quote_amount: sol_amount,
        min_sol_output: 0,
        spendable_sol_in: 0,
        spendable_quote_in: sol_amount,
        min_tokens_out: token_amount,
        fee_recipient: get_account(6).unwrap_or_default(),
        token_program: token_program_or_default(get_account(3).unwrap_or_default()),
        quote_token_program: token_program_or_default(get_account(4).unwrap_or_default()),
        associated_token_program: get_account(5).unwrap_or_default(),
        creator_vault: get_account(16).unwrap_or_default(),
        associated_quote_fee_recipient: get_account(7).unwrap_or_default(),
        buyback_fee_recipient: get_account(8).unwrap_or_default(),
        associated_quote_buyback_fee_recipient: get_account(9).unwrap_or_default(),
        associated_creator_vault: get_account(17).unwrap_or_default(),
        sharing_config: get_account(18).unwrap_or_default(),
        event_authority: get_account(25).unwrap_or_default(),
        program: get_account(26).unwrap_or_default(),
        global_volume_accumulator: get_account(19).unwrap_or_default(),
        user_volume_accumulator: get_account(20).unwrap_or_default(),
        associated_user_volume_accumulator: get_account(21).unwrap_or_default(),
        fee_config: get_account(22).unwrap_or_default(),
        fee_program: get_account(23).unwrap_or_default(),
        is_buy: true,
        is_created_buy,
        timestamp: 0,
        virtual_sol_reserves: 0,
        virtual_token_reserves: 0,
        real_sol_reserves: 0,
        real_token_reserves: 0,
        fee_basis_points: 0,
        fee: 0,
        creator: Pubkey::default(),
        creator_fee_basis_points: 0,
        creator_fee: 0,
        track_volume: false,
        total_unclaimed_tokens: 0,
        total_claimed_tokens: 0,
        current_sol_volume: 0,
        last_update_timestamp: 0,
        ix_name: "buy_exact_quote_in_v2".to_string(),
        mayhem_mode: is_mayhem_mode,
        cashback_fee_basis_points: 0,
        cashback: 0,
        is_cashback_coin: false,
        account: None,
        ..Default::default()
    }))
}

/// `sell_v2`：26 个固定账户（IDL `sell_v2`）。
/// ShredStream can see shortened account lists, so only `mint` is required and all later accounts
/// are filled best-effort.
#[inline]
fn parse_sell_v2_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    recv_us: i64,
) -> Option<DexEvent> {
    let get_account = |idx: usize| -> Option<Pubkey> { accounts.get(idx).copied() };

    let (token_amount, sol_amount) = (read_u64_le(data, 0)?, read_u64_le(data, 8)?);

    let mint = get_account(1)?;

    let metadata = EventMetadata {
        signature,
        slot,
        tx_index,
        block_time_us: 0,
        grpc_recv_us: recv_us,
        recent_blockhash: None,
    };

    Some(DexEvent::PumpFunSell(PumpFunTradeEvent {
        metadata,
        mint,
        quote_mint: quote_mint_from_shred_v2_account(get_account(2)),
        global: get_account(0).unwrap_or_default(),
        bonding_curve: get_account(10).unwrap_or_default(),
        associated_bonding_curve: get_account(11).unwrap_or_default(),
        associated_quote_bonding_curve: get_account(12).unwrap_or_default(),
        associated_user: get_account(14).unwrap_or_default(),
        associated_quote_user: get_account(15).unwrap_or_default(),
        user: get_account(13).unwrap_or_default(),
        system_program: get_account(23).unwrap_or_default(),
        sol_amount,
        token_amount,
        amount: token_amount,
        max_sol_cost: 0,
        min_sol_output: sol_amount,
        spendable_sol_in: 0,
        spendable_quote_in: 0,
        min_tokens_out: 0,
        fee_recipient: get_account(6).unwrap_or_default(),
        token_program: token_program_or_default(get_account(3).unwrap_or_default()),
        quote_token_program: token_program_or_default(get_account(4).unwrap_or_default()),
        associated_token_program: get_account(5).unwrap_or_default(),
        creator_vault: get_account(16).unwrap_or_default(),
        associated_quote_fee_recipient: get_account(7).unwrap_or_default(),
        buyback_fee_recipient: get_account(8).unwrap_or_default(),
        associated_quote_buyback_fee_recipient: get_account(9).unwrap_or_default(),
        associated_creator_vault: get_account(17).unwrap_or_default(),
        sharing_config: get_account(18).unwrap_or_default(),
        event_authority: get_account(24).unwrap_or_default(),
        program: get_account(25).unwrap_or_default(),
        global_volume_accumulator: Pubkey::default(),
        user_volume_accumulator: get_account(19).unwrap_or_default(),
        associated_user_volume_accumulator: get_account(20).unwrap_or_default(),
        fee_config: get_account(21).unwrap_or_default(),
        fee_program: get_account(22).unwrap_or_default(),
        is_buy: false,
        is_created_buy: false,
        timestamp: 0,
        virtual_sol_reserves: 0,
        virtual_token_reserves: 0,
        real_sol_reserves: 0,
        real_token_reserves: 0,
        fee_basis_points: 0,
        fee: 0,
        creator: Pubkey::default(),
        creator_fee_basis_points: 0,
        creator_fee: 0,
        track_volume: false,
        total_unclaimed_tokens: 0,
        total_claimed_tokens: 0,
        current_sol_volume: 0,
        last_update_timestamp: 0,
        ix_name: "sell_v2".to_string(),
        mayhem_mode: false,
        cashback_fee_basis_points: 0,
        cashback: 0,
        is_cashback_coin: false,
        account: None,
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::events::PUMPFUN_WSOL_QUOTE_MINT;
    use solana_sdk::hash::Hash;
    use solana_sdk::message::compiled_instruction::CompiledInstruction;
    use solana_sdk::message::{v0, MessageHeader};
    use solana_sdk::signature::Signature;
    use solana_sdk::transaction::VersionedTransaction;
    use std::str::FromStr;

    fn unique_accounts(n: usize) -> Vec<Pubkey> {
        (0..n).map(|_| Pubkey::new_unique()).collect()
    }

    fn ix_accounts(n: usize) -> Vec<u8> {
        (0..n).map(|i| i as u8).collect()
    }

    fn pk(s: &str) -> Pubkey {
        Pubkey::from_str(s).unwrap()
    }

    fn amount_data(first: u64, second: u64) -> Vec<u8> {
        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&first.to_le_bytes());
        data.extend_from_slice(&second.to_le_bytes());
        data
    }

    fn instruction_data(discriminator: [u8; 8], first: u64, second: u64) -> Vec<u8> {
        let mut data = Vec::with_capacity(24);
        data.extend_from_slice(&discriminator);
        data.extend_from_slice(&amount_data(first, second));
        data
    }

    fn str_arg(s: &str, out: &mut Vec<u8>) {
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
    }

    fn create_data() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&discriminators::CREATE);
        str_arg("Alt Coin", &mut data);
        str_arg("ALT", &mut data);
        str_arg("https://example.invalid/alt.json", &mut data);
        data.extend_from_slice(Pubkey::new_unique().as_ref());
        data
    }

    fn create_v2_data() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&discriminators::CREATE_V2);
        str_arg("Alt Coin", &mut data);
        str_arg("ALT", &mut data);
        str_arg("https://example.invalid/alt.json", &mut data);
        data.extend_from_slice(Pubkey::new_unique().as_ref());
        data.push(1);
        data.push(1);
        data
    }

    fn create_v2_data_with_holder_tail() -> Vec<u8> {
        let mut data = create_v2_data();
        data.extend_from_slice(&250u64.to_le_bytes());
        data.push(1);
        data
    }

    fn v0_tx(
        program_id_index: u8,
        account_keys: Vec<Pubkey>,
        ix_accounts: Vec<u8>,
        data: Vec<u8>,
    ) -> VersionedTransaction {
        VersionedTransaction {
            signatures: vec![Signature::default()],
            message: VersionedMessage::V0(v0::Message {
                header: MessageHeader {
                    num_required_signatures: 1,
                    num_readonly_signed_accounts: 0,
                    num_readonly_unsigned_accounts: 0,
                },
                account_keys,
                recent_blockhash: Hash::default(),
                instructions: vec![CompiledInstruction::new_from_raw_parts(
                    program_id_index,
                    data,
                    ix_accounts,
                )],
                address_table_lookups: Vec::new(),
            }),
        }
    }

    fn v0_tx_with_instructions(
        account_keys: Vec<Pubkey>,
        instructions: Vec<(u8, Vec<u8>, Vec<u8>)>,
    ) -> VersionedTransaction {
        VersionedTransaction {
            signatures: vec![Signature::default()],
            message: VersionedMessage::V0(v0::Message {
                header: MessageHeader {
                    num_required_signatures: 1,
                    num_readonly_signed_accounts: 0,
                    num_readonly_unsigned_accounts: 0,
                },
                account_keys,
                recent_blockhash: Hash::default(),
                instructions: instructions
                    .into_iter()
                    .map(|(program_id_index, accounts, data)| {
                        CompiledInstruction::new_from_raw_parts(program_id_index, data, accounts)
                    })
                    .collect(),
                address_table_lookups: Vec::new(),
            }),
        }
    }

    fn parse_best_effort_shred_events(tx: &VersionedTransaction) -> Vec<DexEvent> {
        let mut events = Vec::new();
        parse_transaction_dex_events_best_effort(
            tx,
            Signature::default(),
            123,
            0,
            456,
            None,
            &mut events,
        );
        crate::core::pumpfun_fee_enrich::enrich_pumpfun_same_tx_post_merge(&mut events);
        events
    }

    #[test]
    fn shred_v3_trades_match_shared_decoder_and_keep_creation_markers() {
        for disc in [
            discriminators::BUY_V3,
            discriminators::BUY_EXACT_QUOTE_IN_V3,
            discriminators::SELL_V3,
        ] {
            let mut accounts = unique_accounts(17);
            accounts[16] = PROGRAM_ID_PUBKEY;
            let data = instruction_data(disc, 101, 202);
            let tx = v0_tx(16, accounts.clone(), ix_accounts(17), data.clone());
            let mut events = Vec::new();
            parse_transaction_dex_events(&tx, Signature::default(), 123, 4, 456, &mut events);
            assert_eq!(events.len(), 1);
            let expected = crate::instr::pump::parse_instruction(
                &data,
                &accounts,
                Signature::default(),
                123,
                4,
                None,
                456,
            )
            .unwrap();
            let (actual, expected) = match (events.pop().unwrap(), expected) {
                (DexEvent::PumpFunBuy(a), DexEvent::PumpFunBuy(e))
                | (DexEvent::PumpFunSell(a), DexEvent::PumpFunSell(e)) => (a, e),
                _ => panic!("same V3 trade variant"),
            };
            assert_eq!(actual.ix_name, expected.ix_name);
            assert_eq!(actual.mint, accounts[1]);
            assert_eq!(actual.bonding_curve, accounts[5]);
            assert_eq!(actual.user, accounts[8]);
            assert_eq!(actual.associated_user, accounts[9]);
            assert_eq!(actual.associated_quote_user, accounts[10]);
            assert_eq!(actual.user_volume_accumulator, accounts[11]);
            assert_eq!(actual.fee_config, accounts[12]);
            assert_eq!(actual.buyback_fee_recipient, accounts[13]);
            assert_eq!(actual.program, accounts[16]);
            assert_eq!(actual.quote_mint, expected.quote_mint);
            assert_eq!(actual.amount, expected.amount);
            assert_eq!(actual.max_sol_cost, expected.max_sol_cost);
            assert_eq!(actual.min_sol_output, expected.min_sol_output);
            assert_eq!(actual.spendable_quote_in, expected.spendable_quote_in);
            assert_eq!(actual.min_tokens_out, expected.min_tokens_out);
            assert_eq!(actual.metadata.slot, 123);
            assert_eq!(actual.metadata.grpc_recv_us, 456);
            assert!(actual.metadata.recent_blockhash.is_some());

            let created = PumpMintSet::from_slice(&[accounts[1]]);
            let marked = parse_pumpfun_instruction(
                &data,
                &accounts,
                Signature::default(),
                123,
                4,
                456,
                &created,
                &created,
            )
            .unwrap();
            match marked {
                DexEvent::PumpFunBuy(e) => {
                    assert!(e.is_created_buy);
                    assert!(e.mayhem_mode);
                }
                DexEvent::PumpFunSell(e) => assert!(!e.is_created_buy),
                _ => panic!("V3 trade"),
            }
            for length in 0..24 {
                assert!(parse_pumpfun_instruction(
                    &data[..length],
                    &accounts,
                    Signature::default(),
                    123,
                    4,
                    456,
                    &created,
                    &created,
                )
                .is_none());
            }
            let incomplete = v0_tx(16, accounts, ix_accounts(16), data);
            events.clear();
            parse_transaction_dex_events(
                &incomplete,
                Signature::default(),
                123,
                4,
                456,
                &mut events,
            );
            assert!(events.is_empty());
        }
    }

    #[test]
    fn shred_pumpswap_upgrade_accounts_and_filters_match_all_trade_modes() {
        use crate::grpc::types::EventType;
        use crate::instr::pump_amm::discriminators as amm;
        for (disc, count, pool_v2_index, kind) in [
            (amm::BUY, 25, None, EventType::PumpSwapBuy),
            (amm::BUY, 26, None, EventType::PumpSwapBuy),
            (amm::BUY, 26, Some(23), EventType::PumpSwapBuy),
            (amm::BUY, 27, Some(24), EventType::PumpSwapBuy),
            (amm::BUY_EXACT_QUOTE_IN, 25, None, EventType::PumpSwapBuy),
            (amm::BUY_EXACT_QUOTE_IN, 26, None, EventType::PumpSwapBuy),
            (
                amm::BUY_EXACT_QUOTE_IN,
                26,
                Some(23),
                EventType::PumpSwapBuy,
            ),
            (
                amm::BUY_EXACT_QUOTE_IN,
                27,
                Some(24),
                EventType::PumpSwapBuy,
            ),
            (amm::SELL, 23, None, EventType::PumpSwapSell),
            (amm::SELL, 25, None, EventType::PumpSwapSell),
            (amm::SELL, 24, Some(21), EventType::PumpSwapSell),
            (amm::SELL, 26, Some(23), EventType::PumpSwapSell),
        ] {
            let mut keys = unique_accounts(count + 1);
            keys[count] = PUMPSWAP_PROGRAM_ID;
            let expected_pool = keys[0];
            let expected_pool_v2 = pool_v2_index
                .map(|index| {
                    let pda = Pubkey::find_program_address(
                        &[b"pool-v2", keys[3].as_ref()],
                        &PUMPSWAP_PROGRAM_ID,
                    )
                    .0;
                    keys[index] = pda;
                    pda
                })
                .unwrap_or_default();
            let expected_recipient = keys[count - 2];
            let expected_recipient_ata = keys[count - 1];
            let mut data = instruction_data(disc, 123, 456);
            if disc != amm::SELL {
                data.push(1);
            }
            let tx = v0_tx(count as u8, keys, ix_accounts(count), data);
            let filter = EventTypeFilter::include_only(vec![kind]);
            let mut events = Vec::new();
            parse_transaction_dex_events_with_filter(
                &tx,
                Signature::default(),
                42,
                3,
                1234,
                Some(&filter),
                &mut events,
            );
            assert_eq!(events.len(), 1);
            let (
                pool,
                pool_v2,
                recipient,
                recipient_ata,
                base_reserve,
                quote_reserve,
                virtual_reserve,
            ) = match &events[0] {
                DexEvent::PumpSwapBuy(e) => {
                    assert!(e.track_volume);
                    assert_eq!(
                        e.ix_name,
                        if disc == amm::BUY {
                            "buy"
                        } else {
                            "buy_exact_quote_in"
                        }
                    );
                    if disc == amm::BUY {
                        assert_eq!(e.base_amount_out, 123);
                        assert_eq!(e.max_quote_amount_in, 456);
                    } else {
                        assert_eq!(e.min_base_amount_out, 456);
                        assert_eq!(e.max_quote_amount_in, 123);
                    }
                    (
                        e.pool,
                        e.pool_v2,
                        e.fee_recipient,
                        e.fee_recipient_quote_token_account,
                        e.pool_base_token_reserves,
                        e.pool_quote_token_reserves,
                        e.virtual_quote_reserves,
                    )
                }
                DexEvent::PumpSwapSell(e) => {
                    assert_eq!(e.base_amount_in, 123);
                    assert_eq!(e.min_quote_amount_out, 456);
                    (
                        e.pool,
                        e.pool_v2,
                        e.fee_recipient,
                        e.fee_recipient_quote_token_account,
                        e.pool_base_token_reserves,
                        e.pool_quote_token_reserves,
                        e.virtual_quote_reserves,
                    )
                }
                _ => panic!("expected PumpSwap trade"),
            };
            assert_eq!(
                (pool, pool_v2, recipient, recipient_ata),
                (
                    expected_pool,
                    expected_pool_v2,
                    expected_recipient,
                    expected_recipient_ata
                )
            );
            // Raw shreds have no execution state: these defaults must not seed a price cache.
            assert_eq!((base_reserve, quote_reserve, virtual_reserve), (0, 0, 0));
            let excluded = EventTypeFilter::include_only(vec![EventType::RaydiumCpmmSwap]);
            events.clear();
            parse_transaction_dex_events_with_filter(
                &tx,
                Signature::default(),
                42,
                3,
                1234,
                Some(&excluded),
                &mut events,
            );
            assert!(events.is_empty());
        }
    }

    #[test]
    fn unified_shred_outer_programs_cover_supported_protocols() {
        for program_id in [
            PUMPSWAP_PROGRAM_ID,
            PUMP_FEES_PROGRAM_ID,
            RAYDIUM_LAUNCHLAB_PROGRAM_ID,
            RAYDIUM_CPMM_PROGRAM_ID,
            RAYDIUM_CLMM_PROGRAM_ID,
            RAYDIUM_AMM_V4_PROGRAM_ID,
            ORCA_WHIRLPOOL_PROGRAM_ID,
            METEORA_POOLS_PROGRAM_ID,
            METEORA_DAMM_V2_PROGRAM_ID,
            METEORA_DLMM_PROGRAM_ID,
        ] {
            assert!(
                is_supported_unified_outer_program(&program_id, None),
                "ShredStream outer parser missing {program_id}"
            );
        }
    }

    #[test]
    fn unified_shred_outer_program_filter_skips_unrequested_protocols() {
        let raydium_only =
            EventTypeFilter::include_only(vec![crate::grpc::types::EventType::RaydiumCpmmSwap]);

        assert!(is_supported_unified_outer_program(
            &RAYDIUM_CPMM_PROGRAM_ID,
            Some(&raydium_only)
        ));
        assert!(!is_supported_unified_outer_program(
            &ORCA_WHIRLPOOL_PROGRAM_ID,
            Some(&raydium_only)
        ));
    }

    #[test]
    fn shred_pumpfun_create_best_effort_keeps_static_mint_when_other_accounts_are_alt() {
        let mut static_keys = vec![Pubkey::new_unique(); 10];
        static_keys[9] = PROGRAM_ID_PUBKEY;
        let mint = static_keys[0];
        let mut ix_accounts = ix_accounts(10);
        ix_accounts[2] = 44;
        ix_accounts[7] = 45;
        ix_accounts[9] = 46;

        let tx = v0_tx(9, static_keys, ix_accounts, create_data());
        let mut events = Vec::new();
        parse_transaction_dex_events_best_effort(
            &tx,
            Signature::default(),
            123,
            0,
            456,
            None,
            &mut events,
        );

        assert_eq!(events.len(), 1);
        match &events[0] {
            DexEvent::PumpFunCreate(event) => {
                assert_eq!(event.mint, mint);
                assert_eq!(event.name, "Alt Coin");
                assert_eq!(event.bonding_curve, Pubkey::default());
                assert_eq!(event.user, Pubkey::default());
                assert_eq!(event.token_program, Pubkey::default());
                assert_eq!(event.quote_mint, PUMPFUN_SOLSCAN_SOL_QUOTE_MINT);
                assert_eq!(event.ix_name, "create");
            }
            other => panic!("expected PumpFunCreate, got {other:?}"),
        }
    }

    #[test]
    fn shred_pumpfun_create_uses_instruction_order_accounts() {
        let mut static_keys = vec![Pubkey::new_unique(); 12];
        static_keys[9] = PROGRAM_ID_PUBKEY;
        let mint = static_keys[5];
        let bonding_curve = static_keys[3];
        let user = static_keys[11];
        let token_program = static_keys[4];
        let ix_accounts = vec![5, 1, 3, 2, 6, 7, 8, 11, 10, 4];

        let tx = v0_tx(9, static_keys, ix_accounts, create_data());
        let mut events = Vec::new();
        parse_transaction_dex_events_with_filter(
            &tx,
            Signature::default(),
            123,
            0,
            456,
            None,
            &mut events,
        );

        assert_eq!(events.len(), 1);
        match &events[0] {
            DexEvent::PumpFunCreate(event) => {
                assert_eq!(event.mint, mint);
                assert_eq!(event.bonding_curve, bonding_curve);
                assert_eq!(event.user, user);
                assert_eq!(event.token_program, token_program);
                assert_eq!(event.quote_mint, PUMPFUN_SOLSCAN_SOL_QUOTE_MINT);
                assert_eq!(event.ix_name, "create");
            }
            other => panic!("expected PumpFunCreate, got {other:?}"),
        }
    }

    #[test]
    fn shred_pumpfun_create_v2_uses_appended_quote_mint_only_for_19_accounts() {
        let mut static_keys = vec![Pubkey::new_unique(); 20];
        static_keys[19] = PROGRAM_ID_PUBKEY;
        static_keys[16] = PUMPFUN_WSOL_QUOTE_MINT;
        static_keys[18] = crate::accounts::program_ids::SPL_TOKEN_PROGRAM_ID;
        let tx = v0_tx(19, static_keys, ix_accounts(19), create_v2_data());
        let mut events = Vec::new();

        parse_transaction_dex_events_best_effort(
            &tx,
            Signature::default(),
            123,
            0,
            456,
            None,
            &mut events,
        );

        assert_eq!(events.len(), 1);
        match &events[0] {
            DexEvent::PumpFunCreate(event) => {
                assert_eq!(event.ix_name, "create_v2");
                assert_eq!(event.quote_mint, PUMPFUN_WSOL_QUOTE_MINT);
                assert_eq!(event.creator_fee_bps, 0);
                assert!(!event.is_holder_reward);
            }
            other => panic!("expected PumpFunCreate, got {other:?}"),
        }

        let mut static_keys = vec![Pubkey::new_unique(); 20];
        static_keys[19] = PROGRAM_ID_PUBKEY;
        let mut alt_ix_accounts = ix_accounts(19);
        alt_ix_accounts[16] = 42;
        let tx = v0_tx(19, static_keys, alt_ix_accounts, create_v2_data());
        events.clear();

        parse_transaction_dex_events_best_effort(
            &tx,
            Signature::default(),
            123,
            0,
            456,
            None,
            &mut events,
        );

        assert_eq!(events.len(), 1);
        match &events[0] {
            DexEvent::PumpFunCreate(event) => {
                assert_eq!(event.ix_name, "create_v2");
                assert_eq!(event.quote_mint, Pubkey::default());
            }
            other => panic!("expected PumpFunCreate, got {other:?}"),
        }

        let mut static_keys = vec![Pubkey::new_unique(); 17];
        static_keys[16] = PROGRAM_ID_PUBKEY;
        let tx = v0_tx(16, static_keys, ix_accounts(16), create_v2_data());
        events.clear();
        let signature =
            "H6azwLqtRtrnVNC5iwcjYM9idU3e9SRyLZXTwjfJGJxA4X7dZL7vyhFAJNvQy7bb6bmQNmFHUt1KkkPPmhdge3G";

        parse_transaction_dex_events_best_effort(
            &tx,
            Signature::default(),
            123,
            0,
            456,
            None,
            &mut events,
        );

        assert_eq!(events.len(), 1);
        match &events[0] {
            DexEvent::PumpFunCreate(event) => {
                assert_eq!(event.ix_name, "create_v2");
                assert_eq!(
                    event.quote_mint, PUMPFUN_SOLSCAN_SOL_QUOTE_MINT,
                    "{signature}"
                );
                assert_eq!(event.quote_vault, Pubkey::default(), "{signature}");
                assert_eq!(event.quote_token_program, Pubkey::default(), "{signature}");
            }
            other => panic!("expected PumpFunCreate, got {other:?}"),
        }
    }

    #[test]
    fn shred_pumpfun_create_v2_reads_holder_rewards_tail() {
        let mut static_keys = vec![Pubkey::new_unique(); 17];
        static_keys[16] = PROGRAM_ID_PUBKEY;
        let tx = v0_tx(
            16,
            static_keys,
            ix_accounts(16),
            create_v2_data_with_holder_tail(),
        );
        let mut events = Vec::new();

        parse_transaction_dex_events_with_filter(
            &tx,
            Signature::default(),
            123,
            0,
            456,
            None,
            &mut events,
        );

        assert_eq!(events.len(), 1);
        match &events[0] {
            DexEvent::PumpFunCreate(event) => {
                assert_eq!(event.creator_fee_bps, 250);
                assert!(event.is_holder_reward);
            }
            other => panic!("expected PumpFunCreate, got {other:?}"),
        }
    }

    #[test]
    fn shred_pumpfun_create_v2_rejects_program_id_as_quote_mint() {
        let mut static_keys = vec![Pubkey::new_unique(); 20];
        static_keys[19] = PROGRAM_ID_PUBKEY;
        static_keys[16] = PROGRAM_ID_PUBKEY;
        static_keys[17] = Pubkey::new_unique();
        static_keys[18] = Pubkey::new_unique();
        let tx = v0_tx(19, static_keys, ix_accounts(19), create_v2_data());

        let events = parse_best_effort_shred_events(&tx);

        assert_eq!(events.len(), 1);
        match &events[0] {
            DexEvent::PumpFunCreate(event) => {
                assert_eq!(event.ix_name, "create_v2");
                assert_eq!(event.quote_mint, Pubkey::default());
                assert_eq!(event.quote_vault, Pubkey::default());
                assert_eq!(event.quote_token_program, Pubkey::default());
            }
            other => panic!("expected PumpFunCreate, got {other:?}"),
        }
    }

    #[test]
    fn shred_pumpfun_create_v2_full_accounts_cover_real_quote_cases() {
        // These cases come from user-provided mainnet signatures:
        // 4GCVgY2F... / 5HwZKTwc... / 3jWGFYXT...: create_v2 accounts[16] = So111...12 (WSOL, 19 accounts)
        // 3MVawF6...: create_v2 accounts[16] = EPjF... (USDC)
        // 2dZAucK...: create_v2 accounts[16] = EPjF... (USDC, 19 accounts)
        // oY9YQbie... and 4h9kYj...: create_v2 accounts[16] = So111...12 (WSOL, 20 accounts)
        struct Case {
            signature: &'static str,
            name: &'static str,
            account_len: usize,
            mint: &'static str,
            user: &'static str,
            quote_mint: Pubkey,
            quote_vault: &'static str,
        }
        let token_2022_program = crate::accounts::program_ids::SPL_TOKEN_2022_PROGRAM_ID;
        let spl_token_program = crate::accounts::program_ids::SPL_TOKEN_PROGRAM_ID;
        let cases = [
            Case {
                signature: "4GCVgY2FnT1s4q5zemnPL4mzSbuhUTgQo9mc9jewhLZzsCXKe8ehz6xD4QDJE853CLrF6doJbf4JNwJVeEYLA4De",
                name: "wsol 19-account create_v2",
                account_len: 19,
                mint: "CGY36MoFU627gPH4TLM5NP4Xnvhz6Nesc71TQecPpump",
                user: "Aqje5DsN4u2PHmQxGF9PKfpsDGwQRCBhWeLKHCFhSMXk",
                quote_mint: PUMPFUN_WSOL_QUOTE_MINT,
                quote_vault: "CWR85PmUfzNNgmNN9Ref8L8BvMibZ1tzchiT5bTZpJhn",
            },
            Case {
                signature: "5HwZKTwcGFjSBPugSX5hE9JSq5wKmUooK3tLXuEoyDDzrTvHu7op3XDbhBXuteiC5EePNPh8TC1j6Fns47YvnyeG",
                name: "wsol 19-account create_v2 exact quote buy",
                account_len: 19,
                mint: "7NSSfLGsjNHzKxrgggQ56C2UdKxJVJvrECJR3dsbBuuG",
                user: "2bBRwhGoL4fRZk6g8NnhBZywsF8PdLJnBRfWDCEMogD2",
                quote_mint: PUMPFUN_WSOL_QUOTE_MINT,
                quote_vault: "6jFz2oefpJUE6opjA7vxs3iXou7YYyb6e6E4LN2BFs1W",
            },
            Case {
                signature: "3MVawF6EPtG7rEPXdsyQfQUBLv3epRVNpNS4tRE4uwTPMqLNPqhuABwxU3QZH4uD6CuVupcpGchpNRK5HTbHRLNK",
                name: "usdc 19-account create_v2",
                account_len: 19,
                mint: "FUsqvH5x8QUrxmJhspt6meQZtfBr17m2YsTFuVsYpump",
                user: "9Gg6Mf8tq9zLSpK8qccrQiue3iE7wmyeogKkGZpnz2w5",
                quote_mint: pk("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),
                quote_vault: "7SLtvqMx4bPoWSbPcnWBWpBem3RXbKraWUsiApXjB1VL",
            },
            Case {
                signature: "3jWGFYXT5V33Qc2roEBFDRAWHeybDowr53dSdnYSRkrPdYybU7oyEH9BfgSRxkgFHVKmUjv4e5T33AEnhJvBCuP2",
                name: "wsol 19-account create_v2 with later buy",
                account_len: 19,
                mint: "5i8AZEBc8o5dhfnTQdD3QTVejgbjitwQ1ADHg1jZpump",
                user: "2b2N2p7xCS9ibDqxwYgXpDSTniJwwye7n93WYuzmr74s",
                quote_mint: PUMPFUN_WSOL_QUOTE_MINT,
                quote_vault: "9QB9SyXGDbHUsvvF8XMbYH5ioJMHKHhXTjQDoL56uHT7",
            },
            Case {
                signature: "2dZAucKwr4n5Lqu3BtJ4P8JsjCDtUXJzthadddfURraEJRTgn6XWaTNUNBbgUfP5c2wcVdubqViQhr48eWsgRqPX",
                name: "usdc 19-account create_v2 exact quote buy",
                account_len: 19,
                mint: "DsE8Ptubc1HWWethf9ant4eV9YnofEv5kfGyLdj7jk2Y",
                user: "easy7tXgADWkRMNjFRS2XsLXUAaKH5tEPodh9g7kcX8",
                quote_mint: pk("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"),
                quote_vault: "8QTKfEBf5yChuos4eTzQPbV3jXveCu5GkNKLFoS8oS7t",
            },
            Case {
                signature: "oY9YQbie16Bw11GsqbAPVnW6YjMHAj3kP9sufjcuQjdfcU86iUY8CiSaDrvu4QXJFnGY4jqQc2Kc1YVuAzujvyv",
                name: "wsol 20-account create_v2",
                account_len: 20,
                mint: "Bv3zjsdJ5KuA9KsGirqssC8pVJwCeCeyLjo4Hqpfpump",
                user: "2SWqdMbn1FJVUMUEpuyP2St8BPRtqJYXJPWFfmZr486q",
                quote_mint: PUMPFUN_WSOL_QUOTE_MINT,
                quote_vault: "9QdMAuwtpnHSzjTQcTkjU1GFSs2gNtR66sdQofFv5P7B",
            },
            Case {
                signature: "4h9kYjzYpqqyYZuFnjf14zRwrGyChCuKAYVy6a4ZBig19bydEYsHwp6VbiKqTzT3pLf6NXnf6E25dn1NiU8LR4YB",
                name: "wsol 20-account create_v2 with jit account",
                account_len: 20,
                mint: "6EvDE4a7Yw8F65oy6UhhN3JBshGk9tV3b2yxNyhypump",
                user: "2SWqdMbn1FJVUMUEpuyP2St8BPRtqJYXJPWFfmZr486q",
                quote_mint: PUMPFUN_WSOL_QUOTE_MINT,
                quote_vault: "27jyvk4PUYjcDQkKn8VGT9zNdAxZWWjqALpRUpjMqc2y",
            },
        ];

        for case in cases {
            let mut static_keys = vec![Pubkey::new_unique(); case.account_len + 1];
            let program_idx = case.account_len as u8;
            static_keys[program_idx as usize] = PROGRAM_ID_PUBKEY;
            static_keys[0] = pk(case.mint);
            static_keys[5] = pk(case.user);
            static_keys[7] = token_2022_program;
            static_keys[16] = case.quote_mint;
            static_keys[17] = pk(case.quote_vault);
            if case.account_len > 18 {
                static_keys[18] = spl_token_program;
            }
            let mut accounts = ix_accounts(case.account_len);
            accounts[15] = program_idx;
            let tx = v0_tx(program_idx, static_keys, accounts, create_v2_data());

            let events = parse_best_effort_shred_events(&tx);

            assert_eq!(events.len(), 1, "{}", case.name);
            match &events[0] {
                DexEvent::PumpFunCreate(event) => {
                    assert_eq!(event.ix_name, "create_v2", "{}", case.name);
                    assert_eq!(
                        event.mint,
                        pk(case.mint),
                        "{}: {}",
                        case.name,
                        case.signature
                    );
                    assert_eq!(
                        event.user,
                        pk(case.user),
                        "{}: {}",
                        case.name,
                        case.signature
                    );
                    assert_eq!(
                        event.token_program, token_2022_program,
                        "{}: {}",
                        case.name, case.signature
                    );
                    assert_eq!(
                        event.quote_mint, case.quote_mint,
                        "{}: {}",
                        case.name, case.signature
                    );
                    assert_eq!(
                        event.quote_vault,
                        pk(case.quote_vault),
                        "{}: {}",
                        case.name,
                        case.signature
                    );
                    assert_eq!(
                        event.quote_token_program, spl_token_program,
                        "{}: {}",
                        case.name, case.signature
                    );
                    assert_eq!(
                        event.program, PROGRAM_ID_PUBKEY,
                        "{}: {}",
                        case.name, case.signature
                    );
                }
                other => panic!("{}: expected PumpFunCreate, got {other:?}", case.name),
            }
        }
    }

    #[test]
    fn shred_pumpfun_create_v2_alt_quote_index_is_not_guessable() {
        // Real compiled-index pattern from the user-provided signatures:
        // - 4GCVgY2F... create_v2 has 19 accounts, but accounts[16] is global idx 27 (ALT).
        // - 5HwZKTwc... create_v2 has 19 accounts, but accounts[16] is global idx 28 (ALT).
        // - 3MVawF6... create_v2 has 19 accounts, but accounts[16] is global idx 30 (ALT).
        // - 2dZAucK... create_v2 has 19 accounts, but accounts[16] is global idx 33 (ALT).
        // - oY9YQbie... create_v2 has 20 accounts, but accounts[16] is global idx 27 (ALT).
        // - 3jWGFYXT... create_v2 has 19 accounts, but accounts[16] is global idx 30 (ALT).
        // - 4h9kYjzY... create_v2 has 20 accounts, but accounts[16] is global idx 27 (ALT).
        //
        // ShredStream currently receives a VersionedTransaction and only has static_account_keys().
        // ALT-loaded addresses are not in that list, so the hot path must not invent USDC/WSOL.
        for (signature, name, static_len, program_idx, quote_global_idx, account_len) in [
            (
                "4GCVgY2FnT1s4q5zemnPL4mzSbuhUTgQo9mc9jewhLZzsCXKe8ehz6xD4QDJE853CLrF6doJbf4JNwJVeEYLA4De",
                "wsol 19-account quote in ALT",
                15usize,
                12u8,
                27u8,
                19usize,
            ),
            (
                "5HwZKTwcGFjSBPugSX5hE9JSq5wKmUooK3tLXuEoyDDzrTvHu7op3XDbhBXuteiC5EePNPh8TC1j6Fns47YvnyeG",
                "wsol 19-account quote in ALT exact quote buy",
                20usize,
                15u8,
                28u8,
                19usize,
            ),
            (
                "3MVawF6EPtG7rEPXdsyQfQUBLv3epRVNpNS4tRE4uwTPMqLNPqhuABwxU3QZH4uD6CuVupcpGchpNRK5HTbHRLNK",
                "usdc 19-account quote in ALT",
                19usize,
                16u8,
                30u8,
                19usize,
            ),
            (
                "oY9YQbie16Bw11GsqbAPVnW6YjMHAj3kP9sufjcuQjdfcU86iUY8CiSaDrvu4QXJFnGY4jqQc2Kc1YVuAzujvyv",
                "wsol 20-account quote in ALT",
                15usize,
                12u8,
                27u8,
                20usize,
            ),
            (
                "3jWGFYXT5V33Qc2roEBFDRAWHeybDowr53dSdnYSRkrPdYybU7oyEH9BfgSRxkgFHVKmUjv4e5T33AEnhJvBCuP2",
                "wsol 19-account quote in ALT with later buy",
                18usize,
                13u8,
                30u8,
                19usize,
            ),
            (
                "2dZAucKwr4n5Lqu3BtJ4P8JsjCDtUXJzthadddfURraEJRTgn6XWaTNUNBbgUfP5c2wcVdubqViQhr48eWsgRqPX",
                "usdc 19-account quote in ALT exact quote buy",
                19usize,
                15u8,
                33u8,
                19usize,
            ),
            (
                "4h9kYjzYpqqyYZuFnjf14zRwrGyChCuKAYVy6a4ZBig19bydEYsHwp6VbiKqTzT3pLf6NXnf6E25dn1NiU8LR4YB",
                "wsol 20-account quote in ALT with jit account",
                15usize,
                12u8,
                27u8,
                20usize,
            ),
        ] {
            let mut static_keys = vec![Pubkey::new_unique(); static_len];
            static_keys[program_idx as usize] = PROGRAM_ID_PUBKEY;
            let mut ix_accounts = ix_accounts(account_len);
            ix_accounts[15] = program_idx;
            ix_accounts[16] = quote_global_idx;
            let tx = v0_tx(program_idx, static_keys, ix_accounts, create_v2_data());

            let events = parse_best_effort_shred_events(&tx);

            assert_eq!(events.len(), 1, "{name}");
            match &events[0] {
                DexEvent::PumpFunCreate(event) => {
                    assert_eq!(event.ix_name, "create_v2", "{name}: {signature}");
                    assert_eq!(event.quote_mint, Pubkey::default(), "{name}: {signature}");
                    assert_eq!(event.quote_vault, Pubkey::default(), "{name}: {signature}");
                    assert_eq!(
                        event.quote_token_program,
                        Pubkey::default(),
                        "{name}: {signature}"
                    );
                }
                other => panic!("{name}: expected PumpFunCreate, got {other:?}"),
            }
        }
    }

    #[test]
    fn shred_pumpfun_create_v2_alt_quote_can_be_recovered_from_static_v2_trade() {
        let usdc = pk("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        let mint = Pubkey::new_unique();
        let mut static_keys = vec![Pubkey::new_unique(); 35];
        static_keys[0] = mint;
        static_keys[16] = PROGRAM_ID_PUBKEY;
        static_keys[30] = usdc;

        let mut create_accounts = ix_accounts(19);
        create_accounts[0] = 0;
        create_accounts[15] = 16;
        create_accounts[16] = 40; // ALT in create_v2, unavailable to Shred static-only parser.

        let mut buy_v2_accounts = ix_accounts(27);
        buy_v2_accounts[1] = 0; // mint
        buy_v2_accounts[2] = 30; // quote mint is static in the follow-up v2 trade.
        buy_v2_accounts[26] = 16;

        let tx = v0_tx_with_instructions(
            static_keys,
            vec![
                (16, create_accounts, create_v2_data()),
                (
                    16,
                    buy_v2_accounts,
                    instruction_data(discriminators::BUY_V2, 1, 2),
                ),
            ],
        );

        let events = parse_best_effort_shred_events(&tx);

        let create = events
            .iter()
            .find_map(|event| match event {
                DexEvent::PumpFunCreate(c) => Some(c),
                _ => None,
            })
            .expect("create event");
        assert_eq!(create.quote_mint, usdc);
    }

    #[test]
    fn shred_pumpfun_create_v2_alt_quote_stays_unknown_when_trade_quote_is_alt_too() {
        // Real compiled-index pattern from 3MVawF6... / 2dZAucK...:
        // create_v2 accounts[16] and follow-up buy_v2 accounts[2] both point to ALT-loaded USDC.
        let mint = Pubkey::new_unique();
        let mut static_keys = vec![Pubkey::new_unique(); 19];
        static_keys[0] = mint;
        static_keys[16] = PROGRAM_ID_PUBKEY;

        let mut create_accounts = ix_accounts(19);
        create_accounts[0] = 0;
        create_accounts[15] = 16;
        create_accounts[16] = 30;

        let mut buy_v2_accounts = ix_accounts(27);
        buy_v2_accounts[1] = 0;
        buy_v2_accounts[2] = 30;
        buy_v2_accounts[26] = 16;

        let tx = v0_tx_with_instructions(
            static_keys,
            vec![
                (16, create_accounts, create_v2_data()),
                (
                    16,
                    buy_v2_accounts,
                    instruction_data(discriminators::BUY_V2, 1, 2),
                ),
            ],
        );

        let events = parse_best_effort_shred_events(&tx);

        let create = events
            .iter()
            .find_map(|event| match event {
                DexEvent::PumpFunCreate(c) => Some(c),
                _ => None,
            })
            .expect("create event");
        assert_eq!(create.quote_mint, Pubkey::default());
    }

    #[test]
    fn strict_parser_skips_unresolved_program_but_best_effort_is_explicit() {
        let tx = v0_tx(
            30,
            unique_accounts(18),
            ix_accounts(18),
            instruction_data(discriminators::BUY, 100, 200),
        );
        let mut events = Vec::new();
        parse_transaction_dex_events(&tx, Signature::default(), 123, 0, 456, &mut events);
        assert!(events.is_empty());
        assert_eq!(parse_best_effort_shred_events(&tx).len(), 1);
    }

    #[test]
    fn strict_parser_skips_unresolved_accounts_and_preserves_creation_context() {
        for unknown_create_program in [false, true] {
            let mut keys = unique_accounts(19);
            keys[18] = PROGRAM_ID_PUBKEY;
            let mut create_accounts = ix_accounts(10);
            create_accounts[1] = 255;
            let tx = v0_tx_with_instructions(
                keys,
                vec![
                    (
                        if unknown_create_program { 255 } else { 18 },
                        create_accounts,
                        create_data(),
                    ),
                    (
                        18,
                        ix_accounts(18),
                        instruction_data(discriminators::BUY, 100, 200),
                    ),
                ],
            );
            let mut events = Vec::new();
            parse_transaction_dex_events(&tx, Signature::default(), 123, 7, 456, &mut events);
            assert_eq!(events.len(), 1);
            let DexEvent::PumpFunBuy(buy) = &events[0] else {
                panic!("buy")
            };
            // Legacy buy's mint is account 2; create's mint is account 0.
            assert!(!buy.is_created_buy);
            assert_eq!(buy.metadata.tx_index, 7);

            let mut matching = tx.clone();
            let VersionedMessage::V0(message) = &mut matching.message else {
                unreachable!()
            };
            message.instructions[0].accounts[0] = 2;
            events.clear();
            parse_transaction_dex_events(&matching, Signature::default(), 123, 7, 456, &mut events);
            assert_eq!(events.len(), 1);
            let DexEvent::PumpFunBuy(buy) = &events[0] else {
                panic!("buy")
            };
            assert_eq!(buy.is_created_buy, !unknown_create_program);
        }
    }

    #[test]
    fn shred_best_effort_parses_when_program_id_is_alt_loaded() {
        let static_keys = vec![PROGRAM_ID_PUBKEY; 2];
        let created_mint = static_keys[0];
        let tx = v0_tx(7, static_keys, ix_accounts(10), create_data());
        let mut events = Vec::new();

        parse_transaction_dex_events_best_effort(
            &tx,
            Signature::default(),
            123,
            0,
            456,
            None,
            &mut events,
        );

        assert_eq!(events.len(), 1);
        match &events[0] {
            DexEvent::PumpFunCreate(e) => assert_eq!(e.mint, created_mint),
            other => panic!("expected PumpFunCreate, got {other:?}"),
        }
    }

    #[test]
    fn unknown_program_without_filter_stops_after_first_matching_candidate() {
        let static_keys = vec![Pubkey::new_unique(); 18];
        let ix_accounts = ix_accounts(18);
        let mut data = Vec::new();
        data.extend_from_slice(&discriminators::BUY);
        data.extend_from_slice(&100_u64.to_le_bytes());
        data.extend_from_slice(&200_u64.to_le_bytes());
        let tx = v0_tx(30, static_keys, ix_accounts, data);
        let mut events = Vec::new();

        parse_transaction_dex_events_best_effort(
            &tx,
            Signature::default(),
            123,
            0,
            456,
            None,
            &mut events,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], DexEvent::PumpFunBuy(_)));
    }

    #[test]
    fn shred_pumpfun_sell_filter_covers_sell_v2() {
        let static_keys = vec![PROGRAM_ID_PUBKEY; 26];
        let filter =
            EventTypeFilter::include_only(vec![crate::grpc::types::EventType::PumpFunSell]);

        let mut data = Vec::new();
        data.extend_from_slice(&discriminators::SELL_V2);
        data.extend_from_slice(&100_u64.to_le_bytes());
        data.extend_from_slice(&200_u64.to_le_bytes());
        let mut events = Vec::new();

        dispatch_shred_outer(
            0,
            &ix_accounts(26),
            &data,
            &static_keys,
            Signature::default(),
            123,
            0,
            456,
            Some(&filter),
            &PumpMintSet::new(),
            &PumpMintSet::new(),
            &mut events,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], DexEvent::PumpFunSell(_)));
    }

    #[test]
    fn unknown_program_outer_uses_filter_to_parse_matching_protocol() {
        let static_keys = vec![RAYDIUM_CPMM_PROGRAM_ID, Pubkey::new_unique()];
        let ix_accounts = std::iter::once(1).chain(42..54).collect::<Vec<u8>>();
        let mut data = Vec::new();
        data.extend_from_slice(&crate::instr::raydium_cpmm::discriminators::SWAP_BASE_IN);
        data.extend_from_slice(&100_u64.to_le_bytes());
        data.extend_from_slice(&90_u64.to_le_bytes());
        let tx = v0_tx(9, static_keys, ix_accounts, data);
        let filter =
            EventTypeFilter::include_only(vec![crate::grpc::types::EventType::RaydiumCpmmSwap]);
        let mut events = Vec::new();

        parse_transaction_dex_events_best_effort(
            &tx,
            Signature::default(),
            123,
            0,
            456,
            Some(&filter),
            &mut events,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], DexEvent::RaydiumCpmmSwap(_)));
    }

    #[test]
    fn shred_pumpfun_trade_filter_normalizes_specific_variants_to_trade() {
        let static_keys = vec![PROGRAM_ID_PUBKEY; 27];
        let mut data = Vec::new();
        data.extend_from_slice(&discriminators::BUY_V2);
        data.extend_from_slice(&100_u64.to_le_bytes());
        data.extend_from_slice(&200_u64.to_le_bytes());
        let filter =
            EventTypeFilter::include_only(vec![crate::grpc::types::EventType::PumpFunTrade]);
        let mut events = Vec::new();

        dispatch_shred_outer(
            0,
            &ix_accounts(27),
            &data,
            &static_keys,
            Signature::default(),
            123,
            0,
            456,
            Some(&filter),
            &PumpMintSet::new(),
            &PumpMintSet::new(),
            &mut events,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], DexEvent::PumpFunTrade(_)));
    }

    #[test]
    fn shred_pumpfun_buy_filter_normalizes_all_buy_variants_to_buy() {
        let static_keys = vec![PROGRAM_ID_PUBKEY; 27];
        let filter = EventTypeFilter::include_only(vec![crate::grpc::types::EventType::PumpFunBuy]);

        let mut buy_data = Vec::new();
        buy_data.extend_from_slice(&discriminators::BUY_V2);
        buy_data.extend_from_slice(&100_u64.to_le_bytes());
        buy_data.extend_from_slice(&200_u64.to_le_bytes());
        let mut events = Vec::new();

        dispatch_shred_outer(
            0,
            &ix_accounts(27),
            &buy_data,
            &static_keys,
            Signature::default(),
            123,
            0,
            456,
            Some(&filter),
            &PumpMintSet::new(),
            &PumpMintSet::new(),
            &mut events,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], DexEvent::PumpFunBuy(_)));

        let mut exact_data = Vec::new();
        exact_data.extend_from_slice(&discriminators::BUY_EXACT_SOL_IN);
        exact_data.extend_from_slice(&100_u64.to_le_bytes());
        exact_data.extend_from_slice(&200_u64.to_le_bytes());
        events.clear();

        dispatch_shred_outer(
            0,
            &ix_accounts(27),
            &exact_data,
            &static_keys,
            Signature::default(),
            123,
            0,
            456,
            Some(&filter),
            &PumpMintSet::new(),
            &PumpMintSet::new(),
            &mut events,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], DexEvent::PumpFunBuy(_)));

        let mut exact_quote_data = Vec::new();
        exact_quote_data.extend_from_slice(&discriminators::BUY_EXACT_QUOTE_IN_V2);
        exact_quote_data.extend_from_slice(&100_u64.to_le_bytes());
        exact_quote_data.extend_from_slice(&200_u64.to_le_bytes());
        events.clear();

        dispatch_shred_outer(
            0,
            &ix_accounts(27),
            &exact_quote_data,
            &static_keys,
            Signature::default(),
            123,
            0,
            456,
            Some(&filter),
            &PumpMintSet::new(),
            &PumpMintSet::new(),
            &mut events,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], DexEvent::PumpFunBuy(_)));
    }

    #[test]
    fn non_pump_outer_accounts_keep_instruction_length_with_alt_defaults() {
        let static_keys = vec![RAYDIUM_CPMM_PROGRAM_ID, Pubkey::new_unique()];
        let ix_accounts = std::iter::once(1).chain(42..54).collect::<Vec<u8>>();
        let mut data = Vec::new();
        data.extend_from_slice(&crate::instr::raydium_cpmm::discriminators::SWAP_BASE_IN);
        data.extend_from_slice(&100_u64.to_le_bytes());
        data.extend_from_slice(&90_u64.to_le_bytes());
        let mut events = Vec::new();

        dispatch_shred_outer(
            0,
            &ix_accounts,
            &data,
            &static_keys,
            Signature::default(),
            123,
            0,
            456,
            None,
            &PumpMintSet::new(),
            &PumpMintSet::new(),
            &mut events,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], DexEvent::RaydiumCpmmSwap(_)));
    }

    #[test]
    fn non_pump_outer_skips_unsupported_discriminator_before_account_fill() {
        let static_keys = vec![RAYDIUM_CPMM_PROGRAM_ID, Pubkey::new_unique()];
        let ix_accounts = vec![99];
        let data = vec![0xff; 8];
        let mut events = Vec::new();

        dispatch_shred_outer(
            0,
            &ix_accounts,
            &data,
            &static_keys,
            Signature::default(),
            123,
            0,
            456,
            None,
            &PumpMintSet::new(),
            &PumpMintSet::new(),
            &mut events,
        );

        assert!(events.is_empty());
    }

    #[test]
    fn shred_pumpfun_trade_variants_are_specific_and_keep_exact_fields() {
        let accounts = unique_accounts(27);
        let no_created = PumpMintSet::new();
        let no_mayhem = PumpMintSet::new();

        let buy = parse_buy_instruction(
            &amount_data(100, 200),
            &accounts,
            Signature::default(),
            1,
            0,
            9,
            &no_created,
            &no_mayhem,
        )
        .expect("buy");
        match buy {
            DexEvent::PumpFunBuy(t) => {
                assert_eq!(t.bonding_curve_v2, accounts[16]);
                assert_eq!(t.buyback_fee_recipient, accounts[17]);
                assert_eq!(t.quote_mint, PUMPFUN_SOLSCAN_SOL_QUOTE_MINT);
            }
            other => panic!("expected buy variant, got {other:?}"),
        }

        let sell = parse_sell_instruction(
            &amount_data(300, 400),
            &accounts,
            Signature::default(),
            1,
            0,
            9,
        )
        .expect("sell");
        match sell {
            DexEvent::PumpFunSell(t) => {
                assert_eq!(t.user_volume_accumulator, accounts[14]);
                assert_eq!(t.bonding_curve_v2, accounts[15]);
                assert_eq!(t.buyback_fee_recipient, accounts[16]);
                assert_eq!(t.quote_mint, PUMPFUN_SOLSCAN_SOL_QUOTE_MINT);
            }
            other => panic!("expected sell variant, got {other:?}"),
        }

        let exact_quote = parse_buy_exact_quote_in_v2_instruction(
            &amount_data(500, 600),
            &accounts,
            Signature::default(),
            1,
            0,
            9,
            &no_created,
            &no_mayhem,
        )
        .expect("exact quote buy");

        match exact_quote {
            DexEvent::PumpFunBuy(t) => {
                assert_eq!(t.ix_name, "buy_exact_quote_in_v2");
                assert_eq!(t.spendable_quote_in, 500);
                assert_eq!(t.min_tokens_out, 600);
                assert_eq!(t.max_sol_cost, 0);
                assert_eq!(t.quote_amount, 500);
                assert_eq!(t.quote_mint, accounts[2]);
                assert_eq!(t.associated_quote_user, accounts[15]);
                assert_eq!(t.associated_creator_vault, accounts[17]);
                assert_eq!(t.sharing_config, accounts[18]);
                assert_eq!(t.global_volume_accumulator, accounts[19]);
                assert_eq!(t.fee_program, accounts[23]);
            }
            other => panic!("expected exact buy variant, got {other:?}"),
        }
    }

    #[test]
    fn shred_pumpfun_v2_buy_parses_short_account_lists_best_effort() {
        let accounts = unique_accounts(16);
        let no_created = PumpMintSet::new();
        let no_mayhem = PumpMintSet::new();

        let buy = parse_buy_v2_instruction(
            &amount_data(100, 200),
            &accounts,
            Signature::default(),
            1,
            0,
            9,
            &no_created,
            &no_mayhem,
        )
        .expect("short buy_v2");
        match buy {
            DexEvent::PumpFunBuy(t) => {
                assert_eq!(t.ix_name, "buy_v2");
                assert_eq!(t.mint, accounts[1]);
                assert_eq!(t.amount, 100);
                assert_eq!(t.max_sol_cost, 200);
                assert_eq!(t.associated_creator_vault, Pubkey::default());
                assert_eq!(t.fee_program, Pubkey::default());
            }
            other => panic!("expected buy variant, got {other:?}"),
        }

        let exact_quote = parse_buy_exact_quote_in_v2_instruction(
            &amount_data(500, 600),
            &accounts,
            Signature::default(),
            1,
            0,
            9,
            &no_created,
            &no_mayhem,
        )
        .expect("short exact quote buy");
        match exact_quote {
            DexEvent::PumpFunBuy(t) => {
                assert_eq!(t.ix_name, "buy_exact_quote_in_v2");
                assert_eq!(t.mint, accounts[1]);
                assert_eq!(t.spendable_quote_in, 500);
                assert_eq!(t.min_tokens_out, 600);
                assert_eq!(t.quote_amount, 500);
                assert_eq!(t.associated_creator_vault, Pubkey::default());
                assert_eq!(t.fee_program, Pubkey::default());
            }
            other => panic!("expected exact buy variant, got {other:?}"),
        }
    }

    #[test]
    fn shred_pumpfun_v2_sell_parses_short_account_lists_best_effort() {
        let accounts = unique_accounts(16);

        let sell = parse_sell_v2_instruction(
            &amount_data(300, 400),
            &accounts,
            Signature::default(),
            1,
            0,
            9,
        )
        .expect("short sell_v2");

        match sell {
            DexEvent::PumpFunSell(t) => {
                assert_eq!(t.ix_name, "sell_v2");
                assert_eq!(t.mint, accounts[1]);
                assert_eq!(t.amount, 300);
                assert_eq!(t.min_sol_output, 400);
                assert_eq!(t.associated_creator_vault, Pubkey::default());
                assert_eq!(t.fee_program, Pubkey::default());
            }
            other => panic!("expected sell variant, got {other:?}"),
        }
    }

    #[test]
    fn shred_pumpfun_legacy_trade_rejects_short_account_lists() {
        let accounts = unique_accounts(16);
        let no_created = PumpMintSet::new();
        let no_mayhem = PumpMintSet::new();

        assert!(parse_buy_instruction(
            &amount_data(100, 200),
            &accounts[..15],
            Signature::default(),
            1,
            0,
            9,
            &no_created,
            &no_mayhem,
        )
        .is_none());

        assert!(parse_sell_instruction(
            &amount_data(300, 400),
            &accounts[..13],
            Signature::default(),
            1,
            0,
            9,
        )
        .is_none());
    }
    fn aligned_outer_events(
        program: Pubkey,
        accounts: Vec<Pubkey>,
        data: Vec<u8>,
    ) -> Vec<DexEvent> {
        let count = accounts.len();
        let mut keys = accounts;
        keys.push(program);
        let tx = v0_tx(count as u8, keys, (0..count as u8).collect(), data);
        let mut events = Vec::new();
        parse_transaction_dex_events(&tx, Signature::default(), 42, 3, 1234, &mut events);
        assert_eq!(events.len(), 1);
        let meta = events[0].metadata();
        assert_eq!(meta.grpc_recv_us, 1234);
        assert_eq!(
            meta.recent_blockhash.as_deref(),
            Some(tx.message.recent_blockhash().to_string().as_str())
        );
        events
    }

    #[test]
    fn shred_dlmm_swap_variants_keep_threshold_and_instruction_accounts() {
        use crate::instr::meteora_dlmm::discriminators as dlmm;
        for (disc, v2, exact_out, impact) in [
            (dlmm::SWAP, false, false, false),
            (dlmm::SWAP2, true, false, false),
            (dlmm::SWAP_EXACT_OUT, false, true, false),
            (dlmm::SWAP_EXACT_OUT2, true, true, false),
            (dlmm::SWAP_WITH_PRICE_IMPACT, false, false, true),
            (dlmm::SWAP_WITH_PRICE_IMPACT2, true, false, true),
        ] {
            let mut accounts: Vec<_> = (0..20).map(|_| Pubkey::new_unique()).collect();
            if v2 {
                accounts[13] = solana_sdk::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
            }
            let program_idx = if v2 { 15 } else { 14 };
            accounts[program_idx] = METEORA_DLMM_PROGRAM_ID;
            // An unused optional bitmap extension uses the program id placeholder.
            accounts[1] = METEORA_DLMM_PROGRAM_ID;
            let mut data = disc.to_vec();
            data.extend_from_slice(&500u64.to_le_bytes());
            if impact {
                data.push(0);
                data.extend_from_slice(&25u16.to_le_bytes());
            } else {
                data.extend_from_slice(&400u64.to_le_bytes());
            }
            if v2 {
                data.extend_from_slice(&0u32.to_le_bytes());
            }
            let events = aligned_outer_events(METEORA_DLMM_PROGRAM_ID, accounts.clone(), data);
            let DexEvent::MeteoraDlmmSwap(e) = &events[0] else {
                panic!("DLMM swap")
            };
            assert_eq!(e.pool, accounts[0]);
            assert_eq!(
                (e.user_token_in, e.user_token_out),
                (accounts[4], accounts[5])
            );
            assert_eq!((e.token_x_mint, e.token_y_mint), (accounts[6], accounts[7]));
            assert_eq!(
                (e.reserve_x, e.reserve_y, e.oracle),
                (accounts[2], accounts[3], accounts[8])
            );
            assert_eq!(
                (e.token_x_program, e.token_y_program),
                (accounts[11], accounts[12])
            );
            assert_eq!(e.bin_arrays, accounts[program_idx + 1..]);
            assert_eq!(e.bitmap_extension, None);
            assert_eq!(e.amount_in, 500);
            assert_eq!(e.min_amount_out, if exact_out || impact { 0 } else { 400 });
            assert_eq!(e.amount_out, if exact_out { 400 } else { 0 });
        }
    }

    #[test]
    fn shred_orca_swap_versions_fill_their_own_account_layouts() {
        use crate::instr::orca_whirlpool::discriminators as orca;
        for (disc, v2) in [(orca::SWAP, false), (orca::SWAP_V2, true)] {
            let mut accounts: Vec<_> = (0..15).map(|_| Pubkey::new_unique()).collect();
            if v2 {
                accounts[2] = solana_sdk::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
            }
            let mut data = disc.to_vec();
            data.extend_from_slice(&500u64.to_le_bytes());
            data.extend_from_slice(&400u64.to_le_bytes());
            data.extend_from_slice(&123u128.to_le_bytes());
            data.extend_from_slice(&[1, 0]);
            let events = aligned_outer_events(ORCA_WHIRLPOOL_PROGRAM_ID, accounts.clone(), data);
            let DexEvent::OrcaWhirlpoolSwap(e) = &events[0] else {
                panic!("Orca swap")
            };
            assert_eq!(e.whirlpool, accounts[if v2 { 4 } else { 2 }]);
            assert_eq!(e.token_program_a, accounts[0]);
            assert_eq!(e.token_program_b, accounts[if v2 { 1 } else { 0 }]);
            assert_eq!(e.token_vault_a, accounts[if v2 { 8 } else { 4 }]);
            assert_eq!(e.token_vault_b, accounts[if v2 { 10 } else { 6 }]);
            assert_eq!(e.tick_array_0, accounts[if v2 { 11 } else { 7 }]);
            assert_eq!(e.tick_array_1, accounts[if v2 { 12 } else { 8 }]);
            assert_eq!(e.tick_array_2, accounts[if v2 { 13 } else { 9 }]);
            assert_eq!(e.oracle, accounts[if v2 { 14 } else { 10 }]);
            if v2 {
                assert_eq!((e.token_mint_a, e.token_mint_b), (accounts[5], accounts[6]));
            } else {
                assert_eq!(
                    (e.token_mint_a, e.token_mint_b),
                    (Pubkey::default(), Pubkey::default())
                );
            }
        }
    }

    #[test]
    fn shred_clmm_quantity_mode_does_not_invent_direction_and_keeps_remaining_accounts() {
        use crate::core::account_fillers::raydium::tick_array_bitmap_extension_pda;
        use crate::instr::raydium_clmm::discriminators as clmm;
        for (disc, v2) in [(clmm::SWAP, false), (clmm::SWAP_V2, true)] {
            for exact_in in [false, true] {
                let mut accounts: Vec<_> = (0..16).map(|_| Pubkey::new_unique()).collect();
                if v2 {
                    accounts[10] =
                        solana_sdk::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
                }
                let remaining_start = if v2 { 13 } else { 9 };
                accounts[remaining_start + 1] = tick_array_bitmap_extension_pda(&accounts[2]);
                let mut data = disc.to_vec();
                data.extend_from_slice(&500u64.to_le_bytes());
                data.extend_from_slice(&400u64.to_le_bytes());
                data.extend_from_slice(&123u128.to_le_bytes());
                data.push(u8::from(exact_in));
                let events = aligned_outer_events(RAYDIUM_CLMM_PROGRAM_ID, accounts.clone(), data);
                let DexEvent::RaydiumClmmSwap(e) = &events[0] else {
                    panic!("CLMM swap")
                };
                assert!(!e.zero_for_one);
                assert_eq!(e.amm_config, accounts[1]);
                assert_eq!(
                    (e.input_vault, e.output_vault, e.observation_state),
                    (accounts[5], accounts[6], accounts[7])
                );
                assert_eq!(
                    e.tick_array_bitmap_extension,
                    Some(accounts[remaining_start + 1])
                );
                let expected_ticks: Vec<_> = accounts[remaining_start..]
                    .iter()
                    .copied()
                    .filter(|k| *k != accounts[remaining_start + 1])
                    .collect();
                assert_eq!(e.tick_arrays, expected_ticks);
                if v2 {
                    assert_eq!((e.input_mint, e.output_mint), (accounts[11], accounts[12]));
                }
                // Execution amounts cannot be recovered from instruction limits.
                assert_eq!((e.amount_0, e.amount_1), (0, 0));
            }
        }
    }

    #[test]
    fn shred_cpmm_swap_versions_fill_account_context_without_execution_amounts() {
        use crate::instr::raydium_cpmm::discriminators as cpmm;
        for (disc, base_input) in [(cpmm::SWAP_BASE_IN, true), (cpmm::SWAP_BASE_OUT, false)] {
            let accounts: Vec<_> = (0..13).map(|_| Pubkey::new_unique()).collect();
            let mut data = disc.to_vec();
            data.extend_from_slice(&500u64.to_le_bytes());
            data.extend_from_slice(&400u64.to_le_bytes());
            let events = aligned_outer_events(RAYDIUM_CPMM_PROGRAM_ID, accounts.clone(), data);
            let DexEvent::RaydiumCpmmSwap(e) = &events[0] else {
                panic!("CPMM swap")
            };
            assert_eq!(e.pool_id, accounts[3]);
            assert_eq!(e.amm_config, accounts[2]);
            assert_eq!((e.input_vault, e.output_vault), (accounts[6], accounts[7]));
            assert_eq!(
                (e.input_token_program, e.output_token_program),
                (accounts[8], accounts[9])
            );
            assert_eq!(
                (e.input_token_mint, e.output_token_mint, e.observation_state),
                (accounts[10], accounts[11], accounts[12])
            );
            assert_eq!(e.base_input, base_input);
            assert_eq!((e.input_amount, e.output_amount), (0, 0));
        }
    }

    #[test]
    fn shred_multi_pool_accounts_and_metadata_are_transaction_local() {
        use crate::instr::meteora_dlmm::discriminators as dlmm;
        let mut keys: Vec<_> = (0..40).map(|_| Pubkey::new_unique()).collect();
        keys.push(METEORA_DLMM_PROGRAM_ID);
        let mut data = dlmm::SWAP.to_vec();
        data.extend_from_slice(&500u64.to_le_bytes());
        data.extend_from_slice(&400u64.to_le_bytes());
        let tx = v0_tx_with_instructions(
            keys.clone(),
            vec![
                (40, (0..20).collect(), data.clone()),
                (40, (20..40).collect(), data),
            ],
        );
        let mut previous = crate::core::events::MeteoraDlmmSwapEvent::default();
        previous.metadata.grpc_recv_us = 77;
        previous.metadata.recent_blockhash = Some("previous".into());
        let mut events = vec![DexEvent::MeteoraDlmmSwap(previous)];
        parse_transaction_dex_events(&tx, Signature::default(), 42, 3, 1234, &mut events);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].metadata().grpc_recv_us, 77);
        assert_eq!(
            events[0].metadata().recent_blockhash.as_deref(),
            Some("previous")
        );
        for (idx, offset) in [(1, 0), (2, 20)] {
            let DexEvent::MeteoraDlmmSwap(e) = &events[idx] else {
                panic!("DLMM swap")
            };
            assert_eq!(e.pool, keys[offset]);
            assert_eq!(
                (e.user_token_in, e.token_x_mint),
                (keys[offset + 4], keys[offset + 6])
            );
        }
    }

    #[test]
    fn resolved_alt_addresses_keep_writable_readonly_order_and_reject_bad_counts() {
        use crate::instr::meteora_dlmm::discriminators as dlmm;
        let mut accounts: Vec<_> = (0..16).map(|_| Pubkey::new_unique()).collect();
        let mint = Pubkey::new_unique();
        let mut data = dlmm::SWAP2.to_vec();
        data.extend_from_slice(&500u64.to_le_bytes());
        data.extend_from_slice(&400u64.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        accounts[13] = solana_sdk::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
        let mut indices: Vec<u8> = (0..16).collect();
        indices[6] = 16; // first loaded writable address
        indices[15] = 17; // first loaded readonly address (the program)
        let mut tx = v0_tx(17, accounts, indices, data);
        let VersionedMessage::V0(message) = &mut tx.message else {
            panic!("v0")
        };
        message
            .address_table_lookups
            .push(v0::MessageAddressTableLookup {
                account_key: Pubkey::new_unique(),
                writable_indexes: vec![5],
                readonly_indexes: vec![9],
            });
        let mut events = Vec::new();
        parse_transaction_dex_events_with_loaded_addresses(
            &tx,
            &[mint],
            &[METEORA_DLMM_PROGRAM_ID],
            Signature::default(),
            42,
            3,
            1234,
            None,
            &mut events,
        )
        .unwrap();
        let DexEvent::MeteoraDlmmSwap(e) = &events[0] else {
            panic!("DLMM")
        };
        assert_eq!(e.token_x_mint, mint);
        let original = serde_json::to_value(&events).unwrap();
        assert!(parse_transaction_dex_events_with_loaded_addresses(
            &tx,
            &[],
            &[METEORA_DLMM_PROGRAM_ID],
            Signature::default(),
            42,
            3,
            1234,
            None,
            &mut events,
        )
        .is_err());
        assert_eq!(serde_json::to_value(&events).unwrap(), original);
        let VersionedMessage::V0(message) = &mut tx.message else {
            panic!("v0")
        };
        message.instructions[0].accounts[7] = 99;
        assert!(parse_transaction_dex_events_with_loaded_addresses(
            &tx,
            &[mint],
            &[METEORA_DLMM_PROGRAM_ID],
            Signature::default(),
            42,
            3,
            1234,
            None,
            &mut events,
        )
        .is_err());
        assert_eq!(serde_json::to_value(&events).unwrap(), original);
    }
    #[test]
    fn shred_pumpfun_create_fills_legacy_accounts_and_buys_keep_track_volume() {
        let accounts: Vec<_> = (0..18).map(|_| Pubkey::new_unique()).collect();
        let mut create_data = discriminators::CREATE.to_vec();
        str_arg("Parity", &mut create_data);
        str_arg("PAR", &mut create_data);
        str_arg("https://example.invalid/parity", &mut create_data);
        create_data.extend_from_slice(accounts[7].as_ref());
        let events = aligned_outer_events(PROGRAM_ID_PUBKEY, accounts[..14].to_vec(), create_data);
        let DexEvent::PumpFunCreate(e) = &events[0] else {
            panic!("create")
        };
        assert_eq!(e.mint_authority, accounts[1]);
        assert_eq!(e.associated_bonding_curve, accounts[3]);
        assert_eq!(e.global, accounts[4]);
        assert_eq!(e.associated_token_program, accounts[10]);
        assert_eq!(e.event_authority, accounts[12]);
        assert_eq!(e.program, accounts[13]);
        for disc in [discriminators::BUY, discriminators::BUY_EXACT_SOL_IN] {
            for flag in [0u8, 1] {
                let mut data = disc.to_vec();
                data.extend_from_slice(&500u64.to_le_bytes());
                data.extend_from_slice(&400u64.to_le_bytes());
                data.push(flag);
                let events = aligned_outer_events(PROGRAM_ID_PUBKEY, accounts.clone(), data);
                let trade = match &events[0] {
                    DexEvent::PumpFunBuy(e) | DexEvent::PumpFunBuyExactSolIn(e) => e,
                    _ => panic!("buy"),
                };
                assert_eq!(trade.track_volume, flag != 0);
            }
        }
    }
    #[test]
    fn strict_v2_accounts_require_full_layout_while_best_effort_remains_available() {
        for (disc, minimum) in [
            (discriminators::BUY_V2, 27),
            (discriminators::BUY_EXACT_QUOTE_IN_V2, 27),
            (discriminators::SELL_V2, 26),
        ] {
            for count in [minimum - 1, minimum] {
                let mut keys = unique_accounts(count + 1);
                keys[count] = PROGRAM_ID_PUBKEY;
                let tx = v0_tx(
                    count as u8,
                    keys,
                    ix_accounts(count),
                    instruction_data(disc, 100, 200),
                );
                let mut events = Vec::new();
                parse_transaction_dex_events(&tx, Signature::default(), 1, 0, 0, &mut events);
                assert_eq!(events.len(), usize::from(count == minimum));
                assert_eq!(parse_best_effort_shred_events(&tx).len(), 1);
            }
        }
    }

    #[test]
    fn legacy_create_requires_complete_creator_argument() {
        let data = create_data();
        let accounts = unique_accounts(10);
        assert!(
            parse_create_instruction(&data, &accounts, Signature::default(), 1, 0, 0).is_some()
        );
        assert!(parse_create_instruction(
            &data[..data.len() - 1],
            &accounts,
            Signature::default(),
            1,
            0,
            0
        )
        .is_none());
    }
}

#[cfg(test)]
mod review_create_regressions {
    use super::*;

    #[test]
    fn create_v2_partial_fee_and_bad_strings_are_rejected_by_both_entries() {
        let accounts: Vec<_> = (0..16).map(|_| Pubkey::new_unique()).collect();
        let mut valid = discriminators::CREATE_V2.to_vec();
        for value in ["a", "b", "c"] {
            valid.extend_from_slice(&(value.len() as u32).to_le_bytes());
            valid.extend_from_slice(value.as_bytes());
        }
        valid.extend_from_slice(Pubkey::new_unique().as_ref());
        valid.extend_from_slice(&[0, 0]);
        let parse_both = |data: &[u8]| {
            (
                crate::instr::pump::parse_instruction(
                    data,
                    &accounts,
                    Signature::default(),
                    1,
                    0,
                    None,
                    0,
                )
                .is_some(),
                parse_pumpfun_instruction(
                    data,
                    &accounts,
                    Signature::default(),
                    1,
                    0,
                    0,
                    &PumpMintSet::new(),
                    &PumpMintSet::new(),
                )
                .is_some(),
            )
        };
        assert_eq!(parse_both(&valid), (true, true));
        for length in 1..8 {
            let mut partial = valid.clone();
            partial.extend(std::iter::repeat_n(1, length));
            assert_eq!(parse_both(&partial), (false, false));
        }
        let mut invalid_mayhem = valid.clone();
        let mayhem_offset = invalid_mayhem.len() - 2;
        invalid_mayhem[mayhem_offset] = 2;
        assert_eq!(parse_both(&invalid_mayhem), (false, false));
        assert!(crate::instr::utils::parse_create_v2_tail_fields(&invalid_mayhem[8..]).is_none());
        let mut invalid_utf8 = valid.clone();
        invalid_utf8[12] = 255;
        assert_eq!(parse_both(&invalid_utf8), (false, false));
        let mut invalid_length = valid;
        invalid_length[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse_both(&invalid_length), (false, false));
    }
}

#[cfg(test)]
mod review_trade_payload_tests {
    use super::*;

    #[test]
    fn strict_pump_trade_payload_rejects_truncation_and_invalid_track_volume() {
        for (disc, count, optional_track) in [
            (discriminators::BUY, 16, true),
            (discriminators::SELL, 14, false),
            (discriminators::BUY_EXACT_SOL_IN, 16, true),
            (discriminators::BUY_V2, 27, false),
            (discriminators::SELL_V2, 26, false),
            (discriminators::BUY_EXACT_QUOTE_IN_V2, 27, false),
        ] {
            let accounts: Vec<_> = (0..count).map(|_| Pubkey::new_unique()).collect();
            for len in 0..16 {
                let mut data = disc.to_vec();
                data.resize(8 + len, 0);
                assert!(
                    parse_pumpfun_instruction(
                        &data,
                        &accounts,
                        Signature::default(),
                        1,
                        0,
                        0,
                        &PumpMintSet::new(),
                        &PumpMintSet::new()
                    )
                    .is_none(),
                    "short payload emitted trade for {disc:?}, length {len}"
                );
            }
            let mut data = disc.to_vec();
            data.resize(24, 0);
            assert!(parse_pumpfun_instruction(
                &data,
                &accounts,
                Signature::default(),
                1,
                0,
                0,
                &PumpMintSet::new(),
                &PumpMintSet::new()
            )
            .is_some());
            if optional_track {
                for value in [0, 1, 2, 255] {
                    let mut candidate = data.clone();
                    candidate.push(value);
                    assert_eq!(
                        parse_pumpfun_instruction(
                            &candidate,
                            &accounts,
                            Signature::default(),
                            1,
                            0,
                            0,
                            &PumpMintSet::new(),
                            &PumpMintSet::new()
                        )
                        .is_some(),
                        value <= 1
                    );
                }
            }
        }
    }
}
