//! Orca Whirlpool 账户填充模块

use crate::core::events::*;
use solana_sdk::pubkey::Pubkey;

pub type AccountGetter<'a> = dyn Fn(usize) -> Pubkey + 'a;

#[inline]
fn fill_if_default(to: &mut Pubkey, from: Pubkey) {
    if *to == Pubkey::default() && from != Pubkey::default() {
        *to = from;
    }
}

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
pub fn fill_whirlpool_swap_accounts(e: &mut OrcaWhirlpoolSwapEvent, get: &AccountGetter<'_>) {
    /// Official Memo program — present at index 2 on `swap_v2`.
    const MEMO_PROGRAM: Pubkey = solana_sdk::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

    let is_v2 = match e.ix_name.as_str() {
        "swap" => false,
        "swap_v2" => true,
        _ => get(2) == MEMO_PROGRAM || (e.whirlpool != Pubkey::default() && get(4) == e.whirlpool),
    };
    if is_v2 {
        if e.whirlpool == Pubkey::default() {
            e.whirlpool = get(4);
        }
        fill_if_default(&mut e.token_program_a, get(0));
        fill_if_default(&mut e.token_program_b, get(1));
        fill_if_default(&mut e.token_authority, get(3));
        fill_if_default(&mut e.token_owner_account_a, get(7));
        fill_if_default(&mut e.token_owner_account_b, get(9));
        fill_if_default(&mut e.token_mint_a, get(5));
        fill_if_default(&mut e.token_mint_b, get(6));
        fill_if_default(&mut e.token_vault_a, get(8));
        fill_if_default(&mut e.token_vault_b, get(10));
        fill_if_default(&mut e.tick_array_0, get(11));
        fill_if_default(&mut e.tick_array_1, get(12));
        fill_if_default(&mut e.tick_array_2, get(13));
        fill_if_default(&mut e.oracle, get(14));
    } else {
        if e.whirlpool == Pubkey::default() {
            e.whirlpool = get(2);
        }
        // v1: single token program for both sides; mints are not in the account list.
        fill_if_default(&mut e.token_authority, get(1));
        fill_if_default(&mut e.token_owner_account_a, get(3));
        fill_if_default(&mut e.token_owner_account_b, get(5));
        let tp = get(0);
        fill_if_default(&mut e.token_program_a, tp);
        fill_if_default(&mut e.token_program_b, tp);
        fill_if_default(&mut e.token_vault_a, get(4));
        fill_if_default(&mut e.token_vault_b, get(6));
        fill_if_default(&mut e.tick_array_0, get(7));
        fill_if_default(&mut e.tick_array_1, get(8));
        fill_if_default(&mut e.tick_array_2, get(9));
        fill_if_default(&mut e.oracle, get(10));
    }
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
        let is_v2 = get(3) == solana_sdk::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
        e.position = get(if is_v2 { 5 } else { 3 });
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
        let is_v2 = get(3) == solana_sdk::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
        e.position = get(if is_v2 { 5 } else { 3 });
    }
    // tick_lower_index, tick_upper_index 等需要从链上 position 账户数据读取
    // 不能直接从指令账户填充
}
