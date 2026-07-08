//! Raydium CPMM 指令解析器
//!
//! 使用 match discriminator 模式解析 Raydium CPMM 指令

use super::program_ids;
use super::utils::*;
use crate::core::events::*;
use solana_sdk::{pubkey::Pubkey, signature::Signature};

/// Raydium CPMM discriminator 常量
pub mod discriminators {
    pub const SWAP_BASE_IN: [u8; 8] = [143, 190, 90, 218, 196, 30, 51, 222];
    pub const SWAP_BASE_OUT: [u8; 8] = [55, 217, 98, 86, 163, 74, 180, 173];
    pub const INITIALIZE: [u8; 8] = [175, 175, 109, 31, 13, 152, 155, 237];
    pub const DEPOSIT: [u8; 8] = [242, 35, 198, 137, 82, 225, 242, 182];
    pub const WITHDRAW: [u8; 8] = [183, 18, 70, 156, 148, 109, 161, 34];
}

/// Raydium CPMM 程序 ID
pub const PROGRAM_ID_PUBKEY: Pubkey = program_ids::RAYDIUM_CPMM_PROGRAM_ID;

/// 主要的 Raydium CPMM 指令解析函数
pub fn parse_instruction(
    instruction_data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    block_time_us: Option<i64>,
) -> Option<DexEvent> {
    if instruction_data.len() < 8 {
        return None;
    }

    let discriminator: [u8; 8] = instruction_data[0..8].try_into().ok()?;
    let data = &instruction_data[8..];

    match discriminator {
        discriminators::SWAP_BASE_IN => {
            parse_swap_base_in_instruction(data, accounts, signature, slot, tx_index, block_time_us)
        }
        discriminators::SWAP_BASE_OUT => parse_swap_base_out_instruction(
            data,
            accounts,
            signature,
            slot,
            tx_index,
            block_time_us,
        ),
        discriminators::INITIALIZE => {
            parse_initialize_instruction(data, accounts, signature, slot, tx_index, block_time_us)
        }
        discriminators::DEPOSIT => {
            parse_deposit_instruction(data, accounts, signature, slot, tx_index, block_time_us)
        }
        discriminators::WITHDRAW => {
            parse_withdraw_instruction(data, accounts, signature, slot, tx_index, block_time_us)
        }
        _ => None,
    }
}

/// 解析 Base In 交换指令
fn parse_swap_base_in_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    block_time_us: Option<i64>,
) -> Option<DexEvent> {
    let mut offset = 0;

    let _amount_in = read_u64_le(data, offset)?;
    offset += 8;

    let _minimum_amount_out = read_u64_le(data, offset)?;

    let pool = get_account(accounts, 0)?;
    let metadata = create_metadata_simple(signature, slot, tx_index, block_time_us, pool);

    Some(DexEvent::RaydiumCpmmSwap(RaydiumCpmmSwapEvent {
        metadata,

        // IDL SwapEvent 事件字段 - 从日志填充，这里设置默认值
        pool_id: Pubkey::default(), // 将从日志填充
        input_vault_before: 0,      // 将从日志填充
        output_vault_before: 0,     // 将从日志填充
        input_amount: 0,            // 将从日志填充
        output_amount: 0,           // 将从日志填充
        input_transfer_fee: 0,      // 将从日志填充
        output_transfer_fee: 0,     // 将从日志填充
        base_input: true,
        // 暂时注释，以后会用，AI禁止改动
        // 指令参数字段
        // amount_in,
        // minimum_amount_out,

        // 指令账户字段 - 从account_filler填充
        // payer: Pubkey::default(),
        // authority: Pubkey::default(),
        // amm_config: Pubkey::default(),
        // pool_state: Pubkey::default(),
        // input_token_account: Pubkey::default(),
        // output_token_account: Pubkey::default(),
        // input_vault: Pubkey::default(),
        // output_vault: Pubkey::default(),
        // input_token_mint: Pubkey::default(),
        // output_token_mint: Pubkey::default(),
    }))
}

