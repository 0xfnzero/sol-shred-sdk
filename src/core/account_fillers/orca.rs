//! Orca Whirlpool 账户填充模块

use crate::core::events::*;
use solana_sdk::pubkey::Pubkey;

pub type AccountGetter<'a> = dyn Fn(usize) -> Pubkey + 'a;

/// Orca Whirlpool Swap 账户填充
///
/// swap instruction account mapping (based on IDL):
/// 0: tokenProgram
/// 1: tokenAuthority
/// 2: whirlpool
/// 3: tokenOwnerAccountA
/// 4: tokenVaultA
/// 5: tokenOwnerAccountB
/// 6: tokenVaultB
/// 7: tickArray0
/// 8: tickArray1
/// 9: tickArray2
/// 10: oracle
pub fn fill_whirlpool_swap_accounts(_e: &mut OrcaWhirlpoolSwapEvent, _get: &AccountGetter<'_>) {
    // whirlpool, input_amount, output_amount, a_to_b 已从事件数据解析
    // 其他 skip 字段（pre_sqrt_price, post_sqrt_price 等）需要从日志或链上数据获取
    // 不需要从指令账户填充
}

/// Orca Whirlpool Liquidity Increased 账户填充
///
/// increaseLiquidity instruction account mapping (based on IDL):
/// 0: whirlpool
/// 1: tokenProgram
/// 2: positionAuthority
/// 3: position
/// 4: positionTokenAccount
/// 5: tokenOwnerAccountA
/// 6: tokenOwnerAccountB
/// 7: tokenVaultA
/// 8: tokenVaultB
/// 9: tickArrayLower
/// 10: tickArrayUpper
pub fn fill_whirlpool_liquidity_increased_accounts(
    e: &mut OrcaWhirlpoolLiquidityIncreasedEvent,
    get: &AccountGetter<'_>,
) {
    if e.position == Pubkey::default() {
        e.position = get(3);
    }
    // tick_lower_index, tick_upper_index 等需要从链上 position 账户数据读取
    // 不能直接从指令账户填充
}

/// Orca Whirlpool Liquidity Decreased 账户填充
///
/// decreaseLiquidity instruction account mapping (based on IDL):
/// 0: whirlpool
/// 1: tokenProgram
/// 2: positionAuthority
/// 3: position
/// 4: positionTokenAccount
/// 5: tokenOwnerAccountA
/// 6: tokenOwnerAccountB
/// 7: tokenVaultA
/// 8: tokenVaultB
/// 9: tickArrayLower
/// 10: tickArrayUpper
pub fn fill_whirlpool_liquidity_decreased_accounts(
    e: &mut OrcaWhirlpoolLiquidityDecreasedEvent,
    get: &AccountGetter<'_>,
) {
    if e.position == Pubkey::default() {
        e.position = get(3);
    }
    // tick_lower_index, tick_upper_index 等需要从链上 position 账户数据读取
    // 不能直接从指令账户填充
}
