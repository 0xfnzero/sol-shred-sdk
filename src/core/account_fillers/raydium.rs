//! Raydium 账户填充模块（包含 CLMM, CPMM, AMM V4）

use crate::core::events::*;
use crate::instr::program_ids::RAYDIUM_CLMM_PROGRAM_ID;
use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

pub type AccountGetter<'a> = dyn Fn(usize) -> Pubkey + 'a;

/// Official Memo program — present at index 10 on CLMM `swap_v2`.
const CLMM_MEMO_PROGRAM: Pubkey = pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

/// PDA seeds from Raydium CLMM IDL: `["pool_tick_array_bitmap_extension", pool_state]`.
#[inline]
pub fn tick_array_bitmap_extension_pda(pool_state: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[b"pool_tick_array_bitmap_extension", pool_state.as_ref()],
        &RAYDIUM_CLMM_PROGRAM_ID,
    )
    .0
}

/// Split remaining accounts into optional bitmap extension + tick arrays.
#[inline]
pub fn split_clmm_remaining_accounts(
    pool_state: &Pubkey,
    remaining: &[Pubkey],
) -> (Option<Pubkey>, Vec<Pubkey>) {
    if remaining.is_empty() {
        return (None, Vec::new());
    }
    let bitmap_pda = tick_array_bitmap_extension_pda(pool_state);
    let mut bitmap = None;
    let mut ticks = Vec::with_capacity(remaining.len());
    for &key in remaining {
        if key == bitmap_pda {
            bitmap = Some(key);
        } else {
            ticks.push(key);
        }
    }
    (bitmap, ticks)
}

// ============================================================================
// Raydium CLMM
// ============================================================================

/// 填充 Raydium CLMM Swap 事件账户
///
/// Swap instruction account mapping (based on IDL):
/// 0: payer
/// 1: ammConfig
/// 2: poolState
/// 3: inputTokenAccount
/// 4: outputTokenAccount
/// 5: inputVault
/// 6: outputVault
/// 7: observationState
/// Remaining: optional tickArrayBitmapExtension, then tick arrays.
/// swap_v2 additionally has memo at #10 and input/output mints at #11/#12.
/// Getter-only compatibility entry. An absent/default key terminates the tail.
/// Use the counted entry when instruction account length is available.
pub fn fill_clmm_swap_accounts(e: &mut RaydiumClmmSwapEvent, get: &AccountGetter<'_>) {
    fill_clmm_swap_accounts_impl(e, get, None);
}

/// Fill the complete instruction tail, including keys after a default address.
/// `account_count` is the number of accounts in the matched compiled instruction.
pub fn fill_clmm_swap_accounts_with_count(
    e: &mut RaydiumClmmSwapEvent,
    get: &AccountGetter<'_>,
    account_count: usize,
) {
    fill_clmm_swap_accounts_impl(e, get, Some(account_count));
}

fn fill_clmm_swap_accounts_impl(
    e: &mut RaydiumClmmSwapEvent,
    get: &AccountGetter<'_>,
    account_count: Option<usize>,
) {
    if e.pool_state == Pubkey::default() {
        e.pool_state = get(2);
    }
    if e.sender == Pubkey::default() {
        e.sender = get(0);
    }
    if e.token_account_0 == Pubkey::default() {
        e.token_account_0 = get(3);
    }
    if e.token_account_1 == Pubkey::default() {
        e.token_account_1 = get(4);
    }
    if e.amm_config == Pubkey::default() {
        e.amm_config = get(1);
    }
    if e.input_vault == Pubkey::default() {
        e.input_vault = get(5);
    }
    if e.output_vault == Pubkey::default() {
        e.output_vault = get(6);
    }
    if e.observation_state == Pubkey::default() {
        e.observation_state = get(7);
    }

    let is_swap_v2 = match e.ix_name.as_str() {
        "swap" => false,
        "swap_v2" => true,
        _ => get(10) == CLMM_MEMO_PROGRAM,
    };
    let remaining_start = if is_swap_v2 {
        if e.input_mint == Pubkey::default() {
            e.input_mint = get(11);
        }
        if e.output_mint == Pubkey::default() {
            e.output_mint = get(12);
        }
        13usize
    } else {
        9usize
    };

    if e.tick_arrays.is_empty() {
        let mut remaining = Vec::new();
        // Unknown-length getters use the u8 account-key space as a finite bound.
        // Built-in parsers supply the actual instruction account count.
        for idx in remaining_start..account_count.unwrap_or(256) {
            let key = get(idx);
            if account_count.is_none() && key == Pubkey::default() {
                break;
            }
            remaining.push(key);
        }
        if e.pool_state != Pubkey::default() && !remaining.is_empty() {
            let bitmap_pda = tick_array_bitmap_extension_pda(&e.pool_state);
            let mut bitmap = None;
            // Remove bitmap keys in place, preserving tick order without a second Vec.
            remaining.retain(|key| {
                if *key == bitmap_pda {
                    bitmap = Some(*key);
                    false
                } else {
                    true
                }
            });
            if e.tick_array_bitmap_extension.is_none() {
                e.tick_array_bitmap_extension = bitmap;
            }
        }
        e.tick_arrays = remaining;
    }
}

