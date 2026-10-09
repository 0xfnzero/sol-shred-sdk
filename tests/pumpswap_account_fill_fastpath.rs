use sol_shred_sdk::{
    core::{
        account_fillers::pumpswap::{fill_buy_accounts_with_count, fill_sell_accounts_with_count},
        events::{PumpSwapBuyEvent, PumpSwapSellEvent},
    },
    Pubkey,
};
use std::cell::Cell;

#[test]
fn resolved_trade_accounts_skip_tail_reads_and_preserve_signed_values() {
    let keys: Vec<_> = (0..27).map(|_| Pubkey::new_unique()).collect();
    let get = |index: usize| keys[index];
    for adjustment in [-500, 0] {
        let mut buy = PumpSwapBuyEvent {
            virtual_quote_reserves: adjustment,
            ..Default::default()
        };
        fill_buy_accounts_with_count(&mut buy, &get, 23);
        buy.pool_v2 = keys[23];
        buy.fee_recipient = keys[24];
        buy.fee_recipient_quote_token_account = keys[25];
        let mut sell = PumpSwapSellEvent {
            virtual_quote_reserves: adjustment,
            ..Default::default()
        };
        fill_sell_accounts_with_count(&mut sell, &get, 21);
        sell.pool_v2 = keys[21];
        sell.fee_recipient = keys[22];
        sell.fee_recipient_quote_token_account = keys[23];
        let calls = Cell::new(0);
        let counted = |index: usize| {
            calls.set(calls.get() + 1);
            keys[index]
        };
        fill_buy_accounts_with_count(&mut buy, &counted, 26);
        fill_sell_accounts_with_count(&mut sell, &counted, 24);
        assert_eq!(calls.get(), 0);
        assert_eq!(buy.virtual_quote_reserves, adjustment);
        assert_eq!(sell.virtual_quote_reserves, adjustment);
        assert_eq!(buy.pool_v2, keys[23]);
        assert_eq!(sell.pool_v2, keys[21]);

        // Completing a missing fixed account must still precede the fast return.
        buy.quote_mint = Pubkey::default();
        sell.quote_mint = Pubkey::default();
        fill_buy_accounts_with_count(&mut buy, &counted, 26);
        fill_sell_accounts_with_count(&mut sell, &counted, 24);
        assert_eq!(calls.get(), 2);
        assert_eq!(buy.quote_mint, keys[4]);
        assert_eq!(sell.quote_mint, keys[4]);
    }
}