/// 解析 Base Out 交换指令
fn parse_swap_base_out_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    block_time_us: Option<i64>,
) -> Option<DexEvent> {
    let mut offset = 0;

    let _maximum_amount_in = read_u64_le(data, offset)?;
    offset += 8;

    let _amount_out = read_u64_le(data, offset)?;

    let pool = get_account(accounts, 0)?;
    let metadata = create_metadata_simple(signature, slot, tx_index, block_time_us, pool);

    Some(DexEvent::RaydiumCpmmSwap(RaydiumCpmmSwapEvent {
        metadata,

        // IDL SwapEvent 事件字段 - 从日志填充，这里设置默认值
        pool_id: Pubkey::default(), // 将从日志填充
        input_vault_before: 0,      // 将从日志填充
        output_vault_before: 0,     // 将从日志填充
        input_amount: 0,            // 将从日志填充
        output_amount: 0,           // 将从日志填充
        input_transfer_fee: 0,      // 将从日志填充
        output_transfer_fee: 0,     // 将从日志填充
        base_input: false,
        // 暂时注释，以后会用，AI禁止改动
        // 指令参数字段
        // amount_in: maximum_amount_in,
        // minimum_amount_out: amount_out,

        // 指令账户字段 - 从account_filler填充
        // payer: Pubkey::default(),
        // authority: Pubkey::default(),
        // amm_config: Pubkey::default(),
        // pool_state: Pubkey::default(),
        // input_token_account: Pubkey::default(),
        // output_token_account: Pubkey::default(),
        // input_vault: Pubkey::default(),
        // output_vault: Pubkey::default(),
        // input_token_mint: Pubkey::default(),
        // output_token_mint: Pubkey::default(),
    }))
}

/// 解析初始化指令
fn parse_initialize_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    block_time_us: Option<i64>,
) -> Option<DexEvent> {
    let mut offset = 0;

    let init_amount0 = read_u64_le(data, offset)?;
    offset += 8;

    let init_amount1 = read_u64_le(data, offset)?;
    offset += 8;

    let _open_time = read_u64_le(data, offset)?;

    let pool = get_account(accounts, 0)?;
    let metadata = create_metadata_simple(signature, slot, tx_index, block_time_us, pool);

    Some(DexEvent::RaydiumCpmmInitialize(
        RaydiumCpmmInitializeEvent {
            metadata,
            pool,
            creator: get_account(accounts, 1).unwrap_or_default(),
            init_amount0,
            init_amount1,
        },
    ))
}

/// 解析存款指令
fn parse_deposit_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    block_time_us: Option<i64>,
) -> Option<DexEvent> {
    let mut offset = 0;

    let lp_token_amount = read_u64_le(data, offset)?;
    offset += 8;

    let maximum_token_0_amount = read_u64_le(data, offset)?;
    offset += 8;

    let maximum_token_1_amount = read_u64_le(data, offset)?;

    let pool = get_account(accounts, 0)?;
    let metadata = create_metadata_simple(signature, slot, tx_index, block_time_us, pool);

    Some(DexEvent::RaydiumCpmmDeposit(RaydiumCpmmDepositEvent {
        metadata,
        pool,
        user: get_account(accounts, 1).unwrap_or_default(),
        lp_token_amount,
        token0_amount: maximum_token_0_amount, // 先赋值为maximum，logs会覆盖
        token1_amount: maximum_token_1_amount, // 先赋值为maximum，logs会覆盖
    }))
}

/// 解析提款指令
fn parse_withdraw_instruction(
    data: &[u8],
    accounts: &[Pubkey],
    signature: Signature,
    slot: u64,
    tx_index: u64,
    block_time_us: Option<i64>,
) -> Option<DexEvent> {
    let mut offset = 0;

    let lp_token_amount = read_u64_le(data, offset)?;
    offset += 8;

    let minimum_token_0_amount = read_u64_le(data, offset)?;
    offset += 8;

    let minimum_token_1_amount = read_u64_le(data, offset)?;

    let pool = get_account(accounts, 0)?;
    let metadata = create_metadata_simple(signature, slot, tx_index, block_time_us, pool);

    Some(DexEvent::RaydiumCpmmWithdraw(RaydiumCpmmWithdrawEvent {
        metadata,
        pool,
        user: get_account(accounts, 1).unwrap_or_default(),
        lp_token_amount,
        token0_amount: minimum_token_0_amount, // 先赋值为minimum，logs会覆盖
        token1_amount: minimum_token_1_amount, // 先赋值为minimum，logs会覆盖
    }))
}