/// Raydium CLMM Create Pool 账户填充
///
/// createPool instruction account mapping (based on IDL):
/// 0: poolCreator
/// 1: ammConfig
/// 2: poolState
/// 3: tokenMint0
/// 4: tokenMint1
/// 5: tokenVault0
/// 6: tokenVault1
/// 7: observationState
/// 8: tokenProgram
/// 9: systemProgram
/// 10: rent
pub fn fill_clmm_create_pool_accounts(e: &mut RaydiumClmmCreatePoolEvent, get: &AccountGetter<'_>) {
    if e.creator == Pubkey::default() {
        e.creator = get(0);
    }
    if e.pool == Pubkey::default() {
        e.pool = get(2);
    }
    if e.token_0_mint == Pubkey::default() {
        e.token_0_mint = get(3);
    }
    if e.token_1_mint == Pubkey::default() {
        e.token_1_mint = get(4);
    }
    if e.token_vault_0 == Pubkey::default() {
        e.token_vault_0 = get(5);
    }
    if e.token_vault_1 == Pubkey::default() {
        e.token_vault_1 = get(6);
    }
}

/// Raydium CLMM Open Position 账户填充
///
/// openPosition instruction account mapping (based on IDL):
/// 0: payer
/// 1: positionNftOwner
/// 2: positionNftMint
/// 3: positionNftAccount
/// 4: metadataAccount
/// 5: poolState
/// 6: protocolPosition
/// 7: tickArrayLower
/// 8: tickArrayUpper
/// 9: personalPosition
/// 10: tokenAccount0
/// 11: tokenAccount1
/// 12: tokenVault0
/// 13: tokenVault1
/// ...
pub fn fill_clmm_open_position_accounts(
    e: &mut RaydiumClmmOpenPositionEvent,
    get: &AccountGetter<'_>,
) {
    fill_clmm_open_position_accounts_with_layout(e, get, false);
}

pub fn fill_clmm_open_position_accounts_with_layout(
    e: &mut RaydiumClmmOpenPositionEvent, get: &AccountGetter<'_>, token_22_nft: bool,
) {
    if e.user == Pubkey::default() {
        e.user = get(1);
    }
    if e.position_nft_mint == Pubkey::default() {
        e.position_nft_mint = get(2);
    }
    if e.pool == Pubkey::default() {
        e.pool = get(if token_22_nft { 4 } else { 5 });
    }
}

/// Raydium CLMM Close Position 账户填充
///
/// closePosition instruction account mapping (based on IDL):
/// 0: nftOwner
/// 1: positionNftMint
/// 2: positionNftAccount
/// 3: personalPosition
/// 4: systemProgram
/// 5: tokenProgram
pub fn fill_clmm_close_position_accounts(
    e: &mut RaydiumClmmClosePositionEvent,
    get: &AccountGetter<'_>,
) {
    if e.user == Pubkey::default() {
        e.user = get(0);
    }
    if e.position_nft_mint == Pubkey::default() {
        e.position_nft_mint = get(1);
    }
    // pool 已从事件数据解析
}

