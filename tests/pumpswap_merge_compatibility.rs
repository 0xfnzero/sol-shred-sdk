use sol_shred_sdk::{
    core::{
        events::{
            PumpSwapBuyEvent, PumpSwapCreatePoolEvent, PumpSwapLiquidityAdded,
            PumpSwapLiquidityRemoved, PumpSwapSellEvent, PumpSwapTradeEvent,
        },
        merger::{can_merge, merge_events},
    },
    signature::Signature,
    DexEvent,
};

#[test]
fn compatibility_check_allows_signed_pumpswap_trade_merges() {
    for buy in [true, false] {
        for adjustment in [i128::MIN, -500, 0, 500, i128::MAX] {
            let (mut outer, inner) = if buy {
                (
                    DexEvent::PumpSwapBuy(PumpSwapBuyEvent::default()),
                    DexEvent::PumpSwapBuy(PumpSwapBuyEvent {
                        pool_quote_token_reserves: 1_000,
                        virtual_quote_reserves: adjustment,
                        ..Default::default()
                    }),
                )
            } else {
                (
                    DexEvent::PumpSwapSell(PumpSwapSellEvent::default()),
                    DexEvent::PumpSwapSell(PumpSwapSellEvent {
                        pool_quote_token_reserves: 1_000,
                        virtual_quote_reserves: adjustment,
                        ..Default::default()
                    }),
                )
            };
            assert!(can_merge(&outer, &inner));
            merge_events(&mut outer, inner);
            match &outer {
                DexEvent::PumpSwapBuy(e) => {
                    assert_eq!(e.virtual_quote_reserves, adjustment);
                    assert_eq!(e.pool_quote_token_reserves, 1_000);
                }
                DexEvent::PumpSwapSell(e) => {
                    assert_eq!(e.virtual_quote_reserves, adjustment);
                    assert_eq!(e.pool_quote_token_reserves, 1_000);
                }
                _ => unreachable!(),
            }
            let mut other_transaction = outer.clone();
            match &mut other_transaction {
                DexEvent::PumpSwapBuy(e) => e.metadata.signature = Signature::from([1; 64]),
                DexEvent::PumpSwapSell(e) => e.metadata.signature = Signature::from([1; 64]),
                _ => unreachable!(),
            }
            assert!(!can_merge(&outer, &other_transaction));
        }
    }
    assert!(!can_merge(
        &DexEvent::PumpSwapBuy(PumpSwapBuyEvent::default()),
        &DexEvent::PumpSwapSell(PumpSwapSellEvent::default()),
    ));
}

#[test]
fn pumpswap_compatibility_rejects_cross_variant_pairs() {
    let events = [
        DexEvent::PumpSwapTrade(PumpSwapTradeEvent::default()),
        DexEvent::PumpSwapBuy(PumpSwapBuyEvent::default()),
        DexEvent::PumpSwapSell(PumpSwapSellEvent::default()),
        DexEvent::PumpSwapCreatePool(PumpSwapCreatePoolEvent::default()),
        DexEvent::PumpSwapLiquidityAdded(PumpSwapLiquidityAdded::default()),
        DexEvent::PumpSwapLiquidityRemoved(PumpSwapLiquidityRemoved::default()),
    ];
    for (left_index, left) in events.iter().enumerate() {
        for (right_index, right) in events.iter().enumerate() {
            assert_eq!(can_merge(left, right), left_index == right_index);
        }
    }
}