/// Raydium CLMM Increase Liquidity 账户填充
///
/// increaseLiquidity instruction account mapping (based on IDL):
/// 0: nftOwner
/// 1: nftAccount
/// 2: poolState
/// 3: protocolPosition
/// 4: personalPosition
/// 5: tickArrayLower
/// 6: tickArrayUpper
/// 7: tokenAccount0
/// 8: tokenAccount1
/// 9: tokenVault0
/// 10: tokenVault1
/// 11: tokenProgram
pub fn fill_clmm_increase_liquidity_accounts(
    e: &mut RaydiumClmmIncreaseLiquidityEvent,
    get: &AccountGetter<'_>,
) {
    if e.user == Pubkey::default() {
        e.user = get(0);
    }
    // NFT token-account identity does not reveal its mint; keep it unknown.
    if e.pool == Pubkey::default() {
        e.pool = get(2);
    }
}

/// Raydium CLMM Decrease Liquidity 账户填充
///
/// decreaseLiquidity instruction account mapping (based on IDL):
/// 0: nftOwner
/// 1: nftAccount
/// 2: personalPosition
/// 3: poolState
/// 4: protocolPosition
/// 5: tokenVault0
/// 6: tokenVault1
/// 7: tickArrayLower
/// 8: tickArrayUpper
/// 9: recipientTokenAccount0
/// 10: recipientTokenAccount1
/// 11: tokenProgram
pub fn fill_clmm_decrease_liquidity_accounts(
    e: &mut RaydiumClmmDecreaseLiquidityEvent,
    get: &AccountGetter<'_>,
) {
    if e.user == Pubkey::default() {
        e.user = get(0);
    }
    // NFT token-account identity does not reveal its mint; keep it unknown.
    if e.pool == Pubkey::default() {
        e.pool = get(3);
    }
}

// ============================================================================
// Raydium CPMM
// ============================================================================

/// Raydium CPMM Swap 账户填充
///
/// swapBaseInput/swapBaseOutput instruction account mapping:
/// 0: payer
/// 1: authority
/// 2: ammConfig
/// 3: poolState
/// 4: inputTokenAccount
/// 5: outputTokenAccount
/// 6: inputVault
/// 7: outputVault
/// 8: inputTokenProgram
/// 9: outputTokenProgram
/// 10: inputTokenMint
/// 11: outputTokenMint
/// 12: observationState
pub fn fill_cpmm_swap_accounts(e: &mut RaydiumCpmmSwapEvent, get: &AccountGetter<'_>) {
    if e.payer == Pubkey::default() {
        e.payer = get(0);
    }
    if e.authority == Pubkey::default() {
        e.authority = get(1);
    }
    if e.input_token_account == Pubkey::default() {
        e.input_token_account = get(4);
    }
    if e.output_token_account == Pubkey::default() {
        e.output_token_account = get(5);
    }
    if e.pool_id == Pubkey::default() {
        e.pool_id = get(3);
    }
    if e.amm_config == Pubkey::default() {
        e.amm_config = get(2);
    }
    if e.input_vault == Pubkey::default() {
        e.input_vault = get(6);
    }
    if e.output_vault == Pubkey::default() {
        e.output_vault = get(7);
    }
    if e.input_token_program == Pubkey::default() {
        e.input_token_program = get(8);
    }
    if e.output_token_program == Pubkey::default() {
        e.output_token_program = get(9);
    }
    if e.input_token_mint == Pubkey::default() {
        e.input_token_mint = get(10);
    }
    if e.output_token_mint == Pubkey::default() {
        e.output_token_mint = get(11);
    }
    if e.observation_state == Pubkey::default() {
        e.observation_state = get(12);
    }
}

/// Raydium CPMM Deposit 账户填充
///
/// deposit instruction account mapping:
/// 0: owner
/// 1: authority
/// 2: poolState
/// 3: ownerLpToken
/// 4: token0Account
/// 5: token1Account
/// 6: token0Vault
/// 7: token1Vault
/// ...
pub fn fill_cpmm_deposit_accounts(e: &mut RaydiumCpmmDepositEvent, get: &AccountGetter<'_>) {
    if e.user == Pubkey::default() {
        e.user = get(0); // owner
    }
}

/// Raydium CPMM Withdraw 账户填充
///
/// withdraw instruction account mapping:
/// 0: owner
/// 1: authority
/// 2: poolState
/// 3: ownerLpToken
/// 4: token0Account
/// 5: token1Account
/// ...
pub fn fill_cpmm_withdraw_accounts(e: &mut RaydiumCpmmWithdrawEvent, get: &AccountGetter<'_>) {
    if e.user == Pubkey::default() {
        e.user = get(0); // owner
    }
}

/// Raydium CPMM Initialize 账户填充
///
/// initialize instruction account mapping:
/// 0: creator
/// 1: ammConfig
/// 2: authority
/// 3: poolState
/// ...
pub fn fill_cpmm_initialize_accounts(e: &mut RaydiumCpmmInitializeEvent, get: &AccountGetter<'_>) {
    if e.creator == Pubkey::default() {
        e.creator = get(0);
    }
    if e.pool == Pubkey::default() {
        e.pool = get(3);
    }
}

// ============================================================================
// Raydium AMM V4
// ============================================================================

/// 填充 Raydium AMM V4 Swap 事件账户
///
/// Swap instruction account mapping (based on IDL):
/// 0: tokenProgram
/// 1: amm
/// 2: ammAuthority
/// 3: ammOpenOrders
/// 4: ammTargetOrders (optional)
/// 5: poolCoinTokenAccount
/// 6: poolPcTokenAccount
/// 7: serumProgramId
/// 8: serumMarket
/// 9: serumBids
/// 10: serumAsks
/// 11: serumEventQueue
/// 12: serumCoinVaultAccount
/// 13: serumPcVaultAccount
/// 14: serumVaultSigner
/// 15: userSourceTokenAccount
/// 16: userDestTokenAccount
/// 17: userSourceOwner
pub fn fill_amm_v4_swap_accounts(e: &mut RaydiumAmmV4SwapEvent, get: &AccountGetter<'_>) {
    if e.amm == Pubkey::default() {
        e.amm = get(1);
    }
}

/// Raydium AMM V4 Deposit 账户填充
///
/// deposit instruction account mapping (based on IDL):
/// 0: tokenProgram
/// 1: amm
/// 2: ammAuthority
/// 3: ammOpenOrders
/// 4: ammTargetOrders
/// 5: lpMintAddress
/// 6: poolCoinTokenAccount
/// 7: poolPcTokenAccount
/// 8: serumMarket
/// 9: userCoinTokenAccount
/// 10: userPcTokenAccount
/// 11: userLpTokenAccount
/// 12: userOwner
/// 13: serumEventQueue
pub fn fill_amm_v4_deposit_accounts(e: &mut RaydiumAmmV4DepositEvent, get: &AccountGetter<'_>) {
    if e.token_program == Pubkey::default() {
        e.token_program = get(0);
    }
    if e.amm_authority == Pubkey::default() {
        e.amm_authority = get(2);
    }
    // amm, max_coin_amount, max_pc_amount 已从事件数据解析
}

/// Raydium AMM V4 Withdraw 账户填充
///
/// withdraw instruction account mapping (based on IDL):
/// 0: tokenProgram
/// 1: amm
/// 2: ammAuthority
/// 3: ammOpenOrders
/// 4: ammTargetOrders
/// 5: lpMintAddress
/// 6: poolCoinTokenAccount
/// 7: poolPcTokenAccount
/// 8: poolWithdrawQueue
/// 9: poolTempLpTokenAccount
/// 10: serumProgram
/// 11: serumMarket
/// 12: serumCoinVaultAccount
/// 13: serumPcVaultAccount
/// 14: serumVaultSigner
/// 15: userLpTokenAccount
/// 16: userCoinTokenAccount
/// 17: userPcTokenAccount
/// 18: userOwner
/// 19: serumEventQ
/// 20: serumBids
/// 21: serumAsks
pub fn fill_amm_v4_withdraw_accounts(e: &mut RaydiumAmmV4WithdrawEvent, get: &AccountGetter<'_>) {
    if e.token_program == Pubkey::default() {
        e.token_program = get(0);
    }
    if e.amm_authority == Pubkey::default() {
        e.amm_authority = get(2);
    }
    if e.amm_open_orders == Pubkey::default() {
        e.amm_open_orders = get(3);
    }
    // amm, amount 已从事件数据解析
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counted_clmm_tail_keeps_long_lists_default_keys_and_existing_bitmap() {
        for v2 in [false, true] {
            let start = if v2 { 13 } else { 9 };
            let mut accounts: Vec<_> = (0..start + 70).map(|_| Pubkey::new_unique()).collect();
            let bitmap = tick_array_bitmap_extension_pda(&accounts[2]);
            accounts[start + 65] = bitmap;
            accounts[start + 25] = Pubkey::default();
            let expected = accounts[start..]
                .iter()
                .copied()
                .filter(|key| *key != bitmap)
                .collect::<Vec<_>>();
            for bitmap_known in [false, true] {
                let mut event = RaydiumClmmSwapEvent {
                    ix_name: if v2 { "swap_v2" } else { "swap" }.to_string(),
                    tick_array_bitmap_extension: bitmap_known.then_some(bitmap),
                    ..Default::default()
                };
                fill_clmm_swap_accounts_with_count(
                    &mut event,
                    &|i| accounts.get(i).copied().unwrap_or_default(),
                    accounts.len(),
                );
                assert_eq!(event.tick_array_bitmap_extension, Some(bitmap));
                assert_eq!(event.tick_arrays, expected);
                assert_eq!(event.tick_arrays.len(), 69);
            }
            // Getter-only callers also retain non-default tails longer than 16.
            accounts[start + 25] = Pubkey::new_unique();
            let mut event = RaydiumClmmSwapEvent {
                ix_name: if v2 { "swap_v2" } else { "swap" }.to_string(),
                ..Default::default()
            };
            fill_clmm_swap_accounts(&mut event, &|i| {
                accounts.get(i).copied().unwrap_or_default()
            });
            assert_eq!(event.tick_arrays.len(), 69);
        }
    }

    fn pk(n: u8) -> Pubkey {
        Pubkey::new_from_array([n; 32])
    }

    #[test]
    fn fill_swap_v2_separates_bitmap_and_ticks() {
        let memo = CLMM_MEMO_PROGRAM;
        let pool = pk(2);
        let bitmap = tick_array_bitmap_extension_pda(&pool);
        let t0 = pk(14);
        let t1 = pk(15);
        let mut accounts: Vec<_> = (0..16).map(|i| pk(i + 20)).collect();
        accounts[2] = pool;
        accounts[10] = memo;
        accounts[13] = bitmap;
        accounts[14] = t0;
        accounts[15] = t1;
        let mut e = RaydiumClmmSwapEvent::default();
        fill_clmm_swap_accounts(&mut e, &|i| accounts.get(i).copied().unwrap_or_default());
        assert_eq!(e.tick_array_bitmap_extension, Some(bitmap));
        assert_eq!(e.tick_arrays, vec![t0, t1]);
        assert_eq!(e.input_mint, accounts[11]);
        assert_eq!(e.output_mint, accounts[12]);
        assert_eq!(e.amm_config, accounts[1]);
        assert_eq!(e.input_vault, accounts[5]);
    }

    #[test]
    fn fill_swap_v1_starts_remaining_at_tick_array_slot() {
        let pool = pk(2);
        let t0 = pk(9);
        let t1 = pk(10);
        let mut accounts: Vec<_> = (0..11).map(|i| pk(i + 30)).collect();
        accounts[2] = pool;
        accounts[9] = t0;
        accounts[10] = t1;
        let mut e = RaydiumClmmSwapEvent::default();
        fill_clmm_swap_accounts(&mut e, &|i| accounts.get(i).copied().unwrap_or_default());
        assert_eq!(e.tick_arrays, vec![t0, t1]);
        assert!(e.input_mint == Pubkey::default());
        assert!(e.tick_array_bitmap_extension.is_none());
    }

    #[test]
    fn fill_cpmm_swap_maps_vaults_and_mints() {
        let accounts: Vec<_> = (0..13).map(|i| pk(i + 40)).collect();
        let mut e = RaydiumCpmmSwapEvent::default();
        fill_cpmm_swap_accounts(&mut e, &|i| accounts.get(i).copied().unwrap_or_default());
        assert_eq!(e.pool_id, accounts[3]);
        assert_eq!(e.amm_config, accounts[2]);
        assert_eq!(e.input_vault, accounts[6]);
        assert_eq!(e.output_vault, accounts[7]);
        assert_eq!(e.input_token_mint, accounts[10]);
        assert_eq!(e.output_token_mint, accounts[11]);
        assert_eq!(e.observation_state, accounts[12]);
    }
}
